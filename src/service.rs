use crate::errors::{PreviewError, Result};
use crate::models::{Beatmap, HitObjects};
use crate::mods::{mods_for_mode, validate_mods, ModSettings};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

pub fn generate_preview(
    bid: &str,
    fmt: Option<&str>,
    convert: Option<&str>,
    mods: Option<ModSettings>,
    times: Option<Vec<f64>>,
) -> Result<Value> {
    if bid.is_empty() || !bid.chars().all(|c| c.is_ascii_digit()) {
        return Err(PreviewError::new("bid must be numeric"));
    }

    let temp_root = std::env::temp_dir().join("osu-beatmap-preview");
    let beatmap_path =
        crate::downloader::download_beatmap_file(bid, &temp_root.join("osu-download-cache"))?;
    let beatmap = crate::parser::parse_beatmap(&beatmap_path)?;

    let mut target_mode = beatmap.mode();
    if let Some(convert_name) = convert {
        target_mode = crate::convert::resolve_convert_target(&beatmap, convert_name)?;
    }

    let fmt: String = match fmt {
        Some(f) => f.to_string(),
        None => {
            if target_mode == 0 {
                "gif".to_string()
            } else {
                "png".to_string()
            }
        }
    };
    if times.is_some() && fmt != "gif" {
        return Err(PreviewError::new("--times is only valid for GIF output"));
    }

    let mods: Option<ModSettings> = match mods {
        Some(m) if m.has_any_mod() => {
            let mode_errors = validate_mods(&m, Some(target_mode), Some(&fmt));
            if !mode_errors.is_empty() {
                return Err(PreviewError::new(format!(
                    "mod conflict: {}",
                    mode_errors.join("; ")
                )));
            }
            Some(mods_for_mode(&m, target_mode))
        }
        _ => None,
    };

    let mut parts: Vec<String> = vec![bid.to_string()];
    if let Some(convert_name) = convert {
        parts.push(convert_name.to_string());
    }
    if let Some(m) = &mods {
        if m.has_any_mod() {
            parts.push(format_mod_suffix(m));
        }
    }
    let output_path: PathBuf = temp_root
        .join("outputs")
        .join(format!("{}.{}", parts.join("_"), fmt));

    let preview_path =
        render_preview_for_mode(beatmap.clone(), &output_path, &fmt, target_mode, mods, times)?;

    let abs = preview_path
        .canonicalize()
        .unwrap_or(preview_path.clone());
    let abs_str = clean_windows_path(&abs.to_string_lossy());

    Ok(json!({
        "status": "success",
        "msg": format!("preview generated successfully for bid {bid}"),
        "preview-img": abs_str,
        "beatmap-info": {
            "meta-data": format_section_keys(&beatmap.metadata),
            "difficulty": format_section_keys(&beatmap.difficulty),
        },
    }))
}

fn clean_windows_path(path: &str) -> String {
    path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
}

fn format_section_keys(section: &crate::models::KvSection) -> Value {
    let mut map = Map::new();
    for (key, value) in &section.entries {
        map.insert(kebab_case(key), Value::String(value.clone()));
    }
    Value::Object(map)
}

fn kebab_case(key: &str) -> String {
    // pass 1: ([a-z0-9])([A-Z]) -> \1-\2 ; pass 2: ([A-Z]+)([A-Z][a-z]) -> \1-\2
    let chars: Vec<char> = key.chars().collect();
    let mut pass1 = String::with_capacity(key.len() + 4);
    for i in 0..chars.len() {
        pass1.push(chars[i]);
        if i + 1 < chars.len()
            && (chars[i].is_ascii_lowercase() || chars[i].is_ascii_digit())
            && chars[i + 1].is_ascii_uppercase()
        {
            pass1.push('-');
        }
    }
    let chars: Vec<char> = pass1.chars().collect();
    let mut pass2 = String::with_capacity(pass1.len() + 4);
    let mut i = 0;
    while i < chars.len() {
        pass2.push(chars[i]);
        // boundary between a run of uppercase and [A-Z][a-z]
        if chars[i].is_ascii_uppercase()
            && i + 2 < chars.len()
            && chars[i + 1].is_ascii_uppercase()
            && chars[i + 2].is_ascii_lowercase()
        {
            pass2.push('-');
        }
        i += 1;
    }
    pass2.to_lowercase()
}

fn format_mod_suffix(mods: &ModSettings) -> String {
    let tokens: Vec<String> = mods
        .tokens
        .iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.chars()
                .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '.' || *c == '-')
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return "mod".to_string();
    }
    format!("mod-{}", tokens.join("-"))
}

fn render_preview_for_mode(
    beatmap: Beatmap,
    output_path: &std::path::Path,
    fmt: &str,
    target_mode: i32,
    mods: Option<ModSettings>,
    times: Option<Vec<f64>>,
) -> Result<PathBuf> {
    let times_ms = crate::time_selection::times_to_milliseconds(times.as_deref());
    let mods_ref = mods.as_ref();

    match target_mode {
        0 => {
            let has_objects = matches!(&beatmap.hit_objects, HitObjects::Standard(v) if !v.is_empty());
            if !has_objects {
                return Err(PreviewError::new("standard beatmap has no hit objects"));
            }
            if fmt == "gif" {
                crate::standard::render_standard_gif(&beatmap, mods_ref, times_ms, output_path)?;
            } else {
                let image = crate::standard::render_standard_png(&beatmap, mods_ref)?;
                crate::composer::save_png(&image, output_path)?;
            }
            Ok(output_path.to_path_buf())
        }
        1 => {
            let beatmap = if beatmap.mode() != 1 {
                crate::convert::convert_beatmap(&beatmap, target_mode, mods_ref)?
            } else {
                beatmap
            };
            if fmt == "gif" {
                crate::taiko::render_taiko_gif(&beatmap, mods_ref, times_ms, output_path)?;
                Ok(output_path.to_path_buf())
            } else {
                crate::taiko::render_taiko_grid(&beatmap, output_path, mods_ref)
            }
        }
        2 => {
            let beatmap = if beatmap.mode() != 2 {
                crate::convert::convert_beatmap(&beatmap, target_mode, mods_ref)?
            } else {
                beatmap
            };
            if fmt == "gif" {
                crate::catch::render_catch_gif(&beatmap, mods_ref, times_ms, output_path)?;
                Ok(output_path.to_path_buf())
            } else {
                crate::catch::render_catch_grid(&beatmap, output_path, mods_ref)
            }
        }
        3 => {
            let beatmap = if beatmap.mode() != 3 {
                crate::convert::convert_beatmap(&beatmap, target_mode, mods_ref)?
            } else {
                beatmap
            };
            if fmt == "gif" {
                crate::mania::render_mania_gif(&beatmap, mods_ref, times_ms, output_path)?;
                Ok(output_path.to_path_buf())
            } else {
                crate::mania::render_mania_grid(&beatmap, output_path, mods_ref)
            }
        }
        _ => Err(PreviewError::new(format!(
            "unsupported beatmap mode: {target_mode}"
        ))),
    }
}
