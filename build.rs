use vergen::BuildBuilder;
use vergen::Emitter;
use serde_yaml::Value;
use std::env;
use std::fs;
use std::path::PathBuf;

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
        generated.push_str(&format!("#[allow(dead_code)]\npub mod {module} {{\n"));
        for (section, entries) in sections {
            let section = section.as_str().ok_or("section name must be a string")?;
            generated.push_str(&format!("#[allow(dead_code)]\npub mod {section} {{\n"));
            if module == "layout" {
                let tiers = entries.as_mapping().ok_or("layout mode must be a mapping")?;
                for (tier, tier_entries) in tiers {
                    let tier = tier.as_str().ok_or("layout tier name must be a string")?;
                    generated.push_str(&format!("#[allow(dead_code)]\npub mod {tier} {{\n"));
                    generate_entries(&mut generated, tier_entries, &[module, section, tier])?;
                    generated.push_str("}\n");
                }
            } else {
                generated.push_str(&module_prelude(&module, &section));
                generate_entries(&mut generated, entries, &[module, section])?;
            }
            generated.push_str("}\n");
        }
        generated.push_str("}\n");
    }
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR missing")?);
    fs::write(out_dir.join("config_constants.rs"), generated)?;
    Ok(())
}

fn generate_entries(
    generated: &mut String,
    raw_entries: &Value,
    path: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    let entries = raw_entries.as_mapping().ok_or("section must be a mapping")?;
    for (name, raw_entry) in entries {
        let name = name.as_str().ok_or("constant name must be a string")?;
        generated.push_str(&generate_constant(path, name, raw_entry)?);
    }
    Ok(())
}

fn module_prelude(module: &str, section: &str) -> &'static str {
    match (module, section) {
        ("network", "downloader_cf_ip") | ("network", "downloader_osz") => {
            "use std::time::Duration;\n"
        }
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
        Some("duration_ms") => format!(
            "pub const {name}: std::time::Duration = Duration::from_millis({value});\n"
        ),
        Some("duration_secs") => format!(
            "pub const {name}: std::time::Duration = Duration::from_secs({value});\n"
        ),
        Some("format") => format!(
            "pub const {name}: &[time::format_description::FormatItem<'static>] = time::macros::format_description!({value});\n"
        ),
        Some("test_axis") => format!("pub const {name}: TimeAxis = TimeAxis::new({value});\n"),
        _ => format!("pub const {name}: {rust_type} = {value};\n"),
    };
    Ok(declaration)
}

fn special_kind(path: &[&str], name: &str) -> Option<&'static str> {
    match path {
        ["logging", "timestamp"] if name == "LOCAL_FORMAT" => Some("format"),
        ["network", "downloader_cf_ip"] if name == "CACHE_TTL" => Some("duration_secs"),
        ["network", "downloader_cf_ip"] if matches!(name, "TCP_TIMEOUT" | "HTTP_TIMEOUT") => {
            Some("duration_ms")
        }
        ["network", "downloader_osz"] if name == "POLL_INTERVAL" => Some("duration_ms"),
        ["network", "downloader_osz"]
            if matches!(
                name,
                "NO_FIRST_BYTE_TIMEOUT"
                    | "LOW_SPEED_WINDOW"
                    | "DOWNLOAD_HARD_TIMEOUT"
                    | "CONNECT_TIMEOUT"
                    | "READ_TIMEOUT"
                    | "WRITE_TIMEOUT"
            ) => Some("duration_secs"),
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
        (["logging", "writer"], "MAX_LINE_BYTES")
            | (["network", "downloader_cf_ip"], "HTTP_CANDIDATES")
            | (["network", "downloader_osz"], "PARALLEL_PARTS")
            | (["network", "downloader_osz"], "MAX_ACTIVE_ATTEMPTS")
            | (["network", "downloader_osz"], "BUFFER_SIZE")
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
    if values.iter().all(|value| matches!(value, Value::Sequence(inner) if inner.iter().all(is_byte_value))) {
        let width = values
            .first()
            .and_then(|value| match value { Value::Sequence(inner) => Some(inner.len()), _ => None })
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
            let values = values.iter().map(rust_literal).collect::<Result<Vec<_>, _>>()?;
            format!("[{}]", values.join(", "))
        }
        Value::Mapping(_) => return Err("constant value must be scalar or sequence".into()),
        Value::Tagged(_) => return Err("tagged YAML values are not supported".into()),
    })
}
