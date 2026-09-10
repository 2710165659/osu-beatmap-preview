use serde_yaml::Value;
use std::env;
use std::fs;
use std::path::PathBuf;
use vergen::BuildBuilder;
use vergen::Emitter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let build = BuildBuilder::default().build_timestamp(true).build()?;
    Emitter::default().add_instructions(&build)?.emit()?;

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("manifest dir missing")?);
    let shared_config_path = manifest_dir.join("../../assets/shared_config.yml");
    let cli_config_path = manifest_dir.join("assets/cli_config.yml");
    for path in [&shared_config_path, &cli_config_path] {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let shared_source = fs::read_to_string(&shared_config_path)?;
    let mut source_value: Value = serde_yaml::from_str(&shared_source)?;
    let cli_source = fs::read_to_string(&cli_config_path)?;
    let cli_value: Value = serde_yaml::from_str(&cli_source)?;
    merge_values(&mut source_value, &cli_value)?;

    let mut generated = String::from(
        "// 由 assets/shared_config.yml 与 crates/osu-beatmap-preview-cli/assets/cli_config.yml 合并自动生成，请勿手动修改。\n",
    );
    generate_runtime_schema(&mut generated, &source_value)?;
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR missing")?);
    fs::write(out_dir.join("config_schema.rs"), generated)?;
    fs::write(
        out_dir.join("default_config.yml"),
        serde_yaml::to_string(&source_value)?,
    )?;
    Ok(())
}

fn merge_values(target: &mut Value, overlay: &Value) -> Result<(), Box<dyn std::error::Error>> {
    match (target, overlay) {
        (Value::Mapping(target), Value::Mapping(overlay)) => {
            for (key, value) in overlay {
                match target.get_mut(key) {
                    Some(existing) => merge_values(existing, value)?,
                    None => {
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
            Ok(())
        }
        (target, overlay) => {
            *target = overlay.clone();
            Ok(())
        }
    }
}

fn generate_runtime_schema(
    generated: &mut String,
    root: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let mapping = root.as_mapping().ok_or("root must be a mapping")?;
    generated.push_str("\n#[derive(Clone, Debug, serde::Deserialize)]\n#[serde(deny_unknown_fields)]\n#[allow(dead_code, non_snake_case, non_camel_case_types)]\npub struct RuntimeConfig {\n");
    for (key, value) in mapping {
        let key = key.as_str().ok_or("config key must be a string")?;
        let ty = match key {
            "render" => "osu_beatmap_preview_core::config::RenderConfig".to_string(),
            "skin" => "osu_beatmap_preview_core::config::SkinConfig".to_string(),
            _ => runtime_type_name(&[key], value)?,
        };
        generated.push_str(&format!("    pub {key}: {ty},\n"));
    }
    generated.push_str("}\n");
    for (key, value) in mapping {
        let key = key.as_str().ok_or("config key must be a string")?;
        if matches!(key, "render" | "skin") {
            continue;
        }
        generate_runtime_struct(generated, &[key], value)?;
    }
    Ok(())
}

fn generate_runtime_struct(
    generated: &mut String,
    path: &[&str],
    value: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mapping) = value.as_mapping() else {
        return Ok(());
    };
    let name = runtime_type_name(path, value)?;
    for (key, child) in mapping {
        let key = key.as_str().ok_or("config key must be a string")?;
        let mut child_path = path.to_vec();
        child_path.push(key);
        generate_runtime_struct(generated, &child_path, child)?;
    }
    generated.push_str(&format!(
        "\n#[derive(Clone, Debug, serde::Deserialize)]\n#[serde(deny_unknown_fields)]\n#[allow(dead_code, non_snake_case, non_camel_case_types)]\npub struct {name} {{\n"
    ));
    for (key, child) in mapping {
        let key = key.as_str().ok_or("config key must be a string")?;
        let mut child_path = path.to_vec();
        child_path.push(key);
        let ty = runtime_field_type(&child_path, child)?;
        if let Some(kind) = special_kind(path, key) {
            if kind == "duration_secs" {
                generated.push_str(
                    "    #[serde(deserialize_with = \"crate::config::deserialize_duration_secs\")]\n",
                );
            } else if kind == "positive_duration_secs" {
                generated.push_str(
                    "    #[serde(deserialize_with = \"crate::config::deserialize_positive_duration_secs\")]\n",
                );
            }
        }
        generated.push_str(&format!("    pub {key}: {ty},\n"));
    }
    generated.push_str("}\n");
    Ok(())
}

fn runtime_field_type(path: &[&str], value: &Value) -> Result<String, Box<dyn std::error::Error>> {
    if path.first() == Some(&"skin")
        && matches!(
            path.last(),
            Some(&"COLUMN_WIDTHS") | Some(&"COLUMN_LINE_WIDTHS")
        )
    {
        return Ok("Vec<i64>".to_string());
    }
    if let Some(kind) = special_kind(&path[..path.len() - 1], path[path.len() - 1]) {
        if matches!(kind, "duration_secs" | "positive_duration_secs") {
            return Ok("std::time::Duration".to_string());
        }
    }
    Ok(match value {
        Value::Mapping(_) => runtime_type_name(path, value)?,
        Value::Number(number) if number.is_f64() => "f64".to_string(),
        Value::Number(_) => integer_type(&path[..path.len() - 1], path[path.len() - 1]).to_string(),
        _ => runtime_scalar_type(value),
    })
}

fn runtime_type_name(path: &[&str], value: &Value) -> Result<String, Box<dyn std::error::Error>> {
    if !value.is_mapping() {
        return Ok(runtime_scalar_type(value));
    }
    let mut name = String::new();
    for part in path {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            name.push(first.to_ascii_uppercase());
            name.extend(chars);
        }
    }
    Ok(format!("{name}Config"))
}

fn runtime_scalar_type(value: &Value) -> String {
    match value {
        Value::Bool(_) => "bool".to_string(),
        Value::String(_) => "String".to_string(),
        Value::Number(number) if number.is_f64() => "f64".to_string(),
        Value::Number(_) => "i64".to_string(),
        Value::Sequence(values) => {
            if values.iter().all(|v| matches!(v, Value::String(_))) {
                "Vec<String>".to_string()
            } else if values.iter().all(is_byte_value) {
                format!("[u8; {}]", values.len())
            } else if values
                .iter()
                .all(|v| matches!(v, Value::Sequence(inner) if inner.iter().all(is_byte_value)))
            {
                let width = values
                    .first()
                    .and_then(|v| match v {
                        Value::Sequence(inner) => Some(inner.len()),
                        _ => None,
                    })
                    .unwrap_or(0);
                format!("Vec<[u8; {width}]>")
            } else {
                "Vec<serde_json::Value>".to_string()
            }
        }
        Value::Null | Value::Mapping(_) | Value::Tagged(_) => "serde_json::Value".to_string(),
    }
}

fn special_kind(path: &[&str], name: &str) -> Option<&'static str> {
    match path {
        ["download", "osz"]
            if matches!(
                name,
                "NO_FIRST_BYTE_TIMEOUT"
                    | "LOW_SPEED_WINDOW"
                    | "DOWNLOAD_HARD_TIMEOUT"
                    | "CONNECT_TIMEOUT"
                    | "READ_TIMEOUT"
                    | "WRITE_TIMEOUT"
            ) =>
        {
            Some("duration_secs")
        }
        ["timeout"] if matches!(name, "PNG_TIMEOUT" | "GIF_TIMEOUT" | "MP4_TIMEOUT") => {
            Some("positive_duration_secs")
        }
        _ => None,
    }
}

