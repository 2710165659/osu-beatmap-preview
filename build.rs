use serde_yaml::Value;
use std::env;
use std::fs;
use std::path::PathBuf;
use vergen::BuildBuilder;
use vergen::Emitter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let build = BuildBuilder::default().build_timestamp(true).build()?;
    Emitter::default().add_instructions(&build)?.emit()?;

    println!("cargo:rerun-if-changed=assets/default_config.yml");
    let source = fs::read_to_string("assets/default_config.yml")?;
    let mut generated = String::from("// @generated from assets/default_config.yml\n");
    let modules: serde_yaml::Mapping = serde_yaml::from_str(&source)?;
    for (module, sections) in modules {
        let module = module.as_str().ok_or("module name must be a string")?;
        let sections = sections.as_mapping().ok_or("module must be a mapping")?;
        // 皮肤配置仅通过下方的类型化运行时快照暴露；旧版常量模块无法表示
        // Mania 各键数对应的嵌套配置块。
        if module == "skin" {
            continue;
        }
        generated.push_str(&format!("#[allow(dead_code)]\npub mod {module} {{\n"));
        if module == "paths" {
            generate_entries_from_mapping(&mut generated, sections, &[module])?;
            generated.push_str("}\n");
            continue;
        }
        for (section, entries) in sections {
            let section = section.as_str().ok_or("section name must be a string")?;
            generated.push_str(&format!("#[allow(dead_code)]\npub mod {section} {{\n"));
            if module == "layout" {
                let tiers = entries
                    .as_mapping()
                    .ok_or("layout mode must be a mapping")?;
                for (tier, tier_entries) in tiers {
                    let tier = tier.as_str().ok_or("layout tier name must be a string")?;
                    generated.push_str(&format!("#[allow(dead_code)]\npub mod {tier} {{\n"));
                    generate_entries(&mut generated, tier_entries, &[module, section, tier])?;
                    generated.push_str("}\n");
                }
            } else {
                generated.push_str(module_prelude(module, section));
                generate_entries(&mut generated, entries, &[module, section])?;
            }
            generated.push_str("}\n");
        }
        generated.push_str("}\n");
    }
    // 在调用方迁移到运行时快照期间，让生成的模式与旧版常量保持分离。
    let source_value: Value = serde_yaml::from_str(&source)?;
    generate_runtime_schema(&mut generated, &source_value)?;
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR missing")?);
    fs::write(out_dir.join("config_constants.rs"), generated)?;
    Ok(())
}

fn generate_runtime_schema(
    generated: &mut String,
    root: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let mapping = root.as_mapping().ok_or("root must be a mapping")?;
    generated.push_str("\n#[derive(Clone, Debug, serde::Deserialize)]\n#[serde(deny_unknown_fields)]\n#[allow(dead_code, non_snake_case, non_camel_case_types)]\npub struct RuntimeConfig {\n");
    for (key, value) in mapping {
        let key = key.as_str().ok_or("config key must be a string")?;
        let ty = runtime_type_name(&[key], value)?;
        generated.push_str(&format!("    pub {key}: {ty},\n"));
    }
    generated.push_str("}\n");
    for (key, value) in mapping {
        let key = key.as_str().ok_or("config key must be a string")?;
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

fn generate_entries(
    generated: &mut String,
    raw_entries: &Value,
    path: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    let entries = raw_entries
        .as_mapping()
        .ok_or("section must be a mapping")?;
    generate_entries_from_mapping(generated, entries, path)
}

fn generate_entries_from_mapping(
    generated: &mut String,
    entries: &serde_yaml::Mapping,
    path: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    for (name, raw_entry) in entries {
        let name = name.as_str().ok_or("constant name must be a string")?;
        generated.push_str(&generate_constant(path, name, raw_entry)?);
    }
    Ok(())
}

fn module_prelude(module: &str, section: &str) -> &'static str {
    match (module, section) {
        ("network", "downloader_osz") => "use std::time::Duration;\n",
        ("timeouts", "render") => "use std::time::Duration;\n",
        _ => "",
    }
}

fn generate_constant(
    path: &[&str],
    name: &str,
    raw_value: &Value,
) -> Result<String, Box<dyn std::error::Error>> {
    let value = rust_literal(raw_value)?;
    let kind = special_kind(path, name);
    let rust_type = inferred_type(path, name, raw_value);
    let declaration = match kind {
        Some("duration_secs") => {
            format!("pub const {name}: std::time::Duration = Duration::from_secs({value});\n")
        }
        Some("positive_duration_secs") => {
            format!("pub const {name}: std::time::Duration = Duration::from_secs({value});\n")
        }
        Some("test_axis") => format!("pub const {name}: TimeAxis = TimeAxis::new({value});\n"),
        _ => format!("pub const {name}: {rust_type} = {value};\n"),
    };
    Ok(declaration)
}

fn special_kind(path: &[&str], name: &str) -> Option<&'static str> {
    match path {
        ["network", "downloader_osz"]
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
        ["timeouts", "render"] if matches!(name, "PNG_TIMEOUT" | "GIF_TIMEOUT" | "MP4_TIMEOUT") => {
            Some("positive_duration_secs")
        }
        _ => None,
    }
}

