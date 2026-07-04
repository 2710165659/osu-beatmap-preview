//! Output caching helpers: file-name formatting, mtime-based cache validity,
//! and deterministic-time checks.

use crate::models::KvSection;
use crate::mods::ModSettings;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Strip the Windows extended-length prefix `\\?\` if present.
pub fn clean_windows_path(path: &str) -> String {
    path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
}

/// Convert a `KvSection` into a JSON object with kebab-case keys.
pub fn format_section_keys(section: &KvSection) -> Value {
    let mut map = Map::new();
    for (key, value) in &section.entries {
        map.insert(kebab_case(key), Value::String(value.clone()));
    }
    Value::Object(map)
}

/// Convert CamelCase / PascalCase to kebab-case.
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

/// Build a filesystem-safe suffix from mod tokens (e.g. "dt1.5-hr").
pub fn format_mod_suffix(mods: &ModSettings) -> String {
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

/// Build a time-point suffix (e.g. "t10-20-30").
pub fn format_time_suffix(times: &[f64]) -> String {
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
pub fn output_cache_hit(
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
    if out_mtime < crate::build_time::build_time() {
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
    // mp4 is always deterministic: full-chart (±2s) is fixed by the beatmap,
    // and an explicit [t1, t2] range is user-pinned.
    if fmt == "mp4" {
        return true;
    }
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
