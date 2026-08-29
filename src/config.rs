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
pub(crate) fn initialize_for_cli(cli_value: Option<&str>) -> Result<(), String> {
    let snapshot = load_layers(cli_value, true)?;
    RUNTIME_CONFIG
        .set(snapshot)
        .map_err(|_| "configuration has already been initialized".to_string())
}

fn load_embedded_snapshot() -> Result<ConfigSnapshot, String> {
    load_layers(None, false)
}

fn load_layers(
    cli_value: Option<&str>,
    include_config_directory: bool,
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
    let runtime = serde_json::from_value(merged.clone())
        .map_err(|error| format!("invalid merged configuration: {error}"))?;
    let variant = config_variant(&defaults, &merged)?;
    Ok(ConfigSnapshot { runtime, variant })
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
        super::load_layers(cli_value, true).map(|snapshot| snapshot.runtime)
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
}
