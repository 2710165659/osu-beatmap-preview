//! Output caching helpers: file-name formatting, mtime-based cache validity,
//! and deterministic-time checks.

use crate::core::models::KvSection;
use crate::core::mods::ModSettings;
use crate::core::errors::{PreviewError, Result};
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
    if out_mtime < crate::core::build_time::build_time() {
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

    // A render interrupted mid-write (e.g. the process was force-killed) leaves
    // a truncated file at the final path. Only structurally complete outputs
    // may be served from cache; anything else is re-rendered from scratch.
    if !output_is_complete(&output_path, fmt) {
        return None;
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

// ── atomic output ──

/// Write an output file atomically: `write` receives a sibling temporary path
/// and must produce the file there; only after it returns `Ok` is the temp
/// file renamed over `output_path`. A render killed mid-write (forced process
/// termination, panic) can therefore never leave a partial file at the final
/// cache path — at worst a stale `.tmp` file is left, which is removed before
/// the next attempt.
///
/// The temp file lives in the same directory as `output_path` so the final
/// rename stays on one volume and is atomic. On Windows `std::fs::rename`
/// replaces an existing destination (`MOVEFILE_REPLACE_EXISTING`), so a
/// previously good cache stays intact until the new file is complete. If
/// `write` fails, the temp file is removed best-effort and the error is
/// propagated with `output_path` untouched.
pub(crate) fn with_atomic_output<T>(
    output_path: &Path,
    tmp_suffix: &str,
    write: impl FnOnce(&Path) -> Result<T>,
) -> Result<T> {
    let file_name = output_path
        .file_name()
        .ok_or_else(|| PreviewError::render("invalid output path: missing file name"))?;
    let tmp_path = output_path.with_file_name(format!(
        "{}.{}",
        file_name.to_string_lossy(),
        tmp_suffix
    ));

    // Clear any temp file left by a previous interrupted run.
    let _ = std::fs::remove_file(&tmp_path);

    let result = write(&tmp_path);
    match result {
        Ok(value) => {
            std::fs::rename(&tmp_path, output_path).map_err(|e| {
                PreviewError::render(format!(
                    "failed to finalize output file '{}' (is it open in another program?): {e}",
                    output_path.display()
                ))
            })?;
            Ok(value)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&tmp_path);
            Err(error)
        }
    }
}

// ── output completeness validation ──

/// Return `true` when an existing output file looks structurally complete for
/// its format. Interrupted renders leave truncated files behind; without this
/// check they would be served from cache as if they were valid.
pub(crate) fn output_is_complete(path: &Path, fmt: &str) -> bool {
    let Ok(data) = std::fs::read(path) else {
        return false;
    };
    match fmt {
        "mp4" => mp4_bytes_complete(&data),
        "gif" => gif_bytes_complete(&data),
        "png" => png_bytes_complete(&data),
        _ => true, // unknown formats are not validated
    }
}

/// MP4 check: the file must start with an `ftyp` brand box and end with a
/// `moov` box whose size field aligns exactly with EOF. Our muxer writes the
/// moov last, so a truncated render (killed mid-`mdat`) fails the alignment.
fn mp4_bytes_complete(data: &[u8]) -> bool {
    // First box must be ftyp (4-byte size + "ftyp" brand).
    if data.len() < 16 || &data[4..8] != b"ftyp" {
        return false;
    }
    // Search backwards so the final box is checked first; keep looking for an
    // earlier "moov" if the latest one does not align with EOF.
    let mut search_end = data.len();
    while search_end >= 4 {
        let Some(rel) = data[..search_end]
            .windows(4)
            .rposition(|w| w == b"moov")
        else {
            return false;
        };
        if rel >= 4 {
            let size_pos = rel - 4;
            let size = u32::from_be_bytes([
                data[size_pos],
                data[size_pos + 1],
                data[size_pos + 2],
                data[size_pos + 3],
            ]);
            let box_start = size_pos as u64;
            if size == 1 {
                // 64-bit extended size: [size=1][type][largesize u64][payload]
                if rel + 8 <= data.len() {
                    let mut large = [0u8; 8];
                    large.copy_from_slice(&data[rel..rel + 8]);
                    if box_start + 16 + u64::from_be_bytes(large) == data.len() as u64 {
                        return true;
                    }
                }
            } else if box_start + u64::from(size) == data.len() as u64 {
                return true;
            }
        }
        search_end = rel;
    }
    false
}

/// GIF check: the trailer byte `0x3B` must be the last byte of the file.
fn gif_bytes_complete(data: &[u8]) -> bool {
    data.last() == Some(&0x3B)
}

/// PNG check: the file must end with an `IEND` chunk (4-byte type followed by
/// a 4-byte CRC, i.e. `IEND` at `len - 8..len - 4`).
fn png_bytes_complete(data: &[u8]) -> bool {
    data.len() >= 8 && &data[data.len() - 8..data.len() - 4] == b"IEND"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "osu-beatmap-preview-cache-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn unique_path(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(format!("{}-{}", name, std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn tmp_for(final_path: &Path, tmp_suffix: &str) -> PathBuf {
        final_path.with_file_name(format!(
            "{}.{}",
            final_path.file_name().unwrap().to_string_lossy(),
            tmp_suffix
        ))
    }

    fn complete_mp4_bytes() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&[0, 0, 0, 24]); // ftyp box size
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(&[0u8; 16]); // brand payload
        data.extend_from_slice(&[0, 0, 0, 12]); // moov box size
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&[0u8; 4]); // payload
        data
    }

    #[test]
    fn mp4_complete_requires_ftyp_and_trailing_moov() {
        let complete = complete_mp4_bytes();
        assert!(mp4_bytes_complete(&complete));

        // truncated: moov payload cut off → moov size no longer aligns with EOF
        let mut truncated = complete.clone();
        truncated.truncate(complete.len() - 4);
        assert!(!mp4_bytes_complete(&truncated));

        // moov missing entirely
        let no_moov = complete[..24].to_vec();
        assert!(!mp4_bytes_complete(&no_moov));

        // ftyp missing
        let mut no_ftyp = complete.clone();
        no_ftyp[4..8].copy_from_slice(b"junk");
        assert!(!mp4_bytes_complete(&no_ftyp));

        // too short
        assert!(!mp4_bytes_complete(&[0u8; 8]));
    }

    #[test]
    fn gif_complete_requires_trailer() {
        let mut complete = b"GIF89a".to_vec();
        complete.extend_from_slice(&[0u8; 10]);
        complete.push(0x3B);
        assert!(gif_bytes_complete(&complete));

        complete.pop();
        assert!(!gif_bytes_complete(&complete));
        assert!(!gif_bytes_complete(&[]));
    }

    #[test]
    fn png_complete_requires_iend_tail() {
        let mut complete = b"\x89PNG\r\n\x1a\n".to_vec();
        complete.extend_from_slice(&[0, 0, 0, 0]);
        complete.extend_from_slice(b"IEND");
        complete.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]);
        assert!(png_bytes_complete(&complete));

        complete.truncate(complete.len() - 4);
        assert!(!png_bytes_complete(&complete));
        assert!(!png_bytes_complete(b"no-iend-here"));
    }

    #[test]
    fn output_is_complete_reads_file_and_dispatches_by_format() {
        let dir = test_dir();
        let complete = complete_mp4_bytes();
        let path = unique_path(&dir, "check.mp4");
        std::fs::write(&path, &complete).unwrap();
        assert!(output_is_complete(&path, "mp4"));

        std::fs::write(&path, &complete[..24]).unwrap();
        assert!(!output_is_complete(&path, "mp4"));
        assert!(output_is_complete(&path, "unknown"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn atomic_output_success_writes_final_and_removes_temp() {
        let dir = test_dir();
        let final_path = unique_path(&dir, "out.mp4");
        let tmp = tmp_for(&final_path, "mp4.tmp");
        let result = with_atomic_output(&final_path, "mp4.tmp", |tmp_path| {
            std::fs::write(tmp_path, b"complete").map_err(|e| PreviewError::render(e.to_string()))
        });
        assert!(result.is_ok());
        assert_eq!(std::fs::read(&final_path).unwrap(), b"complete");
        assert!(!tmp.exists());
        let _ = std::fs::remove_file(&final_path);
    }

    #[test]
    fn atomic_output_error_keeps_existing_final_and_cleans_temp() {
        let dir = test_dir();
        let final_path = unique_path(&dir, "out.gif");
        let tmp = tmp_for(&final_path, "gif.tmp");
        std::fs::write(&final_path, b"old-cache").unwrap();
        let result = with_atomic_output(&final_path, "gif.tmp", |_tmp| -> Result<()> {
            Err(PreviewError::render("boom"))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&final_path).unwrap(), b"old-cache");
        assert!(!tmp.exists());
        let _ = std::fs::remove_file(&final_path);
    }

    #[test]
    fn atomic_output_removes_stale_temp_before_write() {
        let dir = test_dir();
        let final_path = unique_path(&dir, "out.png");
        let tmp = tmp_for(&final_path, "png.tmp");
        std::fs::write(&tmp, b"stale").unwrap();
        with_atomic_output(&final_path, "png.tmp", |tmp_path| {
            std::fs::write(tmp_path, b"new").map_err(|e| PreviewError::render(e.to_string()))
        })
        .unwrap();
        assert_eq!(std::fs::read(&final_path).unwrap(), b"new");
        assert!(!tmp.exists());
        let _ = std::fs::remove_file(&final_path);
    }
}
