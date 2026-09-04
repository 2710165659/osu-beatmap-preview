//! 应用配置。
//!
//! 内嵌 YAML 是默认配置层。CLI 可从 `CONFIG_DIR` 加载可选文件，
//! 再叠加命令行配置，最后初始化不可变的进程级快照。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::de::Error as _;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[allow(dead_code)]
pub fn default_config_yaml() -> &'static str {
    include_str!("../assets/default_config.yml")
}

include!(concat!(env!("OUT_DIR"), "/config_constants.rs"));

static RUNTIME_CONFIG: OnceLock<ConfigSnapshot> = OnceLock::new();

#[derive(Debug)]
struct ConfigSnapshot {
    runtime: RuntimeConfig,
    variant: Option<ConfigVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigVariant {
    hash: String,
    difference: Value,
}

/// 返回当前生效的配置。未初始化 CLI 配置层的库调用方会得到内嵌默认值。
pub(crate) fn current() -> &'static RuntimeConfig {
    &RUNTIME_CONFIG
        .get_or_init(|| {
            load_embedded_snapshot()
                .unwrap_or_else(|error| panic!("invalid embedded configuration: {error}"))
        })
        .runtime
}

/// 为命令行程序初始化进程级配置。
#[allow(dead_code)]
pub(crate) fn initialize_for_cli(
    cli_value: Option<&str>,
    scale_override: Option<f64>,
) -> Result<(), String> {
    let snapshot = load_layers(cli_value, true, scale_override)?;
    RUNTIME_CONFIG
        .set(snapshot)
        .map_err(|_| "configuration has already been initialized".to_string())
}

fn load_embedded_snapshot() -> Result<ConfigSnapshot, String> {
    load_layers(None, false, None)
}

fn load_layers(
    cli_value: Option<&str>,
    include_config_directory: bool,
    scale_override: Option<f64>,
) -> Result<ConfigSnapshot, String> {
    let defaults = parse_document(default_config_yaml(), "embedded defaults")?;
    let mut merged = defaults.clone();
    let default_config_dir = merged
        .get("paths")
        .and_then(Value::as_object)
        .and_then(|paths| paths.get("CONFIG_DIR"))
        .and_then(Value::as_str)
        .map(resolve_config_dir)
        .ok_or_else(|| "embedded configuration is missing paths.CONFIG_DIR".to_string())?;
    let config_file = default_config_dir.join("config.yml");
    if include_config_directory && config_file.exists() {
        let overlay = read_document_file(&config_file)?;
        merge_values(&mut merged, &overlay, "")?;
    }
    if let Some(value) = cli_value {
        let overlay = parse_argument(value)?;
        merge_values(&mut merged, &overlay, "")?;
    }
    validate_positive_timeouts(&merged)?;
    validate_video_background(&merged)?;
    validate_layout_scales(&merged)?;
    let mut runtime_value = merged.clone();
    apply_layout_scales(&mut runtime_value, scale_override)?;
    let runtime = serde_json::from_value(runtime_value)
        .map_err(|error| format!("invalid merged configuration: {error}"))?;
    let variant = config_variant(&defaults, &merged)?;
    Ok(ConfigSnapshot { runtime, variant })
}

fn validate_layout_scales(config: &Value) -> Result<(), String> {
    for mode in ["standard", "taiko", "catch", "mania"] {
        for format in ["png", "gif", "mp4"] {
            let path = format!("layout.{mode}.{format}.SCALE");
            let pointer = format!("/layout/{mode}/{format}/SCALE");
            let scale = config.pointer(&pointer).and_then(Value::as_f64);
            if scale.is_none_or(|scale| !scale.is_finite() || scale <= 0.0) {
                return Err(format!(
                    "configuration field '{path}' must be a positive finite number"
                ));
            }
        }
    }
    Ok(())
}

