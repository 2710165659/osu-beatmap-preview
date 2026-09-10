//! 应用配置。
//!
//! 内嵌 YAML 由共享配置与 CLI 专用配置合并生成。CLI 可从 `CONFIG_DIR` 加载可选文件，
//! 再叠加请求指定的配置，最后初始化不可变的进程级快照。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;

use serde::de::Error as _;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[allow(dead_code)]
pub fn default_config_yaml() -> &'static str {
    include_str!(concat!(env!("OUT_DIR"), "/default_config.yml"))
}

include!(concat!(env!("OUT_DIR"), "/config_schema.rs"));

static RUNTIME_CONFIG: OnceLock<ConfigSnapshot> = OnceLock::new();

#[derive(Clone, Debug)]
pub(crate) struct ConfigSnapshot {
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

/// 将 CLI 配置中的绘制部分转换为 core 可消费的类型化配置。
pub(crate) fn core_config() -> Arc<osu_beatmap_preview_core::config::CoreConfig> {
    Arc::new(osu_beatmap_preview_core::config::CoreConfig {
        render: current().render.clone(),
        skin: current().skin.clone(),
    })
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
        osu_beatmap_preview_core::processing::validation::validate_positive_finite(
            "output scale override",
            scale,
        )
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
                    let rounded =
                        osu_beatmap_preview_core::processing::parse::round_half_even(scaled);
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
mod tests;
