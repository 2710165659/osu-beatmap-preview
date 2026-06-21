use crate::errors::{PreviewError, Result};
use crate::models::{Beatmap, HitObjects};
use crate::mods::ModSettings;
use crate::validate::{self, ValidateContext};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub fn generate_preview(
    bid: &str,
    fmt: Option<&str>,
    convert: Option<&str>,
    mods: Option<ModSettings>,
    times: Option<Vec<f64>>,
    bpm: Option<f64>,
    no_cache: bool,
) -> Result<Value> {
    let temp_root = std::env::temp_dir().join("osu-beatmap-preview");
    let beatmap_path =
        crate::downloader::download_beatmap_file(bid, &temp_root.join("osu-download-cache"), no_cache)?;
    let beatmap = crate::parser::parse_beatmap(&beatmap_path)?;

    let mut target_mode = beatmap.mode();
    let mut convert_used: Option<&str> = None;
    if let Some(convert_name) = convert {
        let mode = crate::convert::resolve_convert_target(&beatmap, convert_name)?;
        if mode != beatmap.mode() {
            target_mode = mode;
            convert_used = Some(convert_name);
        }
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

    let ctx = ValidateContext {
        bid,
        fmt: &fmt,
        target_mode,
    };
    let mods = validate::validate_with_context(
        &ctx,
        times.as_deref(),
        bpm,
        mods,
    )?;

    let mode_name = match target_mode {
        0 => "standard",
        1 => "taiko",
        2 => "catch",
        3 => "mania",
        _ => "unknown",
    };

    let mut parts: Vec<String> = vec![mode_name.to_string(), bid.to_string()];
    if convert_used.is_some() {
        parts.push("convert".to_string());
    }
    if let Some(m) = &mods {
        if m.has_any_mod() {
            parts.push(format_mod_suffix(m));
        }
    }
    if let Some(t) = &times {
        if !t.is_empty() {
            parts.push(format_time_suffix(t));
        }
    }
    if let Some(b) = bpm {
        parts.push(format!("bpm{}", b));
    }
    let output_path: PathBuf = temp_root
        .join("outputs")
        .join(format!("{}.{}", parts.join("_"), fmt));

    // ── image cache check ──
    let cached = output_cache_hit(&output_path, &beatmap_path, &times, &fmt, target_mode, no_cache);
    if let Some(cached_path) = cached {
        let abs = cached_path
            .canonicalize()
            .unwrap_or(cached_path.clone());
        let abs_str = clean_windows_path(&abs.to_string_lossy());
        return Ok(json!({
            "status": "success",
            "msg": format!("preview generated successfully for bid {bid}"),
            "preview-img": abs_str,
            "beatmap-info": {
                "meta-data": format_section_keys(&beatmap.metadata),
                "difficulty": format_section_keys(&beatmap.difficulty),
            },
        }));
    }

    let preview_path =
        render_preview_for_mode(beatmap.clone(), &output_path, &fmt, target_mode, mods, times, bpm)?;

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
    tokens.join("-")
}

fn format_time_suffix(times: &[f64]) -> String {
    format!(
        "t{}",
        times
            .iter()
            .map(|t| format!("{}", t))
            .collect::<Vec<_>>()
            .join("-")
    )
}

// ── output cache helpers ──

/// Returns `Some(path)` if the cached output is still valid, `None` otherwise.
fn output_cache_hit(
    output_path: &Path,
    beatmap_path: &Path,
    times: &Option<Vec<f64>>,
    fmt: &str,
    target_mode: i32,
    no_cache: bool,
) -> Option<PathBuf> {
    if no_cache {
        return None;
    }
    let out_meta = output_path.metadata().ok()?;
    if out_meta.len() == 0 {
        return None;
    }

    // Output must be newer than the program build.
    let out_mtime = out_meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    if out_mtime < crate::utils::build_time() {
        return None;
    }

    // Output must be newer than the beatmap file.
    if let Ok(beatmap_meta) = beatmap_path.metadata() {
        if let Ok(beatmap_mtime) = beatmap_meta.modified() {
            if out_mtime < beatmap_mtime {
                return None;
            }
        }
    }

    // When random time selection is involved and the user did NOT pin ALL
    // required time points, the output is non-deterministic → never cache.
    if !all_times_pinned(fmt, target_mode, times) {
        return None;
    }

    Some(output_path.to_path_buf())
}

/// Returns `true` when the output is fully deterministic w.r.t. time selection.
///
/// * GIF (all modes): needs 4 segments → cache only when `--time` gives all 4.
/// * Standard PNG: needs 5 rows but `--time` accepts at most 4 → never cachable.
/// * Taiko / Catch / Mania PNG: no time selection at all → always cachable.
fn all_times_pinned(fmt: &str, target_mode: i32, times: &Option<Vec<f64>>) -> bool {
    // Modes that don't use PreviewTimeSelector at all are always deterministic.
    if fmt == "png" && target_mode != 0 {
        return true;
    }

    // GIF needs 4, std PNG needs 5 (but max allowed is 4 → unreachable).
    let needed: usize = if fmt == "gif" { 4 } else { 5 };
    match times {
        Some(ts) => ts.len() >= needed,
        None => false,
    }
}

fn render_preview_for_mode(
    beatmap: Beatmap,
    output_path: &std::path::Path,
    fmt: &str,
    target_mode: i32,
    mods: Option<ModSettings>,
    times: Option<Vec<f64>>,
    bpm: Option<f64>,
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
                let image = crate::standard::render_standard_png(&beatmap, mods_ref, times_ms)?;
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
                crate::taiko::render_taiko_grid(&beatmap, output_path, mods_ref, bpm)
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
