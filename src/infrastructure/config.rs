//! 应用配置。
//!
//! 内嵌 YAML 是默认配置层。CLI 可从 `CONFIG_DIR` 加载可选文件，
//! 再叠加请求指定的配置，最后初始化不可变的进程级快照。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::de::Error as _;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[allow(dead_code)]
pub fn default_config_yaml() -> &'static str {
    include_str!("../../assets/default_config.yml")
}

include!(concat!(env!("OUT_DIR"), "/config_schema.rs"));

static RUNTIME_CONFIG: OnceLock<ConfigSnapshot> = OnceLock::new();

#[derive(Debug)]
struct ConfigSnapshot {
    runtime: RuntimeConfig,
    variant: Option<ConfigVariant>,
    identity: String,
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
pub(crate) fn initialize(
    cli_value: Option<&str>,
    scale_override: Option<f64>,
) -> Result<(), String> {
    let snapshot = load_layers(cli_value, true, scale_override)?;
    if let Some(current) = RUNTIME_CONFIG.get() {
        return (current.identity == snapshot.identity)
            .then_some(())
            .ok_or_else(|| {
                "configuration has already been initialized with different values".to_string()
            });
    }
    match RUNTIME_CONFIG.set(snapshot) {
        Ok(()) => Ok(()),
        Err(snapshot) => {
            let current = RUNTIME_CONFIG
                .get()
                .expect("配置竞争初始化后必须存在进程快照");
            (current.identity == snapshot.identity)
                .then_some(())
                .ok_or_else(|| {
                    "configuration has already been initialized with different values".to_string()
                })
        }
    }
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
    validate_mania_lane_darken_alpha(&merged)?;
    validate_render_scales(&merged)?;
    let mut runtime_value = merged.clone();
    apply_render_scales(&mut runtime_value, scale_override)?;
    let runtime = serde_json::from_value(runtime_value)
        .map_err(|error| format!("invalid merged configuration: {error}"))?;
    let variant = config_variant(&defaults, &merged)?;
    let identity = canonical_json(&merged)?;
    Ok(ConfigSnapshot {
        runtime,
        variant,
        identity: format!("{identity}|scale={scale_override:?}"),
    })
}

fn validate_render_scales(config: &Value) -> Result<(), String> {
    for mode in ["standard", "taiko", "catch", "mania"] {
        for format in ["png", "gif", "mp4"] {
            let path = format!("render.{mode}.{format}.SCALE");
            let pointer = format!("/render/{mode}/{format}/SCALE");
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
fn apply_render_scales(config: &mut Value, scale_override: Option<f64>) -> Result<(), String> {
    if let Some(scale) = scale_override {
        crate::domain::validate::validate_positive_finite("output scale override", scale)
            .map_err(|error| error.to_string())?;
    }
    for mode in ["standard", "taiko", "catch", "mania"] {
        for format in ["png", "gif", "mp4"] {
            let pointer = format!("/render/{mode}/{format}");
            let Some(section) = config.pointer_mut(&pointer).and_then(Value::as_object_mut) else {
                return Err(format!(
                    "configuration section 'render.{mode}.{format}' is missing"
                ));
            };
            let configured_scale =
                section
                    .get("SCALE")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| {
                        format!("configuration field 'render.{mode}.{format}.SCALE' is invalid")
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
            let sizing = section
                .get_mut("sizing")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| {
                    format!("configuration section 'render.{mode}.{format}.sizing' is missing")
                })?;
            for (name, value) in sizing.iter_mut() {
                let Some(number) = value.as_f64() else {
                    return Err(format!(
                        "configuration field 'render.{mode}.{format}.sizing.{name}' must be numeric"
                    ));
                };
                let scaled = if is_bitmap_font_size(name) {
                    // 位图字体以 8px 字形为基础。先换算旧实现实际绘制的基础高度，
                    // 再应用输出倍率，既保持 1x 外观，又允许小数倍率精确缩放。
                    let glyph_scale = (number.max(8.0) / 8.0).floor().max(1.0);
                    (glyph_scale * 8.0 * scale).max(1.0)
                } else {
                    number * scale
                };
                if value.as_i64().is_some() || value.as_u64().is_some() {
                    let rounded = crate::domain::parser::round_half_even(scaled);
                    *value = Value::Number(rounded.into());
                } else {
                    *value = Value::Number(
                        serde_json::Number::from_f64(scaled).ok_or_else(|| {
                            format!("scaled configuration field 'render.{mode}.{format}.sizing.{name}' is not finite")
                        })?,
                    );
                }
            }
        }
    }
    Ok(())
}

fn is_bitmap_font_size(name: &str) -> bool {
    matches!(
        name,
        "TIME_LABEL_FONT_SIZE"
            | "TIME_LABEL_NOTE_FONT_SIZE"
            | "LABEL_FONT_SIZE"
            | "BPM_FONT_SIZE"
            | "SV_TEXT_FONT_SIZE"
            | "EDGE_COMBO_LABEL_FONT_SIZE"
            | "BREAK_OVERLAY_COUNTER_FONT_SIZE"
            | "BREAK_OVERLAY_INFO_FONT_SIZE"
    )
}

fn validate_video_background(config: &Value) -> Result<(), String> {
    for mode in ["standard", "taiko", "catch", "mania"] {
        let path = format!("render.{mode}.mp4.style.BACKGROUND_DIM");
        let pointer = format!("/render/{mode}/mp4/style/BACKGROUND_DIM");
        let value = config.pointer(&pointer).and_then(Value::as_f64);
        if value.is_none_or(|value| !(0.0..=1.0).contains(&value)) {
            return Err(format!(
                "configuration field '{path}' must be a number from 0 to 1"
            ));
        }
    }
    Ok(())
}

fn validate_mania_lane_darken_alpha(config: &Value) -> Result<(), String> {
    let path = "render.mania.mp4.style.LANE_DARKEN_ALPHA";
    let pointer = "/render/mania/mp4/style/LANE_DARKEN_ALPHA";
    let value = config.pointer(pointer).and_then(Value::as_f64);
    if value.is_none_or(|value| !(0.0..=1.0).contains(&value)) {
        return Err(format!(
            "configuration field '{path}' must be a number from 0 to 1"
        ));
    }
    Ok(())
}

fn validate_positive_timeouts(config: &Value) -> Result<(), String> {
    for name in ["PNG_TIMEOUT", "GIF_TIMEOUT", "MP4_TIMEOUT"] {
        let pointer = format!("/timeout/{name}");
        if config
            .pointer(&pointer)
            .and_then(Value::as_u64)
            .is_none_or(|seconds| seconds == 0)
        {
            return Err(format!(
                "configuration field 'timeout.{name}' must be a positive integer number of seconds"
            ));
        }
    }
    Ok(())
}

/// 返回当前有效配置对应的输出缓存目录。
pub(crate) fn output_directory(output_dir_override: Option<&str>) -> Result<PathBuf, String> {
    let snapshot = RUNTIME_CONFIG.get_or_init(|| {
        load_embedded_snapshot()
            .unwrap_or_else(|error| panic!("invalid embedded configuration: {error}"))
    });
    output_directory_for_snapshot(snapshot, output_dir_override)
}

fn output_directory_for_snapshot(
    snapshot: &ConfigSnapshot,
    output_dir_override: Option<&str>,
) -> Result<PathBuf, String> {
    let root =
        resolve_path(output_dir_override.unwrap_or(snapshot.runtime.paths.OUTPUT_DIR.as_str()));
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
        config_variant, merge_values, output_directory_for_snapshot, parse_argument,
        parse_document, resolve_path, write_variant_file,
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
    fn output_directory_override_replaces_only_the_root() {
        let default_snapshot = super::load_layers(None, false, None).unwrap();
        let default_root = std::env::temp_dir().join(format!(
            "osu-preview-output-override-default-test-{}",
            std::process::id()
        ));
        assert_eq!(
            output_directory_for_snapshot(&default_snapshot, Some(default_root.to_str().unwrap()))
                .unwrap(),
            default_root
        );

        let configured_snapshot = super::load_layers(
            Some(r#"{"render":{"standard":{"gif":{"structure":{"ROW_COUNT":1}}}}}"#),
            false,
            None,
        )
        .unwrap();
        let hash = configured_snapshot.variant.as_ref().unwrap().hash.clone();
        let first_root = std::env::temp_dir().join(format!(
            "osu-preview-output-override-first-test-{}",
            std::process::id()
        ));
        let second_root = std::env::temp_dir().join(format!(
            "osu-preview-output-override-second-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&first_root);
        let _ = std::fs::remove_dir_all(&second_root);

        let first =
            output_directory_for_snapshot(&configured_snapshot, Some(first_root.to_str().unwrap()))
                .unwrap();
        let second = output_directory_for_snapshot(
            &configured_snapshot,
            Some(second_root.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(first, first_root.join(&hash));
        assert_eq!(second, second_root.join(&hash));
        assert!(first.join("config.yml").is_file());
        assert!(second.join("config.yml").is_file());

        let _ = std::fs::remove_dir_all(first_root);
        let _ = std::fs::remove_dir_all(second_root);
    }

    #[test]
    fn json_overlay_preserves_unset_defaults() {
        let config = load_snapshot(Some(
            r#"{"render":{"standard":{"gif":{"structure":{"ROW_COUNT":1}}}}}"#,
        ))
        .expect("valid overlay");
        assert_eq!(config.render.standard.gif.structure.ROW_COUNT, 1);
        assert_eq!(config.render.standard.gif.structure.IMAGES_PER_ROW, 2);
    }

    #[test]
    fn render_timeouts_default_to_five_minutes() {
        let config = load_snapshot(None).expect("valid defaults");
        assert_eq!(config.timeout.PNG_TIMEOUT.as_secs(), 300);
        assert_eq!(config.timeout.GIF_TIMEOUT.as_secs(), 300);
        assert_eq!(config.timeout.MP4_TIMEOUT.as_secs(), 300);
    }

    #[test]
    fn taiko_animation_outputs_have_independent_measure_line_switches() {
        let defaults = load_snapshot(None).unwrap();
        assert!(defaults.render.taiko.gif.style.SHOW_MEASURE_LINES);
        assert!(defaults.render.taiko.mp4.style.SHOW_MEASURE_LINES);

        let config = load_snapshot(Some(
            r#"{"render":{"taiko":{"gif":{"style":{"SHOW_MEASURE_LINES":false}}}}}"#,
        ))
        .unwrap();
        assert!(!config.render.taiko.gif.style.SHOW_MEASURE_LINES);
        assert!(config.render.taiko.mp4.style.SHOW_MEASURE_LINES);
    }

    #[test]
    fn catch_png_banana_route_switch_defaults_on_and_accepts_override() {
        let defaults = load_snapshot(None).unwrap();
        assert!(defaults.render.catch.png.style.SHOW_BANANA_ROUTE);

        let config = load_snapshot(Some(
            r#"{"render":{"catch":{"png":{"style":{"SHOW_BANANA_ROUTE":false}}}}}"#,
        ))
        .unwrap();
        assert!(!config.render.catch.png.style.SHOW_BANANA_ROUTE);
    }

    #[test]
    fn render_timeout_overlays_accept_positive_integer_seconds() {
        let config = load_snapshot(Some(
            r#"{"timeout":{"PNG_TIMEOUT":"10","GIF_TIMEOUT":20,"MP4_TIMEOUT":30}}"#,
        ))
        .expect("valid timeout overlay");
        assert_eq!(config.timeout.PNG_TIMEOUT.as_secs(), 10);
        assert_eq!(config.timeout.GIF_TIMEOUT.as_secs(), 20);
        assert_eq!(config.timeout.MP4_TIMEOUT.as_secs(), 30);
    }

    #[test]
    fn render_timeouts_reject_non_positive_or_fractional_values_with_field_path() {
        for (name, value) in [
            ("PNG_TIMEOUT", "0"),
            ("GIF_TIMEOUT", "-1"),
            ("MP4_TIMEOUT", "1.5"),
        ] {
            let source = format!("{{\"timeout\":{{\"{name}\":{value}}}}}");
            let error = load_snapshot(Some(&source)).expect_err("must reject invalid timeout");
            assert!(error.contains(&format!("timeout.{name}")), "{error}");
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
            r#"{"render":{"standard":{"gif":{"sizing":{"IMAGE_WIDTH":640}}}}}"#,
            r#"{"render":{"taiko":{"mp4":{"sizing":{"ROW_HEIGHT":80}}}}}"#,
            r#"{"render":{"catch":{"gif":{"sizing":{"IMAGE_HEIGHT":384}}}}}"#,
            r#"{"render":{"catch":{"gif":{"sizing":{"PAGE_MARGIN_X":15}}}}}"#,
            r#"{"render":{"mania":{"gif":{"sizing":{"FRAME_HEIGHT":768}}}}}"#,
            r#"{"render":{"standard":{"png":{"style":{"SNAKING_IN_SLIDERS":false}}}}}"#,
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
render:
  standard:
    gif:
      structure:
        ROW_COUNT: 2
        IMAGES_PER_ROW: 2
      style:
        SHOW_TIME_LABEL: true
        DURATION_MS: 5000
"#;
        let partial_defaults = r#"
render:
  standard:
    gif:
      structure:
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
render:
  standard:
    gif:
      structure:
        ROW_COUNT: 1
        IMAGES_PER_ROW: 2
      style:
        SHOW_TIME_LABEL: false
        DURATION_MS: 10000
"#;
        let json = r#"{"render":{"standard":{"gif":{"structure":{"ROW_COUNT":1},"style":{"SHOW_TIME_LABEL":false,"DURATION_MS":10000}}}}}"#;
        let yaml_variant = variant(yaml).unwrap();
        let json_variant = variant(json).unwrap();
        assert_eq!(yaml_variant, json_variant);
        assert_eq!(yaml_variant.hash.len(), 6);
        assert_eq!(
            yaml_variant.difference,
            serde_json::json!({
                "render": {
                    "standard": {
                        "gif": {
                            "structure": {"ROW_COUNT": 1},
                            "style": {
                                "SHOW_TIME_LABEL": false,
                                "DURATION_MS": 10000
                            }
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
        let source = "render:\n  standard:\n    gif:\n      structure:\n        ROW_COUNT: 1\n";
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
        let variant = variant(
            r#"{"render":{"standard":{"gif":{"structure":{"ROW_COUNT":1,"IMAGES_PER_ROW":2}}}}}"#,
        )
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
            serde_json::json!({"render": {"standard": {"gif": {"structure": {"ROW_COUNT": 1}}}}})
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn video_background_defaults_and_overlays_are_typed() {
        let defaults = load_snapshot(None).unwrap();
        assert!(defaults.render.standard.mp4.style.ENABLE_BACKGROUND_IMAGE);
        assert_eq!(defaults.render.standard.mp4.style.BACKGROUND_DIM, 0.7);

        let configured = load_snapshot(Some(
            r#"{"render":{"standard":{"mp4":{"style":{"ENABLE_BACKGROUND_IMAGE":false,"BACKGROUND_DIM":0.25}}}}}"#,
        ))
        .unwrap();
        assert!(!configured.render.standard.mp4.style.ENABLE_BACKGROUND_IMAGE);
        assert_eq!(configured.render.standard.mp4.style.BACKGROUND_DIM, 0.25);
    }

    #[test]
    fn video_background_dim_rejects_values_outside_unit_interval() {
        for value in [-0.1, 1.1] {
            let source = serde_json::json!({
                "render": {"standard": {"mp4": {"style": {"BACKGROUND_DIM": value}}}}
            })
            .to_string();
            let error = load_snapshot(Some(&source)).expect_err("暗化程度超出范围时必须报错");
            assert!(
                error.contains("render.standard.mp4.style.BACKGROUND_DIM"),
                "{error}"
            );
        }
    }

    #[test]
    fn mania_mp4_lane_darken_alpha_defaults_and_overlays_are_typed() {
        let defaults = load_snapshot(None).unwrap();
        assert_eq!(defaults.render.mania.mp4.style.LANE_DARKEN_ALPHA, 1.0);

        let configured = load_snapshot(Some(
            r#"{"render":{"mania":{"mp4":{"style":{"LANE_DARKEN_ALPHA":0.35}}}}}"#,
        ))
        .unwrap();
        assert_eq!(configured.render.mania.mp4.style.LANE_DARKEN_ALPHA, 0.35);
    }

    #[test]
    fn mania_mp4_lane_darken_alpha_rejects_values_outside_unit_interval() {
        for value in [-0.1, 1.1] {
            let source = serde_json::json!({
                "render": {"mania": {"mp4": {"style": {"LANE_DARKEN_ALPHA": value}}}}
            })
            .to_string();
            let error = load_snapshot(Some(&source)).expect_err("轨道暗化透明度越界时必须报错");
            assert!(
                error.contains("render.mania.mp4.style.LANE_DARKEN_ALPHA"),
                "{error}"
            );
        }
    }

    #[test]
    fn every_mode_format_has_scale_and_pixel_scaling_is_precomputed() {
        let configured = load_snapshot(Some(
            r#"{
                "render": {
                    "standard": {"png": {"SCALE": 2.0}},
                    "taiko": {"gif": {"SCALE": 2.0}},
                    "catch": {"mp4": {"SCALE": 2.0}},
                    "mania": {"png": {"SCALE": 2.0}}
                }
            }"#,
        ))
        .unwrap();
        assert_eq!(configured.render.standard.png.sizing.PAGE_MARGIN_TOP, 40);
        assert_eq!(configured.render.taiko.gif.sizing.ROW_GAP, 14);
        assert_eq!(configured.render.catch.mp4.sizing.LABEL_PAD, 24);
        assert_eq!(configured.render.mania.png.sizing.PIXELS_PER_MS, 0.8);
        assert_eq!(configured.render.mania.png.sizing.SV_TEXT_FONT_SIZE, 16);
        assert_eq!(
            configured
                .render
                .mania
                .png
                .sizing
                .MAX_AREA_HEIGHT_0_TO_1_MINUTES,
            6000
        );
        // 时长、帧率和数量不是像素量，不随 SCALE 改变。
        assert_eq!(configured.render.standard.png.style.MS_PER_IMAGE, 400);
        assert_eq!(configured.render.taiko.gif.style.FPS, 15.0);
        assert_eq!(
            configured.render.catch.png.sizing.MAX_TOTAL_CHART_HEIGHT,
            180000
        );
    }

    #[test]
    fn mp4_visual_options_are_independent_per_mode() {
        let configured = load_snapshot(Some(
            r#"{
                "render": {
                    "standard": {"mp4": {"style": {"ENABLE_BACKGROUND_IMAGE": false}}},
                    "taiko": {"mp4": {"style": {"BACKGROUND_DIM": 0.2}}},
                    "catch": {"mp4": {"sizing": {"LABEL_PAD": 3}}},
                    "mania": {"mp4": {"sizing": {"LABEL_FONT_SIZE": 18}}}
                }
            }"#,
        ))
        .unwrap();
        assert!(!configured.render.standard.mp4.style.ENABLE_BACKGROUND_IMAGE);
        assert_eq!(configured.render.taiko.mp4.style.BACKGROUND_DIM, 0.2);
        assert_eq!(configured.render.catch.mp4.sizing.LABEL_PAD, 3);
        // 18px 配置在旧 8x8 位图字体中实际为 16px；运行时保存实际绘制高度。
        assert_eq!(configured.render.mania.mp4.sizing.LABEL_FONT_SIZE, 16);
        assert_eq!(configured.render.standard.mp4.style.BACKGROUND_DIM, 0.7);
        assert_eq!(configured.render.catch.mp4.style.BACKGROUND_DIM, 0.7);
    }

    #[test]
    fn mania_sv_label_switches_are_typed_and_independent() {
        let configured = load_snapshot(Some(
            r#"{
                "render": {
                    "mania": {
                        "png": {"style": {"SHOW_SV_LABEL": false}},
                        "gif": {"style": {"SHOW_SV_LABEL": false}},
                        "mp4": {"style": {"SHOW_SV_LABEL": false}}
                    }
                }
            }"#,
        ))
        .unwrap();
        assert!(!configured.render.mania.png.style.SHOW_SV_LABEL);
        assert!(!configured.render.mania.gif.style.SHOW_SV_LABEL);
        assert!(!configured.render.mania.mp4.style.SHOW_SV_LABEL);
    }

    #[test]
    fn bitmap_font_sizes_follow_fractional_output_scales() {
        let half = load_snapshot(Some(
            r#"{
                "render": {
                    "standard": {"png": {"SCALE": 0.5}},
                    "mania": {"gif": {"SCALE": 0.5}, "mp4": {"SCALE": 0.5}}
                }
            }"#,
        ))
        .unwrap();
        assert_eq!(half.render.standard.png.sizing.TIME_LABEL_FONT_SIZE, 12);
        assert_eq!(half.render.mania.gif.sizing.SV_TEXT_FONT_SIZE, 4);
        assert_eq!(half.render.mania.mp4.sizing.SV_TEXT_FONT_SIZE, 4);

        let one_and_half =
            load_snapshot(Some(r#"{"render":{"mania":{"gif":{"SCALE":1.5}}}}"#)).unwrap();
        assert_eq!(one_and_half.render.mania.gif.sizing.SV_TEXT_FONT_SIZE, 12);
    }

    #[test]
    fn command_line_scale_replaces_config_scale_for_all_pixel_fields() {
        let configured = load_snapshot_with_scale(
            Some(
                r#"{
                    "render": {
                        "standard": {"png": {"SCALE": 2.0}}
                    }
                }"#,
            ),
            Some(0.5),
        )
        .unwrap();
        assert_eq!(configured.render.standard.png.SCALE, 0.5);
        assert_eq!(configured.render.standard.png.sizing.PAGE_MARGIN_TOP, 10);
        assert_eq!(
            configured.render.standard.png.sizing.TIME_LABEL_FONT_SIZE,
            12
        );
        assert_eq!(configured.render.standard.png.style.MS_PER_IMAGE, 400);
    }

    #[test]
    fn png_layout_limits_and_taiko_spacing_have_explicit_scale_semantics() {
        let configured = load_snapshot(Some(
            r#"{
                "render": {
                    "taiko": {"png": {"SCALE": 2.0, "style": {"SPACING_PER_BPM": 180.0}}},
                    "catch": {"png": {"SCALE": 2.0}},
                    "mania": {"png": {"SCALE": 2.0}}
                }
            }"#,
        ))
        .unwrap();

        assert_eq!(
            configured
                .render
                .taiko
                .png
                .sizing
                .BASE_ROW_WIDTH_0_TO_1_MINUTES,
            5200
        );
        assert_eq!(
            configured
                .render
                .taiko
                .png
                .style
                .ROW_WIDTH_MULTIPLIER_BPM_180_TO_240,
            1.15
        );
        assert_eq!(configured.render.taiko.png.style.SPACING_PER_BPM, 180.0);
        assert_eq!(
            configured
                .render
                .catch
                .png
                .sizing
                .MAX_AREA_HEIGHT_0_TO_1_MINUTES,
            6000
        );
        assert_eq!(
            configured.render.catch.png.sizing.MAX_TOTAL_CHART_HEIGHT,
            360000
        );
        assert_eq!(
            configured
                .render
                .mania
                .png
                .sizing
                .MAX_AREA_HEIGHT_0_TO_1_MINUTES,
            6000
        );
        assert_eq!(
            configured
                .render
                .mania
                .png
                .structure
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
                r#"{{"render":{{"taiko":{{"png":{{"SCALE":{scale}}}}},"catch":{{"png":{{"SCALE":{scale}}}}},"mania":{{"png":{{"SCALE":{scale}}}}}}}}}"#
            );
            let configured = load_snapshot(Some(&source)).unwrap();
            let taiko_actual = [
                configured
                    .render
                    .taiko
                    .png
                    .sizing
                    .BASE_ROW_WIDTH_0_TO_1_MINUTES,
                configured
                    .render
                    .taiko
                    .png
                    .sizing
                    .BASE_ROW_WIDTH_1_TO_2_MINUTES,
                configured
                    .render
                    .taiko
                    .png
                    .sizing
                    .BASE_ROW_WIDTH_2_TO_3_MINUTES,
                configured
                    .render
                    .taiko
                    .png
                    .sizing
                    .BASE_ROW_WIDTH_3_TO_4_MINUTES,
                configured
                    .render
                    .taiko
                    .png
                    .sizing
                    .BASE_ROW_WIDTH_4_TO_5_MINUTES,
                configured
                    .render
                    .taiko
                    .png
                    .sizing
                    .BASE_ROW_WIDTH_5_TO_6_MINUTES,
                configured
                    .render
                    .taiko
                    .png
                    .sizing
                    .BASE_ROW_WIDTH_6_TO_10_MINUTES,
            ];
            let catch_actual = [
                configured
                    .render
                    .catch
                    .png
                    .sizing
                    .MAX_AREA_HEIGHT_0_TO_1_MINUTES,
                configured
                    .render
                    .catch
                    .png
                    .sizing
                    .MAX_AREA_HEIGHT_1_TO_2_MINUTES,
                configured
                    .render
                    .catch
                    .png
                    .sizing
                    .MAX_AREA_HEIGHT_2_TO_3_MINUTES,
                configured
                    .render
                    .catch
                    .png
                    .sizing
                    .MAX_AREA_HEIGHT_3_TO_4_MINUTES,
                configured
                    .render
                    .catch
                    .png
                    .sizing
                    .MAX_AREA_HEIGHT_4_TO_5_MINUTES,
                configured
                    .render
                    .catch
                    .png
                    .sizing
                    .MAX_AREA_HEIGHT_5_TO_6_MINUTES,
            ];
            let mania_actual = [
                configured
                    .render
                    .mania
                    .png
                    .sizing
                    .MAX_AREA_HEIGHT_0_TO_1_MINUTES,
                configured
                    .render
                    .mania
                    .png
                    .sizing
                    .MAX_AREA_HEIGHT_1_TO_2_MINUTES,
                configured
                    .render
                    .mania
                    .png
                    .sizing
                    .MAX_AREA_HEIGHT_2_TO_3_MINUTES,
                configured
                    .render
                    .mania
                    .png
                    .sizing
                    .MAX_AREA_HEIGHT_3_TO_4_MINUTES,
                configured
                    .render
                    .mania
                    .png
                    .sizing
                    .MAX_AREA_HEIGHT_4_TO_5_MINUTES,
                configured
                    .render
                    .mania
                    .png
                    .sizing
                    .MAX_AREA_HEIGHT_5_TO_6_MINUTES,
            ];
            let scaled = |value: i64| crate::domain::parser::round_half_even(value as f64 * scale);

            assert_eq!(taiko_actual, taiko_base.map(scaled));
            assert_eq!(catch_actual, catch_base.map(scaled));
            assert_eq!(mania_actual, mania_base.map(scaled));
            assert_eq!(
                configured.render.catch.png.sizing.MAX_TOTAL_CHART_HEIGHT,
                scaled(180000)
            );
            assert_eq!(
                configured
                    .render
                    .taiko
                    .png
                    .style
                    .ROW_WIDTH_MULTIPLIER_BPM_0_TO_180,
                1.0
            );
            assert_eq!(
                configured
                    .render
                    .taiko
                    .png
                    .style
                    .ROW_WIDTH_MULTIPLIER_BPM_180_TO_240,
                1.15
            );
            assert_eq!(
                configured
                    .render
                    .taiko
                    .png
                    .style
                    .ROW_WIDTH_MULTIPLIER_BPM_240_TO_300,
                1.3
            );
            assert_eq!(
                configured
                    .render
                    .taiko
                    .png
                    .style
                    .ROW_WIDTH_MULTIPLIER_BPM_300_PLUS,
                1.45
            );
            assert_eq!(configured.render.taiko.png.style.SPACING_PER_BPM, 0.0);
            assert_eq!(
                configured
                    .render
                    .mania
                    .png
                    .structure
                    .FIXED_COLUMN_COUNT_6_TO_10_MINUTES,
                30
            );
            assert_eq!(configured.render.mania.png.style.BOTTOM_PADDING_MS, 2000);
        }
    }

    #[test]
    fn every_render_format_uses_explicit_structure_sizing_and_style_sections() {
        let defaults = parse_document(super::default_config_yaml(), "embedded defaults").unwrap();
        for mode in ["standard", "taiko", "catch", "mania"] {
            for format in ["png", "gif", "mp4"] {
                let pointer = format!("/render/{mode}/{format}");
                let section = defaults.pointer(&pointer).unwrap().as_object().unwrap();
                assert!(section.get("SCALE").is_some(), "{pointer} 缺少 SCALE");
                assert!(section.get("sizing").is_some(), "{pointer} 缺少 sizing");
                assert!(section.get("style").is_some(), "{pointer} 缺少 style");
                assert!(section.keys().all(|name| matches!(
                    name.as_str(),
                    "SCALE" | "structure" | "sizing" | "style"
                )));
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

    #[test]
    fn configured_scale_and_fps_change_config_variant_hash() {
        let scale = variant(r#"{"render":{"standard":{"gif":{"SCALE":2.0}}}}"#).unwrap();
        let fps = variant(r#"{"render":{"standard":{"gif":{"style":{"FPS":30}}}}}"#).unwrap();

        assert_ne!(scale.hash, fps.hash);
        assert_eq!(
            scale.difference,
            serde_json::json!({"render": {"standard": {"gif": {"SCALE": 2.0}}}})
        );
        assert_eq!(
            fps.difference,
            serde_json::json!({"render": {"standard": {"gif": {"style": {"FPS": 30}}}}})
        );
    }
}