/// 将配置中的像素量预先换算成目标绘制尺寸，渲染器不会再对最终图像做整体缩放。
fn apply_layout_scales(config: &mut Value, scale_override: Option<f64>) -> Result<(), String> {
    if let Some(scale) = scale_override {
        crate::core::validate::validate_positive_finite("output scale override", scale)
            .map_err(|error| error.to_string())?;
    }
    for mode in ["standard", "taiko", "catch", "mania"] {
        for format in ["png", "gif", "mp4"] {
            let pointer = format!("/layout/{mode}/{format}");
            let Some(section) = config.pointer_mut(&pointer).and_then(Value::as_object_mut) else {
                return Err(format!(
                    "configuration section 'layout.{mode}.{format}' is missing"
                ));
            };
            let configured_scale =
                section
                    .get("SCALE")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| {
                        format!("configuration field 'layout.{mode}.{format}.SCALE' is invalid")
                    })?;
            let scale = scale_override.unwrap_or(configured_scale);
            if let Some(scale_override) = scale_override {
                // 命令行倍率是本次请求的临时覆盖，不参与配置差异哈希，
                // 但必须写入运行时快照，使所有渲染器读取到同一个倍率。
                let number = serde_json::Number::from_f64(scale_override).ok_or_else(|| {
                    "output scale override must be a positive finite number".to_string()
                })?;
                section.insert("SCALE".to_string(), Value::Number(number));
            }
            for (name, value) in section.iter_mut() {
                let Some(number) = value.as_f64() else {
                    continue;
                };
                let Some(kind) = layout_number_kind(name) else {
                    return Err(format!(
                        "configuration field 'layout.{mode}.{format}.{name}' has no scaling classification"
                    ));
                };
                let scaled = match kind {
                    LayoutNumberKind::Invariant => continue,
                    LayoutNumberKind::Pixel => number * scale,
                    LayoutNumberKind::BitmapFont => {
                        // 位图字体以 8px 字形为基础。先换算旧实现实际绘制的基础高度，
                        // 再应用输出倍率，既保持 1x 外观，又允许小数倍率精确缩放。
                        let glyph_scale = (number.max(8.0) / 8.0).floor().max(1.0);
                        (glyph_scale * 8.0 * scale).max(1.0)
                    }
                };
                if value.as_i64().is_some() || value.as_u64().is_some() {
                    let rounded = crate::parser::round_half_even(scaled);
                    *value = Value::Number(rounded.into());
                } else {
                    *value = Value::Number(
                        serde_json::Number::from_f64(scaled).ok_or_else(|| {
                            format!("scaled configuration field 'layout.{mode}.{format}.{name}' is not finite")
                        })?,
                    );
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayoutNumberKind {
    Invariant,
    Pixel,
    BitmapFont,
}

/// 所有布局数字都必须显式归类，避免字段名同时表达像素上限和资源上限时误判。
fn layout_number_kind(name: &str) -> Option<LayoutNumberKind> {
    use LayoutNumberKind::{BitmapFont, Invariant, Pixel};

    let kind = match name {
        "TIME_LABEL_FONT_SIZE"
        | "TIME_LABEL_NOTE_FONT_SIZE"
        | "LABEL_FONT_SIZE"
        | "BPM_FONT_SIZE"
        | "SV_TEXT_FONT_SIZE"
        | "EDGE_COMBO_LABEL_FONT_SIZE" => BitmapFont,

        "PAGE_MARGIN_TOP"
        | "PAGE_MARGIN_RIGHT"
        | "PAGE_MARGIN_BOTTOM"
        | "PAGE_MARGIN_LEFT"
        | "INFO_MARGIN_TOP"
        | "INFO_MARGIN_RIGHT"
        | "INFO_MARGIN_BOTTOM"
        | "INFO_MARGIN_LEFT"
        | "COLUMN_GAP"
        | "ROW_GAP"
        | "GRID_GAP"
        | "TIME_LABEL_TOP_GAP"
        | "TIME_LABEL_NOTE_TOP_GAP"
        | "LABEL_PAD"
        | "BASE_ROW_WIDTH_0_TO_1_MINUTES"
        | "BASE_ROW_WIDTH_1_TO_2_MINUTES"
        | "BASE_ROW_WIDTH_2_TO_3_MINUTES"
        | "BASE_ROW_WIDTH_3_TO_4_MINUTES"
        | "BASE_ROW_WIDTH_4_TO_5_MINUTES"
        | "BASE_ROW_WIDTH_5_TO_6_MINUTES"
        | "BASE_ROW_WIDTH_6_TO_10_MINUTES"
        | "ROW_HEIGHT"
        | "ROW_INNER_PADDING_X"
        | "LABEL_RIGHT_PADDING"
        | "BPM_TOP_GAP"
        | "SV_TOP_GAP"
        | "MAX_AREA_HEIGHT_0_TO_1_MINUTES"
        | "MAX_AREA_HEIGHT_1_TO_2_MINUTES"
        | "MAX_AREA_HEIGHT_2_TO_3_MINUTES"
        | "MAX_AREA_HEIGHT_3_TO_4_MINUTES"
        | "MAX_AREA_HEIGHT_4_TO_5_MINUTES"
        | "MAX_AREA_HEIGHT_5_TO_6_MINUTES"
        | "MAX_TOTAL_CHART_HEIGHT"
        | "LEFT_PANEL_WIDTH"
        | "COLUMN_WIDTH"
        | "BPM_LABEL_GAP"
        | "EDGE_GUIDE_WIDTH"
        | "EDGE_COMBO_LABEL_GAP"
        | "EDGE_COMBO_LABEL_PADDING"
        | "EDGE_COMBO_LABEL_SHADOW_GAP"
        | "PIXELS_PER_MS"
        | "LANE_WIDTH"
        | "NOTE_HEAD_HEIGHT"
        | "HIT_TARGET_FROM_BOTTOM"
        | "NOTE_SIDE_PADDING"
        | "LANE_GAP"
        | "SEPARATOR_WIDTH" => Pixel,

        "SCALE"
        | "MS_PER_IMAGE"
        | "ROW_COUNT"
        | "IMAGES_PER_ROW"
        | "SLIDER_BODY_SUPERSAMPLE"
        | "DURATION_MS"
        | "FPS"
        | "BACKGROUND_DIM"
        | "MAX_SUPPORTED_DURATION_MS"
        | "ROW_WIDTH_MULTIPLIER_BPM_0_TO_180"
        | "ROW_WIDTH_MULTIPLIER_BPM_180_TO_240"
        | "ROW_WIDTH_MULTIPLIER_BPM_240_TO_300"
        | "ROW_WIDTH_MULTIPLIER_BPM_300_PLUS"
        | "SPACING_PER_BPM"
        | "TIME_LABEL_MIN_INTERVAL_MS"
        | "RNG_SEED"
        | "FIXED_COLUMN_COUNT_6_TO_10_MINUTES"
        | "BOTTOM_PADDING_MS"
        | "SCROLL_SPEED" => Invariant,
        _ => return None,
    };
    Some(kind)
}

fn validate_video_background(config: &Value) -> Result<(), String> {
    for mode in ["standard", "taiko", "catch", "mania"] {
        let path = format!("layout.{mode}.mp4.BACKGROUND_DIM");
        let pointer = format!("/layout/{mode}/mp4/BACKGROUND_DIM");
        let value = config.pointer(&pointer).and_then(Value::as_f64);
        if value.is_none_or(|value| !(0.0..=1.0).contains(&value)) {
            return Err(format!(
                "configuration field '{path}' must be a number from 0 to 1"
            ));
        }
    }
    Ok(())
}

fn validate_positive_timeouts(config: &Value) -> Result<(), String> {
    for name in ["PNG_TIMEOUT", "GIF_TIMEOUT", "MP4_TIMEOUT"] {
        let pointer = format!("/timeouts/render/{name}");
        if config
            .pointer(&pointer)
            .and_then(Value::as_u64)
            .is_none_or(|seconds| seconds == 0)
        {
            return Err(format!(
                "configuration field 'timeouts.render.{name}' must be a positive integer number of seconds"
            ));
        }
    }
    Ok(())
}

/// 返回当前有效配置对应的输出缓存目录。
/// 与默认配置等价的配置直接使用 OUTPUT_DIR。
pub(crate) fn output_directory() -> Result<PathBuf, String> {
    let snapshot = RUNTIME_CONFIG.get_or_init(|| {
        load_embedded_snapshot()
            .unwrap_or_else(|error| panic!("invalid embedded configuration: {error}"))
    });
    let root = resolve_path(snapshot.runtime.paths.OUTPUT_DIR.as_str());
    let Some(variant) = &snapshot.variant else {
        return Ok(root);
    };

    let directory = root.join(&variant.hash);
    std::fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "failed to create configuration output directory '{}': {error}",
            directory.display()
        )
    })?;
    write_variant_file(&directory, variant)?;
    Ok(directory)
}

fn config_variant(defaults: &Value, active: &Value) -> Result<Option<ConfigVariant>, String> {
    let Some(difference) = difference(defaults, active) else {
        return Ok(None);
    };
    let canonical = canonical_json(&difference)?;
    let digest = Sha256::digest(canonical.as_bytes());
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(Some(ConfigVariant {
        hash: hex[hex.len() - 6..].to_string(),
        difference,
    }))
}

fn difference(defaults: &Value, active: &Value) -> Option<Value> {
    match (defaults, active) {
        (Value::Object(defaults), Value::Object(active)) => {
            let mut result = serde_json::Map::new();
            for (key, active_value) in active {
                match defaults.get(key) {
                    Some(default_value) => {
                        if let Some(value) = difference(default_value, active_value) {
                            result.insert(key.clone(), value);
                        }
                    }
                    None => {
                        result.insert(key.clone(), active_value.clone());
                    }
                }
            }
            (!result.is_empty()).then_some(Value::Object(result))
        }
        _ if defaults == active => None,
        _ => Some(active.clone()),
    }
}

fn canonical_json(value: &Value) -> Result<String, String> {
    fn sort(value: &Value) -> Value {
        match value {
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .map(|(key, value)| (key.clone(), sort(value)))
                    .collect(),
            ),
            Value::Array(values) => Value::Array(values.iter().map(sort).collect()),
            _ => value.clone(),
        }
    }

    serde_json::to_string(&sort(value))
        .map_err(|error| format!("failed to serialize canonical configuration: {error}"))
}

