//! Application configuration.
//!
//! The embedded YAML is the default layer. The CLI can add an optional file
//! from `CONFIG_DIR` and a final command-line overlay before the immutable
//! process-wide snapshot is initialized.

use std::path::PathBuf;
use std::sync::OnceLock;

use serde::de::Error as _;
use serde::Deserialize;
use serde_json::Value;

#[allow(dead_code)]
pub fn default_config_yaml() -> &'static str {
    include_str!("../assets/default_config.yml")
}

include!(concat!(env!("OUT_DIR"), "/config_constants.rs"));

static RUNTIME_CONFIG: OnceLock<RuntimeConfig> = OnceLock::new();

/// Return the active configuration. Library callers that do not initialize
/// the CLI layer receive the embedded defaults.
pub(crate) fn current() -> &'static RuntimeConfig {
    RUNTIME_CONFIG.get_or_init(|| {
        load_embedded_snapshot()
            .unwrap_or_else(|error| panic!("invalid embedded configuration: {error}"))
    })
}

/// Initialize the process-wide configuration for the command-line binary.
#[allow(dead_code)]
pub(crate) fn initialize_for_cli(cli_value: Option<&str>) -> Result<(), String> {
    let snapshot = load_snapshot(cli_value)?;
    RUNTIME_CONFIG
        .set(snapshot)
        .map_err(|_| "configuration has already been initialized".to_string())
}

fn load_snapshot(cli_value: Option<&str>) -> Result<RuntimeConfig, String> {
    load_layers(cli_value, true)
}

fn load_embedded_snapshot() -> Result<RuntimeConfig, String> {
    load_layers(None, false)
}

fn load_layers(
    cli_value: Option<&str>,
    include_config_directory: bool,
) -> Result<RuntimeConfig, String> {
    let mut merged = parse_document(default_config_yaml(), "embedded defaults")?;
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
    let format = merged
        .pointer("/logging/timestamp/LOCAL_FORMAT")
        .and_then(Value::as_str)
        .ok_or_else(|| "logging.timestamp.LOCAL_FORMAT must be a string".to_string())?;
    time::format_description::parse_owned::<2>(format)
        .map_err(|error| format!("invalid logging.timestamp.LOCAL_FORMAT: {error}"))?;
    serde_json::from_value(merged).map_err(|error| format!("invalid merged configuration: {error}"))
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
    Ok(value.clone())
}

fn display_path(path: &str) -> &str {
    if path.is_empty() {
        "<root>"
    } else {
        path
    }
}

pub(crate) fn deserialize_duration_ms<'de, D>(
    deserializer: D,
) -> Result<std::time::Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let millis = value.as_u64().ok_or_else(|| {
        D::Error::custom("expected a non-negative integer number of milliseconds")
    })?;
    Ok(std::time::Duration::from_millis(millis))
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

/// Expand portable directory placeholders from the embedded configuration.
/// `%TEMP%` always uses the platform's temporary directory; other `%NAME%`
/// placeholders are resolved from the process environment when available.
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

/// Resolve the automatic configuration directory. Relative directories are
/// anchored beside the executable so `CONFIG_DIR: "./"` finds `config.yml`
/// next to the binary regardless of the process working directory.
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
    use super::{load_snapshot, resolve_path};

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
    fn yaml_overlay_and_scalar_conversion_work() {
        let config = load_snapshot(Some("logging:\n  writer:\n    MAX_LINE_BYTES: \"1024\"\n"))
            .expect("valid overlay");
        assert_eq!(config.logging.writer.MAX_LINE_BYTES, 1024);
    }

    #[test]
    fn arrays_replace_and_convert_nested_scalars() {
        let config = load_snapshot(Some(
            r#"{"layout":{"catch":{"png":{"BANANA_COLORS":[["1","2","3"]]}}}}"#,
        ))
        .expect("valid array overlay");
        assert_eq!(config.layout.catch.png.BANANA_COLORS, vec![[1, 2, 3]]);
    }

    #[test]
    fn boolean_strings_are_converted() {
        let config = load_snapshot(Some(
            r#"{"layout":{"standard":{"png":{"SNAKING_IN_SLIDERS":"false"}}}}"#,
        ))
        .expect("valid boolean overlay");
        assert!(!config.layout.standard.png.SNAKING_IN_SLIDERS);
    }

    #[test]
    fn invalid_scalar_types_include_field_path() {
        let error = load_snapshot(Some(
            r#"{"layout":{"standard":{"png":{"SNAKING_IN_SLIDERS":"maybe"}}}}"#,
        ))
        .expect_err("must reject invalid boolean");
        assert!(error.contains("layout.standard.png.SNAKING_IN_SLIDERS"));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let error = load_snapshot(Some(r#"{"unknown":true}"#)).expect_err("must reject unknown");
        assert!(error.contains("unknown configuration field 'unknown'"));
    }

    #[test]
    fn invalid_timestamp_format_is_rejected() {
        let error = load_snapshot(Some(
            r#"{"logging":{"timestamp":{"LOCAL_FORMAT":"[not-a-format]"}}}"#,
        ))
        .expect_err("must reject invalid format");
        assert!(error.contains("LOCAL_FORMAT"));
    }
}