fn integer_type(path: &[&str], name: &str) -> &'static str {
    if matches!(
        name,
        "PARALLEL_PARTS"
            | "MAX_ACTIVE_ATTEMPTS"
            | "ROW_COUNT"
            | "IMAGES_PER_ROW"
            | "PAR_CHUNK_SIZE"
            | "MAX_PAR_FRAME_BYTES"
            | "PALETTE_COLORS"
    ) {
        return "usize";
    }
    if matches!(name, "MAX_OSZ_BYTES" | "MAX_EXTRACTED_AUDIO_BYTES") {
        return "u64";
    }
    if matches!(
        name,
        "TIME_LABEL_FONT_SIZE"
            | "TIME_LABEL_NOTE_FONT_SIZE"
            | "LABEL_FONT_SIZE"
            | "BPM_FONT_SIZE"
            | "SV_TEXT_FONT_SIZE"
            | "EDGE_COMBO_LABEL_FONT_SIZE"
            | "BREAK_OVERLAY_COUNTER_FONT_SIZE"
            | "BREAK_OVERLAY_INFO_FONT_SIZE"
            | "VIDEO_BITRATE"
            | "CPU_VIDEO_BITRATE"
            | "AUDIO_SAMPLE_RATE"
            | "AUDIO_BITRATE"
    ) {
        return "u32";
    }
    let _ = path;
    "i64"
}

fn is_byte_value(value: &Value) -> bool {
    value
        .as_i64()
        .map(|number| (0..=255).contains(&number))
        .unwrap_or(false)
}