fn inferred_type(path: &[&str], name: &str, value: &Value) -> String {
    match value {
        Value::Bool(_) => "bool".to_string(),
        Value::String(_) => "&str".to_string(),
        Value::Number(number) if number.is_f64() => "f64".to_string(),
        Value::Number(_) => integer_type(path, name).to_string(),
        Value::Sequence(values) => sequence_type(values),
        Value::Null => "()".to_string(),
        Value::Mapping(_) | Value::Tagged(_) => "()".to_string(),
    }
}

fn integer_type(path: &[&str], name: &str) -> &'static str {
    if matches!(
        (path, name),
        (["network", "downloader_osz"], "PARALLEL_PARTS")
            | (["network", "downloader_osz"], "MAX_ACTIVE_ATTEMPTS")
            | (["layout", "catch", "gif"], "SEGMENT_COUNT")
            | (["layout", "taiko", "gif"], "SEGMENT_COUNT")
            | (["layout", "standard", "png"], "ROW_COUNT")
            | (["layout", "standard", "png"], "IMAGES_PER_ROW")
            | (["layout", "standard", "gif"], "ROW_COUNT")
            | (["layout", "standard", "gif"], "IMAGES_PER_ROW")
            | (["video", "composer"], "PAR_CHUNK_SIZE")
            | (["video", "composer"], "MAX_PAR_FRAME_BYTES")
            | (["video", "composer"], "PALETTE_COLORS")
            | (["video", "video"], "PAR_CHUNK_SIZE")
            | (["video", "video"], "MAX_PAR_FRAME_BYTES")
    ) {
        return "usize";
    }
    if matches!(
        (path, name),
        (["network", "downloader_osz"], "MIB_BYTES")
            | (["network", "downloader_osz"], "MAX_OSZ_BYTES")
            | (["network", "downloader_osz"], "LOW_SPEED_BYTES_PER_SECOND")
            | (["audio", "video_audio"], "MAX_EXTRACTED_AUDIO_BYTES")
    ) {
        return "u64";
    }
    if matches!(
        (path, name),
        (["layout", "catch", _], "RNG_SEED")
            | (["layout", "catch", "png"], "TIME_LABEL_FONT_SIZE")
            | (["layout", "catch", "gif"], "TIME_LABEL_FONT_SIZE")
            | (["layout", "catch", "gif"], "TIME_LABEL_NOTE_FONT_SIZE")
            | (["layout", "mania", "png"], "SV_TEXT_FONT_SIZE")
            | (["layout", "mania", "png"], "TIME_LABEL_FONT_SIZE")
            | (["layout", "mania", "gif"], "SV_TEXT_FONT_SIZE")
            | (["layout", "mania", "gif"], "TIME_LABEL_FONT_SIZE")
            | (["layout", "mania", "gif"], "TIME_LABEL_NOTE_FONT_SIZE")
            | (["layout", "standard", _], "TIME_LABEL_FONT_SIZE")
            | (["layout", "standard", _], "TIME_LABEL_NOTE_FONT_SIZE")
            | (["layout", "standard", _], "BREAK_OVERLAY_COUNTER_FONT_SIZE")
            | (["layout", "standard", _], "BREAK_OVERLAY_INFO_FONT_SIZE")
            | (["layout", "taiko", "png"], "TIME_LABEL_FONT_SIZE")
            | (["layout", "taiko", "png"], "TIME_LABEL_NOTE_FONT_SIZE")
            | (["layout", "taiko", "png"], "BPM_FONT_SIZE")
            | (["layout", "taiko", "png"], "SV_TEXT_FONT_SIZE")
            | (["layout", "taiko", "gif"], "TIME_LABEL_FONT_SIZE")
            | (["layout", "taiko", "gif"], "TIME_LABEL_NOTE_FONT_SIZE")
            | (["video", "video"], "LABEL_FONT_SIZE")
            | (["video", "video"], "VIDEO_BITRATE")
            | (["video", "video_cpu"], "CPU_VIDEO_BITRATE")
            | (["audio", "video_audio"], "AUDIO_SAMPLE_RATE")
            | (["audio", "video_audio"], "AUDIO_BITRATE")
    ) {
        return "u32";
    }
    "i64"
}

fn sequence_type(values: &[Value]) -> String {
    if values.iter().all(|value| matches!(value, Value::String(_))) {
        return format!("[&str; {}]", values.len());
    }
    if values.iter().all(is_byte_value) {
        return format!("[u8; {}]", values.len());
    }
    if values
        .iter()
        .all(|value| matches!(value, Value::Sequence(inner) if inner.iter().all(is_byte_value)))
    {
        let width = values
            .first()
            .and_then(|value| match value {
                Value::Sequence(inner) => Some(inner.len()),
                _ => None,
            })
            .unwrap_or(0);
        return format!("[[u8; {width}]; {}]", values.len());
    }
    "&[()]".to_string()
}

fn is_byte_value(value: &Value) -> bool {
    value
        .as_i64()
        .map(|number| (0..=255).contains(&number))
        .unwrap_or(false)
}

fn rust_literal(value: &Value) -> Result<String, Box<dyn std::error::Error>> {
    Ok(match value {
        Value::Null => "()".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")),
        Value::Sequence(values) => {
            let values = values
                .iter()
                .map(rust_literal)
                .collect::<Result<Vec<_>, _>>()?;
            format!("[{}]", values.join(", "))
        }
        Value::Mapping(_) => return Err("constant value must be scalar or sequence".into()),
        Value::Tagged(_) => return Err("tagged YAML values are not supported".into()),
    })
}