fn write_variant_file(directory: &Path, variant: &ConfigVariant) -> Result<(), String> {
    let path = directory.join("config.yml");
    if path.exists() {
        let existing = read_document_file(&path)?;
        if existing != variant.difference {
            return Err(format!(
                "configuration hash collision at '{}': existing config.yml has different values",
                directory.display()
            ));
        }
        return Ok(());
    }

    let yaml = serde_yaml::to_string(&variant.difference)
        .map_err(|error| format!("failed to serialize configuration variant: {error}"))?;
    let temp = directory.join(format!("config.yml.{}.tmp", std::process::id()));
    std::fs::write(&temp, yaml).map_err(|error| {
        format!(
            "failed to write configuration variant '{}': {error}",
            temp.display()
        )
    })?;
    match std::fs::rename(&temp, &path) {
        Ok(()) => Ok(()),
        Err(_error) if path.exists() => {
            let _ = std::fs::remove_file(&temp);
            let existing = read_document_file(&path)?;
            if existing == variant.difference {
                Ok(())
            } else {
                Err(format!(
                    "configuration hash collision at '{}': existing config.yml has different values",
                    directory.display()
                ))
            }
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            Err(format!(
                "failed to finalize configuration variant '{}': {error}",
                path.display()
            ))
        }
    }
}

fn parse_argument(value: &str) -> Result<Value, String> {
    let path = std::path::Path::new(value);
    if path.is_file() {
        return read_document_file(path);
    }
    parse_document(value, "--config value")
}

fn read_document_file(path: &std::path::Path) -> Result<Value, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read config '{}': {error}", path.display()))?;
    parse_document(&source, &format!("config file '{}'", path.display()))
}

fn parse_document(source: &str, origin: &str) -> Result<Value, String> {
    if source.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    let value = match serde_json::from_str::<Value>(source) {
        Ok(value) => value,
        Err(json_error) => {
            let yaml = serde_yaml::from_str::<serde_yaml::Value>(source).map_err(|yaml_error| {
                format!("failed to parse {origin} as JSON ({json_error}) or YAML ({yaml_error})")
            })?;
            serde_json::to_value(yaml)
                .map_err(|error| format!("failed to normalize {origin}: {error}"))?
        }
    };
    if !value.is_object() {
        return Err(format!("{origin} must contain a top-level object"));
    }
    Ok(value)
}

fn merge_values(base: &mut Value, overlay: &Value, path: &str) -> Result<(), String> {
    let Some(overlay_object) = overlay.as_object() else {
        return Err(format!(
            "configuration at '{}' must be an object",
            display_path(path)
        ));
    };
    let Some(base_object) = base.as_object_mut() else {
        return Err(format!(
            "configuration at '{}' cannot be overridden",
            display_path(path)
        ));
    };
    for (key, overlay_value) in overlay_object {
        let child_path = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        let Some(base_value) = base_object.get_mut(key) else {
            return Err(format!("unknown configuration field '{child_path}'"));
        };
        if overlay_value.is_object() {
            if !base_value.is_object() {
                return Err(format!(
                    "configuration field '{child_path}' must be a scalar or array"
                ));
            }
            merge_values(base_value, overlay_value, &child_path)?;
        } else {
            *base_value = coerce_scalar(overlay_value, base_value, &child_path)?;
        }
    }
    Ok(())
}

