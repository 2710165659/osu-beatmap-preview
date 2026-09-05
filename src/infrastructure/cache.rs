//! 输出缓存辅助函数：基于修改时间的缓存有效性和原子写入。

use crate::domain::errors::{PreviewError, Result};
use crate::domain::models::KvSection;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 如果存在 Windows 扩展长度前缀 `\\?\`，则将其移除。
pub fn clean_windows_path(path: &str) -> String {
    path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
}

/// 将 `KvSection` 转换为键名使用 kebab-case 的 JSON 对象。
pub fn format_section_keys(section: &KvSection) -> Value {
    let mut map = Map::new();
    for (key, value) in &section.entries {
        map.insert(kebab_case(key), Value::String(value.clone()));
    }
    Value::Object(map)
}

/// 将 CamelCase / PascalCase 转换为 kebab-case。
fn kebab_case(key: &str) -> String {
    // 第一遍处理小写/数字到大写的边界，第二遍处理连续大写到大写小写的边界。
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
        // 连续大写与 [A-Z][a-z] 之间的边界。
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

// ── 输出缓存辅助函数 ──

/// 缓存输出仍有效时返回 `Some(path)`，否则返回 `None`。
pub fn output_cache_hit(
    output_path: &Path,
    beatmap_path: &Path,
    fmt: &str,
    _target_mode: i32,
    no_cache: bool,
) -> Option<PathBuf> {
    if no_cache {
        return None;
    }
    let out_meta = output_path.metadata().ok()?;
    if out_meta.len() == 0 {
        return None;
    }

    // 输出文件必须晚于程序构建时间。
    let out_mtime = out_meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    if out_mtime < crate::domain::build_time::build_time() {
        return None;
    }

    // 输出文件必须晚于谱面文件。
    if let Ok(beatmap_meta) = beatmap_path.metadata() {
        if let Ok(beatmap_mtime) = beatmap_meta.modified() {
            if out_mtime < beatmap_mtime {
                return None;
            }
        }
    }

    // 渲染在写入中断（例如进程被强制终止）会在最终路径留下截断文件。
    // 只有结构完整的输出才能从缓存提供，其余情况必须重新渲染。
    if !output_is_complete(output_path, fmt) {
        return None;
    }

    Some(output_path.to_path_buf())
}

// ── 原子输出 ──

/// 以原子方式写入输出文件：`write` 接收同目录临时路径并写入，
/// 仅在返回 `Ok` 后才将临时文件重命名覆盖 `output_path`。
/// 因此渲染中途被终止或 panic 不会在缓存最终路径留下残缺文件，
/// 最坏只会留下下次尝试前可清理的旧 `.tmp` 文件。
///
/// 临时文件与 `output_path` 位于同一目录，确保重命名在同一卷内完成并具备原子性。
/// Windows 的 `std::fs::rename` 会替换目标（`MOVEFILE_REPLACE_EXISTING`），
/// 因此旧的有效缓存会一直保留到新文件完整写入。若 `write` 失败，
/// 会尽力删除临时文件并原样返回错误，`output_path` 不受影响。
pub(crate) fn with_atomic_output<T>(
    output_path: &Path,
    tmp_suffix: &str,
    write: impl FnOnce(&Path) -> Result<T>,
) -> Result<T> {
    let file_name = output_path
        .file_name()
        .ok_or_else(|| PreviewError::render("invalid output path: missing file name"))?;
    let tmp_path =
        output_path.with_file_name(format!("{}.{}", file_name.to_string_lossy(), tmp_suffix));

    // 清理上次中断运行遗留的临时文件。
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

pub(crate) fn with_atomic_output_deadline<T>(
    output_path: &Path,
    tmp_suffix: &str,
    deadline: &crate::domain::timeout::RequestDeadline,
    write: impl FnOnce(&Path) -> Result<T>,
) -> Result<T> {
    with_atomic_output(output_path, tmp_suffix, |tmp_path| {
        let value = write(tmp_path)?;
        deadline.check()?;
        Ok(value)
    })
}

// ── 输出完整性校验 ──

/// 当已有输出文件在其格式上看起来结构完整时返回 `true`。
/// 中断渲染可能留下截断文件，缺少此检查会把它们误当作有效缓存。
pub(crate) fn output_is_complete(path: &Path, fmt: &str) -> bool {
    let Ok(data) = std::fs::read(path) else {
        return false;
    };
    match fmt {
        "mp4" => mp4_bytes_complete(&data),
        "gif" => gif_bytes_complete(&data),
        "png" => png_bytes_complete(&data),
        _ => true, // 未知格式不做校验。
    }
}

/// MP4 检查：文件必须以 `ftyp` brand box 开头，所有顶层 box 必须恰好对齐 EOF，
/// 且同时包含 `moov` 和 `mdat`。同时接受 faststart（`ftyp` + `moov` + `mdat`）
/// 与尾部索引（`ftyp` + `mdat` + `moov`）文件。
fn mp4_bytes_complete(data: &[u8]) -> bool {
    if data.len() < 16 {
        return false;
    }
    let mut pos = 0;
    let mut seen_ftyp = false;
    let mut seen_moov = false;
    let mut seen_mdat = false;

    while pos < data.len() {
        if pos + 8 > data.len() {
            return false;
        };
        let size32 = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap());
        let typ = &data[pos + 4..pos + 8];
        if pos == 0 && typ != b"ftyp" {
            return false;
        }
        let size = match size32 {
            0 => data.len() - pos,
            1 => {
                if pos + 16 > data.len() {
                    return false;
                }
                let large = u64::from_be_bytes(data[pos + 8..pos + 16].try_into().unwrap());
                let Ok(large) = usize::try_from(large) else {
                    return false;
                };
                if large < 16 {
                    return false;
                }
                large
            }
            n if n >= 8 => n as usize,
            _ => return false,
        };
        let Some(end) = pos.checked_add(size) else {
            return false;
        };
        if end > data.len() {
            return false;
        }
        match typ {
            b"ftyp" => seen_ftyp = true,
            b"moov" => seen_moov = true,
            b"mdat" => seen_mdat = true,
            _ => {}
        }
        pos = end;
    }

    seen_ftyp && seen_moov && seen_mdat
}

/// GIF 检查：尾标记字节 `0x3B` 必须是文件最后一个字节。
fn gif_bytes_complete(data: &[u8]) -> bool {
    data.last() == Some(&0x3B)
}

/// PNG 检查：文件必须以 `IEND` 区块结尾（4 字节类型后跟 4 字节 CRC，
/// 即 `IEND` 位于 `len - 8..len - 4`）。
fn png_bytes_complete(data: &[u8]) -> bool {
    data.len() >= 8 && &data[data.len() - 8..data.len() - 4] == b"IEND"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::timeout::RequestDeadline;
    use std::time::{Duration, Instant};

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
        data.extend_from_slice(&[0, 0, 0, 12]); // mdat box size
        data.extend_from_slice(b"mdat");
        data.extend_from_slice(&[1u8; 4]); // media payload
        data.extend_from_slice(&[0, 0, 0, 12]); // moov box size
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&[0u8; 4]); // payload
        data
    }

    #[test]
    fn mp4_complete_requires_aligned_ftyp_mdat_and_moov() {
        let complete = complete_mp4_bytes();
        assert!(mp4_bytes_complete(&complete));

        // faststart 顺序同样完整：ftyp + moov + mdat。
        let mut faststart = complete[..24].to_vec();
        faststart.extend_from_slice(&complete[36..48]);
        faststart.extend_from_slice(&complete[24..36]);
        assert!(mp4_bytes_complete(&faststart));

        // 截断：最后一个 box 的负载被截断，顶层 box 无法对齐到 EOF。
        let mut truncated = complete.clone();
        truncated.truncate(complete.len() - 4);
        assert!(!mp4_bytes_complete(&truncated));

        // 完全缺少 moov。
        let no_moov = complete[..36].to_vec();
        assert!(!mp4_bytes_complete(&no_moov));

        // 完全缺少 mdat。
        let mut no_mdat = complete[..24].to_vec();
        no_mdat.extend_from_slice(&complete[36..]);
        assert!(!mp4_bytes_complete(&no_mdat));

        // 缺少 ftyp。
        let mut no_ftyp = complete.clone();
        no_ftyp[4..8].copy_from_slice(b"junk");
        assert!(!mp4_bytes_complete(&no_ftyp));

        // 文件过短。
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
    fn atomic_output_timeout_keeps_existing_final_and_cleans_temp() {
        let dir = test_dir();
        let final_path = unique_path(&dir, "timeout.gif");
        let tmp = tmp_for(&final_path, "gif.tmp");
        std::fs::write(&final_path, b"old-cache").unwrap();
        let deadline = RequestDeadline::new(
            Instant::now() - Duration::from_secs(2),
            "gif",
            Duration::from_secs(1),
        );
        let result = with_atomic_output_deadline(&final_path, "gif.tmp", &deadline, |tmp_path| {
            std::fs::write(tmp_path, b"new-output")
                .map_err(|error| PreviewError::render(error.to_string()))
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