fn coerce_scalar(value: &Value, expected: &Value, path: &str) -> Result<Value, String> {
    if let (Some(values), Some(expected_values)) = (value.as_array(), expected.as_array()) {
        let template = expected_values.first();
        let mut converted = Vec::with_capacity(values.len());
        for (index, item) in values.iter().enumerate() {
            let expected_item = expected_values
                .get(index)
                .or(template)
                .unwrap_or(&Value::Null);
            converted.push(coerce_scalar(
                item,
                expected_item,
                &format!("{path}[{index}]"),
            )?);
        }
        return Ok(Value::Array(converted));
    }
    if let Some(text) = value.as_str() {
        if expected.is_boolean() {
            return text
                .parse::<bool>()
                .map(Value::Bool)
                .map_err(|_| format!("configuration field '{path}' must be a boolean"));
        }
        if expected.as_i64().is_some() || expected.as_u64().is_some() {
            let number = text
                .parse::<i64>()
                .map_err(|_| format!("configuration field '{path}' must be an integer"))?;
            return Ok(Value::Number(number.into()));
        }
        if expected.as_f64().is_some() {
            let number = text
                .parse::<f64>()
                .map_err(|_| format!("configuration field '{path}' must be a number"))?;
            let number = serde_json::Number::from_f64(number)
                .ok_or_else(|| format!("configuration field '{path}' must be finite"))?;
            return Ok(Value::Number(number));
        }
    }
    if expected.is_f64() {
        if let Some(number) = value.as_f64() {
            let number = serde_json::Number::from_f64(number)
                .ok_or_else(|| format!("configuration field '{path}' must be finite"))?;
            return Ok(Value::Number(number));
        }
    }
    Ok(value.clone())
}

fn display_path(path: &str) -> &str {
    if path.is_empty() {
        "<root>"
    } else {
        path
    }
}

pub(crate) fn deserialize_duration_secs<'de, D>(
    deserializer: D,
) -> Result<std::time::Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let seconds = value
        .as_u64()
        .ok_or_else(|| D::Error::custom("expected a non-negative integer number of seconds"))?;
    Ok(std::time::Duration::from_secs(seconds))
}

pub(crate) fn deserialize_positive_duration_secs<'de, D>(
    deserializer: D,
) -> Result<std::time::Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let seconds = value
        .as_u64()
        .filter(|seconds| *seconds > 0)
        .ok_or_else(|| D::Error::custom("expected a positive integer number of seconds"))?;
    Ok(std::time::Duration::from_secs(seconds))
}

/// 展开内嵌配置中的可移植目录占位符。
/// `%TEMP%` 始终使用平台临时目录；其它 `%NAME%` 占位符在环境变量存在时解析。
pub(crate) fn resolve_path(template: &str) -> PathBuf {
    let mut expanded = String::with_capacity(template.len());
    let mut remainder = template;
    while let Some(start) = remainder.find('%') {
        expanded.push_str(&remainder[..start]);
        let after_start = &remainder[start + 1..];
        let Some(end) = after_start.find('%') else {
            expanded.push_str(&remainder[start..]);
            remainder = "";
            break;
        };
        let name = &after_start[..end];
        let value = if name.eq_ignore_ascii_case("TEMP") {
            Some(std::env::temp_dir().to_string_lossy().into_owned())
        } else {
            std::env::var_os(name).map(|value| value.to_string_lossy().into_owned())
        };
        match value {
            Some(value) => expanded.push_str(&value),
            None => {
                expanded.push('%');
                expanded.push_str(name);
                expanded.push('%');
            }
        }
        remainder = &after_start[end + 1..];
    }
    expanded.push_str(remainder);
    PathBuf::from(expanded)
}

/// 解析自动配置目录。相对目录以可执行文件所在目录为基准，
/// 因此无论进程工作目录如何，`CONFIG_DIR: "./"` 都能找到旁边的 `config.yml`。
fn resolve_config_dir(template: &str) -> PathBuf {
    let path = resolve_path(template);
    if path.is_absolute() {
        return path;
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.join(&path)))
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::{
        config_variant, merge_values, parse_argument, parse_document, resolve_path,
        write_variant_file,
    };

    fn variant(source: &str) -> Option<super::ConfigVariant> {
        let defaults = parse_document(super::default_config_yaml(), "defaults").unwrap();
        let mut active = defaults.clone();
        let overlay = parse_document(source, "test config").unwrap();
        merge_values(&mut active, &overlay, "").unwrap();
        config_variant(&defaults, &active).unwrap()
    }

    fn load_snapshot(cli_value: Option<&str>) -> Result<super::RuntimeConfig, String> {
        super::load_layers(cli_value, true, None).map(|snapshot| snapshot.runtime)
    }

    fn load_snapshot_with_scale(
        cli_value: Option<&str>,
        scale_override: Option<f64>,
    ) -> Result<super::RuntimeConfig, String> {
        super::load_layers(cli_value, true, scale_override).map(|snapshot| snapshot.runtime)
    }

    #[test]
    fn expands_temp_placeholder() {
        let path = resolve_path("%TEMP%/osu-beatmap-preview");
        assert_eq!(path, std::env::temp_dir().join("osu-beatmap-preview"));
    }

    #[test]
    fn json_overlay_preserves_unset_defaults() {
        let config = load_snapshot(Some(r#"{"layout":{"standard":{"gif":{"ROW_COUNT":1}}}}"#))
            .expect("valid overlay");
        assert_eq!(config.layout.standard.gif.ROW_COUNT, 1);
        assert_eq!(config.layout.standard.gif.IMAGES_PER_ROW, 2);
    }

    #[test]
    fn render_timeouts_default_to_five_minutes() {
        let config = load_snapshot(None).expect("valid defaults");
        assert_eq!(config.timeouts.render.PNG_TIMEOUT.as_secs(), 300);
        assert_eq!(config.timeouts.render.GIF_TIMEOUT.as_secs(), 300);
        assert_eq!(config.timeouts.render.MP4_TIMEOUT.as_secs(), 300);
    }

    #[test]
    fn taiko_animation_outputs_have_independent_measure_line_switches() {
        let defaults = load_snapshot(None).unwrap();
        assert!(defaults.layout.taiko.gif.SHOW_MEASURE_LINES);
        assert!(defaults.layout.taiko.mp4.SHOW_MEASURE_LINES);

        let config = load_snapshot(Some(
            r#"{"layout":{"taiko":{"gif":{"SHOW_MEASURE_LINES":false}}}}"#,
        ))
        .unwrap();
        assert!(!config.layout.taiko.gif.SHOW_MEASURE_LINES);
        assert!(config.layout.taiko.mp4.SHOW_MEASURE_LINES);
    }

    #[test]
    fn catch_png_banana_route_switch_defaults_on_and_accepts_override() {
        let defaults = load_snapshot(None).unwrap();
        assert!(defaults.layout.catch.png.SHOW_BANANA_ROUTE);

        let config = load_snapshot(Some(
            r#"{"layout":{"catch":{"png":{"SHOW_BANANA_ROUTE":false}}}}"#,
        ))
        .unwrap();
        assert!(!config.layout.catch.png.SHOW_BANANA_ROUTE);
    }

    #[test]
    fn render_timeout_overlays_accept_positive_integer_seconds() {
        let config = load_snapshot(Some(
            r#"{"timeouts":{"render":{"PNG_TIMEOUT":"10","GIF_TIMEOUT":20,"MP4_TIMEOUT":30}}}"#,
        ))
        .expect("valid timeout overlay");
        assert_eq!(config.timeouts.render.PNG_TIMEOUT.as_secs(), 10);
        assert_eq!(config.timeouts.render.GIF_TIMEOUT.as_secs(), 20);
        assert_eq!(config.timeouts.render.MP4_TIMEOUT.as_secs(), 30);
    }

    #[test]
    fn render_timeouts_reject_non_positive_or_fractional_values_with_field_path() {
        for (name, value) in [
            ("PNG_TIMEOUT", "0"),
            ("GIF_TIMEOUT", "-1"),
            ("MP4_TIMEOUT", "1.5"),
        ] {
            let source = format!("{{\"timeouts\":{{\"render\":{{\"{name}\":{value}}}}}}}");
            let error = load_snapshot(Some(&source)).expect_err("must reject invalid timeout");
            assert!(
                error.contains(&format!("timeouts.render.{name}")),
                "{error}"
            );
        }
    }

    #[test]
    fn removed_logging_fields_are_rejected() {
        for source in [
            r#"{"logging":{"timestamp":{"LOCAL_FORMAT":"[year]"}}}"#,
            r#"{"logging":{"writer":{"MAX_LINE_BYTES":1024}}}"#,
        ] {
            let error = load_snapshot(Some(source)).expect_err("removed field must be rejected");
            assert!(error.contains("unknown configuration field"), "{error}");
        }
    }

    #[test]
    fn arrays_replace_and_convert_nested_scalars() {
        let config = load_snapshot(Some(
            r#"{"skin":{"MANIA":{"KEYS_3":{"COLUMN_WIDTHS":["70","71","72"]}}}}"#,
        ))
        .expect("valid array overlay");
        assert_eq!(config.skin.MANIA.KEYS_3.COLUMN_WIDTHS, vec![70, 71, 72]);
    }

    #[test]
    fn skin_defaults_cover_all_mania_keycounts() {
        let config = load_snapshot(None).expect("valid defaults");
        macro_rules! assert_block {
            ($block:expr, $keys:expr, $width:expr, $line_count:expr, $hit_position:expr) => {{
                assert_eq!($block.COLUMN_WIDTHS, vec![$width; $keys]);
                assert_eq!($block.COLUMN_LINE_WIDTHS.len(), $line_count);
                assert_eq!($block.COLUMN_COLORS.len(), $keys);
                assert_eq!($block.HIT_POSITION, $hit_position);
            }};
        }
        assert_block!(config.skin.MANIA.KEYS_1, 1, 68, 2, 473);
        assert_block!(config.skin.MANIA.KEYS_2, 2, 68, 3, 473);
        assert_block!(config.skin.MANIA.KEYS_3, 3, 68, 4, 473);
        assert_block!(config.skin.MANIA.KEYS_4, 4, 68, 5, 473);
        assert_block!(config.skin.MANIA.KEYS_5, 5, 60, 6, 460);
        assert_block!(config.skin.MANIA.KEYS_6, 6, 55, 7, 460);
        assert_block!(config.skin.MANIA.KEYS_7, 7, 53, 8, 460);
        assert_block!(config.skin.MANIA.KEYS_8, 8, 53, 9, 460);
        assert_block!(config.skin.MANIA.KEYS_9, 9, 53, 10, 460);
        assert_block!(config.skin.MANIA.KEYS_10, 10, 53, 11, 460);
        assert_block!(config.skin.MANIA.KEYS_11, 11, 53, 12, 460);
        assert_block!(config.skin.MANIA.KEYS_12, 12, 53, 13, 460);
        assert_block!(config.skin.MANIA.KEYS_13, 13, 53, 14, 460);
        assert_block!(config.skin.MANIA.KEYS_14, 14, 53, 15, 460);
        assert_block!(config.skin.MANIA.KEYS_15, 15, 53, 16, 460);
        assert_block!(config.skin.MANIA.KEYS_16, 16, 53, 17, 460);
        assert_block!(config.skin.MANIA.KEYS_17, 17, 48, 18, 460);
        assert_block!(config.skin.MANIA.KEYS_18, 18, 48, 19, 460);
        assert_eq!(config.skin.COMBO_COLORS.len(), 3);
        assert_eq!(config.skin.HIT_CIRCLE_OVERLAP, 10);
        assert_eq!(config.skin.HYPER_DASH, [255, 82, 139]);
    }

    #[test]
    fn skin_overlay_accepts_mania_integer_arrays() {
        let config = load_snapshot(Some(
            r#"{
                "skin": {
                    "MANIA": {
                        "KEYS_3": {
                            "COLUMN_WIDTHS": ["70", "71", "72"],
                            "HIT_POSITION": 450
                        }
                    }
                }
            }"#,
        ))
        .expect("valid skin overlay");
        assert_eq!(config.skin.MANIA.KEYS_3.COLUMN_WIDTHS, vec![70, 71, 72]);
        assert_eq!(config.skin.MANIA.KEYS_3.HIT_POSITION, 450);
    }

    #[test]
    fn removed_layout_fields_are_rejected() {
        for source in [
            r#"{"layout":{"standard":{"gif":{"IMAGE_WIDTH":640}}}}"#,
            r#"{"layout":{"taiko":{"mp4":{"ROW_HEIGHT":80}}}}"#,
            r#"{"layout":{"catch":{"gif":{"IMAGE_HEIGHT":384}}}}"#,
            r#"{"layout":{"catch":{"gif":{"PAGE_MARGIN_X":15}}}}"#,
            r#"{"layout":{"mania":{"gif":{"FRAME_HEIGHT":768}}}}"#,
            r#"{"layout":{"standard":{"png":{"SNAKING_IN_SLIDERS":false}}}}"#,
        ] {
            let error = load_snapshot(Some(source)).expect_err("removed field must be rejected");
            assert!(error.contains("unknown configuration field"), "{error}");
        }
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let error = load_snapshot(Some(r#"{"unknown":true}"#)).expect_err("must reject unknown");
        assert!(error.contains("unknown configuration field 'unknown'"));
    }

    #[test]
    fn removed_timestamp_format_is_rejected() {
        let error = load_snapshot(Some(
            r#"{"logging":{"timestamp":{"LOCAL_FORMAT":"[not-a-format]"}}}"#,
        ))
        .expect_err("removed field must be rejected");
        assert!(error.contains("unknown configuration field"), "{error}");
    }

    #[test]
    fn default_equivalent_inputs_have_no_variant() {
        let explicit_defaults = r#"
layout:
  standard:
    gif:
      ROW_COUNT: 2
      IMAGES_PER_ROW: 2
      SHOW_TIME_LABEL: true
      DURATION_MS: 5000
"#;
        let partial_defaults = r#"
layout:
  standard:
    gif:
      ROW_COUNT: 2
      IMAGES_PER_ROW: 2
"#;
        assert!(variant(explicit_defaults).is_none());
        assert!(variant("").is_none());
        assert!(variant(partial_defaults).is_none());
    }

    #[test]
    fn equivalent_json_and_yaml_have_same_hash_and_minimal_difference() {
        let yaml = r#"
layout:
  standard:
    gif:
      ROW_COUNT: 1
      IMAGES_PER_ROW: 2
      SHOW_TIME_LABEL: false
      DURATION_MS: 10000
"#;
        let json = r#"{"layout":{"standard":{"gif":{"ROW_COUNT":1,"SHOW_TIME_LABEL":false,"DURATION_MS":10000}}}}"#;
        let yaml_variant = variant(yaml).unwrap();
        let json_variant = variant(json).unwrap();
        assert_eq!(yaml_variant, json_variant);
        assert_eq!(yaml_variant.hash, "6bde85");
        assert_eq!(
            yaml_variant.difference,
            serde_json::json!({
                "layout": {
                    "standard": {
                        "gif": {
                            "ROW_COUNT": 1,
                            "SHOW_TIME_LABEL": false,
                            "DURATION_MS": 10000
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn file_input_has_same_variant_as_inline_document() {
        let directory = std::env::temp_dir().join(format!(
            "osu-preview-config-input-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("variant.yml");
        let source = "layout:\n  standard:\n    gif:\n      ROW_COUNT: 1\n";
        std::fs::write(&path, source).unwrap();

        let defaults = parse_document(super::default_config_yaml(), "defaults").unwrap();
        let mut from_file = defaults.clone();
        merge_values(
            &mut from_file,
            &parse_argument(path.to_str().unwrap()).unwrap(),
            "",
        )
        .unwrap();
        let mut inline = defaults.clone();
        merge_values(&mut inline, &parse_argument(source).unwrap(), "").unwrap();

        assert_eq!(
            config_variant(&defaults, &from_file).unwrap(),
            config_variant(&defaults, &inline).unwrap()
        );
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(directory);
    }

    #[test]
    fn variant_file_contains_only_normalized_difference() {
        let variant =
            variant(r#"{"layout":{"standard":{"gif":{"ROW_COUNT":1,"IMAGES_PER_ROW":2}}}}"#)
                .unwrap();
        let directory = std::env::temp_dir().join(format!(
            "osu-preview-config-output-test-{}-{}",
            std::process::id(),
            variant.hash
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();

        write_variant_file(&directory, &variant).unwrap();
        let written = parse_argument(directory.join("config.yml").to_str().unwrap()).unwrap();
        assert_eq!(written, variant.difference);
        assert_eq!(
            written,
            serde_json::json!({"layout": {"standard": {"gif": {"ROW_COUNT": 1}}}})
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn video_background_defaults_and_overlays_are_typed() {
        let defaults = load_snapshot(None).unwrap();
        assert!(defaults.layout.standard.mp4.ENABLE_BACKGROUND_IMAGE);
        assert_eq!(defaults.layout.standard.mp4.BACKGROUND_DIM, 0.7);

        let configured = load_snapshot(Some(
            r#"{"layout":{"standard":{"mp4":{"ENABLE_BACKGROUND_IMAGE":false,"BACKGROUND_DIM":0.25}}}}"#,
        ))
        .unwrap();
        assert!(!configured.layout.standard.mp4.ENABLE_BACKGROUND_IMAGE);
        assert_eq!(configured.layout.standard.mp4.BACKGROUND_DIM, 0.25);
    }

    #[test]
    fn video_background_dim_rejects_values_outside_unit_interval() {
        for value in [-0.1, 1.1] {
            let source =
                format!(r#"{{"layout":{{"standard":{{"mp4":{{"BACKGROUND_DIM":{value}}}}}}}}}"#);
            let error = load_snapshot(Some(&source)).expect_err("暗化程度超出范围时必须报错");
            assert!(
                error.contains("layout.standard.mp4.BACKGROUND_DIM"),
                "{error}"
            );
        }
    }

    #[test]
    fn every_mode_format_has_scale_and_pixel_scaling_is_precomputed() {
        let configured = load_snapshot(Some(
            r#"{
                "layout": {
                    "standard": {"png": {"SCALE": 2.0}},
                    "taiko": {"gif": {"SCALE": 2.0}},
                    "catch": {"mp4": {"SCALE": 2.0}},
                    "mania": {"png": {"SCALE": 2.0}}
                }
            }"#,
        ))
        .unwrap();
        assert_eq!(configured.layout.standard.png.PAGE_MARGIN_TOP, 40);
        assert_eq!(configured.layout.taiko.gif.ROW_GAP, 14);
        assert_eq!(configured.layout.catch.mp4.LABEL_PAD, 24);
        assert_eq!(configured.layout.mania.png.PIXELS_PER_MS, 0.8);
        assert_eq!(configured.layout.mania.png.SV_TEXT_FONT_SIZE, 16);
        assert_eq!(
            configured.layout.mania.png.MAX_AREA_HEIGHT_0_TO_1_MINUTES,
            6000
        );
        // 时长、帧率和数量不是像素量，不随 SCALE 改变。
        assert_eq!(configured.layout.standard.png.MS_PER_IMAGE, 400);
        assert_eq!(configured.layout.taiko.gif.FPS, 15.0);
        assert_eq!(configured.layout.catch.png.MAX_TOTAL_CHART_HEIGHT, 180000);
    }

    #[test]
    fn mp4_visual_options_are_independent_per_mode() {
        let configured = load_snapshot(Some(
            r#"{
                "layout": {
                    "standard": {"mp4": {"ENABLE_BACKGROUND_IMAGE": false}},
                    "taiko": {"mp4": {"BACKGROUND_DIM": 0.2}},
                    "catch": {"mp4": {"LABEL_PAD": 3}},
                    "mania": {"mp4": {"LABEL_FONT_SIZE": 18}}
                }
            }"#,
        ))
        .unwrap();
        assert!(!configured.layout.standard.mp4.ENABLE_BACKGROUND_IMAGE);
        assert_eq!(configured.layout.taiko.mp4.BACKGROUND_DIM, 0.2);
        assert_eq!(configured.layout.catch.mp4.LABEL_PAD, 3);
        // 18px 配置在旧 8x8 位图字体中实际为 16px；运行时保存实际绘制高度。
        assert_eq!(configured.layout.mania.mp4.LABEL_FONT_SIZE, 16);
        assert_eq!(configured.layout.standard.mp4.BACKGROUND_DIM, 0.7);
        assert_eq!(configured.layout.catch.mp4.BACKGROUND_DIM, 0.7);
    }

    #[test]
    fn mania_sv_label_switches_are_typed_and_independent() {
        let configured = load_snapshot(Some(
            r#"{
                "layout": {
                    "mania": {
                        "png": {"SHOW_SV_LABEL": false},
                        "gif": {"SHOW_SV_LABEL": false},
                        "mp4": {"SHOW_SV_LABEL": false}
                    }
                }
            }"#,
        ))
        .unwrap();
        assert!(!configured.layout.mania.png.SHOW_SV_LABEL);
        assert!(!configured.layout.mania.gif.SHOW_SV_LABEL);
        assert!(!configured.layout.mania.mp4.SHOW_SV_LABEL);
    }

    #[test]
    fn bitmap_font_sizes_follow_fractional_output_scales() {
        let half = load_snapshot(Some(
            r#"{
                "layout": {
                    "standard": {"png": {"SCALE": 0.5}},
                    "mania": {"gif": {"SCALE": 0.5}, "mp4": {"SCALE": 0.5}}
                }
            }"#,
        ))
        .unwrap();
        assert_eq!(half.layout.standard.png.TIME_LABEL_FONT_SIZE, 12);
        assert_eq!(half.layout.mania.gif.SV_TEXT_FONT_SIZE, 4);
        assert_eq!(half.layout.mania.mp4.SV_TEXT_FONT_SIZE, 4);

        let one_and_half =
            load_snapshot(Some(r#"{"layout":{"mania":{"gif":{"SCALE":1.5}}}}"#)).unwrap();
        assert_eq!(one_and_half.layout.mania.gif.SV_TEXT_FONT_SIZE, 12);
    }

    #[test]
    fn command_line_scale_replaces_config_scale_for_all_pixel_fields() {
        let configured = load_snapshot_with_scale(
            Some(
                r#"{
                    "layout": {
                        "standard": {"png": {"SCALE": 2.0}}
                    }
                }"#,
            ),
            Some(0.5),
        )
        .unwrap();
        assert_eq!(configured.layout.standard.png.SCALE, 0.5);
        assert_eq!(configured.layout.standard.png.PAGE_MARGIN_TOP, 10);
        assert_eq!(configured.layout.standard.png.TIME_LABEL_FONT_SIZE, 12);
        assert_eq!(configured.layout.standard.png.MS_PER_IMAGE, 400);
    }

    #[test]
    fn png_layout_limits_and_taiko_spacing_have_explicit_scale_semantics() {
        let configured = load_snapshot(Some(
            r#"{
                "layout": {
                    "taiko": {"png": {"SCALE": 2.0, "SPACING_PER_BPM": 180.0}},
                    "catch": {"png": {"SCALE": 2.0}},
                    "mania": {"png": {"SCALE": 2.0}}
                }
            }"#,
        ))
        .unwrap();

        assert_eq!(
            configured.layout.taiko.png.BASE_ROW_WIDTH_0_TO_1_MINUTES,
            5200
        );
        assert_eq!(
            configured
                .layout
                .taiko
                .png
                .ROW_WIDTH_MULTIPLIER_BPM_180_TO_240,
            1.15
        );
        assert_eq!(configured.layout.taiko.png.SPACING_PER_BPM, 180.0);
        assert_eq!(
            configured.layout.catch.png.MAX_AREA_HEIGHT_0_TO_1_MINUTES,
            6000
        );
        assert_eq!(configured.layout.catch.png.MAX_TOTAL_CHART_HEIGHT, 360000);
        assert_eq!(
            configured.layout.mania.png.MAX_AREA_HEIGHT_0_TO_1_MINUTES,
            6000
        );
        assert_eq!(
            configured
                .layout
                .mania
                .png
                .FIXED_COLUMN_COUNT_6_TO_10_MINUTES,
            30
        );
    }

    #[test]
    fn all_png_layout_limit_tiers_scale_at_supported_fractional_factors() {
        let taiko_base = [2600, 3200, 3800, 4400, 5000, 5600, 6400];
        let catch_base = [3000, 4125, 5250, 6375, 7500, 8625];
        let mania_base = [3000, 5000, 7000, 8500, 10000, 11500];

        for scale in [0.5, 1.0, 1.5, 2.0] {
            let source = format!(
                r#"{{"layout":{{"taiko":{{"png":{{"SCALE":{scale}}}}},"catch":{{"png":{{"SCALE":{scale}}}}},"mania":{{"png":{{"SCALE":{scale}}}}}}}}}"#
            );
            let configured = load_snapshot(Some(&source)).unwrap();
            let taiko_actual = [
                configured.layout.taiko.png.BASE_ROW_WIDTH_0_TO_1_MINUTES,
                configured.layout.taiko.png.BASE_ROW_WIDTH_1_TO_2_MINUTES,
                configured.layout.taiko.png.BASE_ROW_WIDTH_2_TO_3_MINUTES,
                configured.layout.taiko.png.BASE_ROW_WIDTH_3_TO_4_MINUTES,
                configured.layout.taiko.png.BASE_ROW_WIDTH_4_TO_5_MINUTES,
                configured.layout.taiko.png.BASE_ROW_WIDTH_5_TO_6_MINUTES,
                configured.layout.taiko.png.BASE_ROW_WIDTH_6_TO_10_MINUTES,
            ];
            let catch_actual = [
                configured.layout.catch.png.MAX_AREA_HEIGHT_0_TO_1_MINUTES,
                configured.layout.catch.png.MAX_AREA_HEIGHT_1_TO_2_MINUTES,
                configured.layout.catch.png.MAX_AREA_HEIGHT_2_TO_3_MINUTES,
                configured.layout.catch.png.MAX_AREA_HEIGHT_3_TO_4_MINUTES,
                configured.layout.catch.png.MAX_AREA_HEIGHT_4_TO_5_MINUTES,
                configured.layout.catch.png.MAX_AREA_HEIGHT_5_TO_6_MINUTES,
            ];
            let mania_actual = [
                configured.layout.mania.png.MAX_AREA_HEIGHT_0_TO_1_MINUTES,
                configured.layout.mania.png.MAX_AREA_HEIGHT_1_TO_2_MINUTES,
                configured.layout.mania.png.MAX_AREA_HEIGHT_2_TO_3_MINUTES,
                configured.layout.mania.png.MAX_AREA_HEIGHT_3_TO_4_MINUTES,
                configured.layout.mania.png.MAX_AREA_HEIGHT_4_TO_5_MINUTES,
                configured.layout.mania.png.MAX_AREA_HEIGHT_5_TO_6_MINUTES,
            ];
            let scaled = |value: i64| crate::parser::round_half_even(value as f64 * scale);

            assert_eq!(taiko_actual, taiko_base.map(scaled));
            assert_eq!(catch_actual, catch_base.map(scaled));
            assert_eq!(mania_actual, mania_base.map(scaled));
            assert_eq!(
                configured.layout.catch.png.MAX_TOTAL_CHART_HEIGHT,
                scaled(180000)
            );
            assert_eq!(
                configured
                    .layout
                    .taiko
                    .png
                    .ROW_WIDTH_MULTIPLIER_BPM_0_TO_180,
                1.0
            );
            assert_eq!(
                configured
                    .layout
                    .taiko
                    .png
                    .ROW_WIDTH_MULTIPLIER_BPM_180_TO_240,
                1.15
            );
            assert_eq!(
                configured
                    .layout
                    .taiko
                    .png
                    .ROW_WIDTH_MULTIPLIER_BPM_240_TO_300,
                1.3
            );
            assert_eq!(
                configured
                    .layout
                    .taiko
                    .png
                    .ROW_WIDTH_MULTIPLIER_BPM_300_PLUS,
                1.45
            );
            assert_eq!(configured.layout.taiko.png.SPACING_PER_BPM, 0.0);
            assert_eq!(
                configured
                    .layout
                    .mania
                    .png
                    .FIXED_COLUMN_COUNT_6_TO_10_MINUTES,
                30
            );
            assert_eq!(configured.layout.mania.png.BOTTOM_PADDING_MS, 2000);
        }
    }

    #[test]
    fn every_numeric_layout_default_has_a_scale_classification() {
        let defaults = parse_document(super::default_config_yaml(), "embedded defaults").unwrap();
        for mode in ["standard", "taiko", "catch", "mania"] {
            for format in ["png", "gif", "mp4"] {
                let pointer = format!("/layout/{mode}/{format}");
                let section = defaults.pointer(&pointer).unwrap().as_object().unwrap();
                for (name, value) in section {
                    if value.is_number() {
                        assert!(
                            super::layout_number_kind(name).is_some(),
                            "{pointer}/{name} 缺少缩放语义分类"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn command_line_scale_is_validated_before_runtime_snapshot_creation() {
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let error = load_snapshot_with_scale(None, Some(scale)).expect_err("倍率必须被拒绝");
            assert!(error.contains("positive finite"), "{error}");
        }
    }

    #[test]
    fn command_line_scale_does_not_change_config_variant_hash() {
        let without_scale = super::load_layers(None, true, None).unwrap();
        let with_scale = super::load_layers(None, true, Some(2.0)).unwrap();
        assert_eq!(
            without_scale.variant.as_ref().map(|variant| &variant.hash),
            with_scale.variant.as_ref().map(|variant| &variant.hash)
        );
    }
}
