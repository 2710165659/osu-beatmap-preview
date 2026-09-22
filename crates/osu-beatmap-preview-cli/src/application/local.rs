//! 本地谱面文件（`.osu` / `.osz`）的读取与 `.osu` 查找规则。
//!
//! `.osz` 就是 ZIP。「压缩包里找 `.osu`」的判定对齐 osu! 的导入行为（参考
//! `E:\MyCodes\c#\PROJECTS\osu` 的 `osu.Game/Beatmaps/BeatmapImporter.cs`）：
//! 后缀 `.osu` 不区分大小写，且只认压缩包**顶层**的谱面——stable 会忽略子目录里的
//! `.osu`，osu!lazer 也是按 `!f.Filename.Contains('/')` 过滤的。
//!
//! 一个 `.osz` 通常含多个难度（多个 `.osu`），预览哪一个由请求的 `bid` 决定：
//! 按 `.osu` 的 `[Metadata] BeatmapID` 匹配，匹配不到直接报错，绝不随便挑一个，
//! 否则会预览到错误的难度。匹配过程会解析候选谱面（毫秒级），返回给调用方的仍
//! 是 `.osu` 字节，由调用方按自己的流程统一解析。

use osu_beatmap_preview_core::model::Beatmap;
use osu_beatmap_preview_core::processing::media::normalize_entry_path;
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
use std::io::Read;
use std::path::Path;

/// 单个 `.osu` 允许的最大字节数；正常谱面只有几 MiB，超过它一定不是谱面文本。
const MAX_BEATMAP_BYTES: u64 = 32 * 1024 * 1024;

/// 本地输入文件类型（按扩展名判定，不区分大小写）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalInputKind {
    Osu,
    Osz,
}

/// 按扩展名判定本地输入文件类型；其他后缀一律拒绝。
pub(crate) fn input_kind(path: &str) -> Result<LocalInputKind> {
    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "osu" => Ok(LocalInputKind::Osu),
        "osz" => Ok(LocalInputKind::Osz),
        _ => Err(PreviewError::new(format!(
            "--input-file must be a .osu or .osz file; got '{path}'"
        ))),
    }
}

/// 读取本地 `.osu` 文件的字节。
pub(crate) fn load_local_osu(path: &Path) -> Result<Vec<u8>> {
    read_file_bytes(path)
}

/// 从本地 `.osz` 中查找并读取指定 `bid` 的 `.osu` 字节。
///
/// 找不到任何顶层 `.osu`、或没有任何难度的 `BeatmapID` 等于 `bid` 时都返回错误：
/// `.osz` 靠 `bid` 指定难度，猜错难度比报错更糟。
pub(crate) fn load_local_osz(path: &Path, bid: &str) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path).map_err(|error| {
        PreviewError::new(format!(
            "failed to open input file {}: {error}",
            path.display()
        ))
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        PreviewError::new(format!("invalid .osz archive {}: {error}", path.display()))
    })?;

    // 顶层 `.osu` 条目清单；保持压缩包内顺序，同一压缩包的结果稳定可复现。
    let mut candidates = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| PreviewError::new(format!("failed to inspect .osz entry: {error}")))?;
        if is_beatmap_entry(entry.name()) {
            candidates.push((index, entry.name().to_string()));
        }
    }
    if candidates.is_empty() {
        return Err(PreviewError::new(format!(
            "no .osu beatmap file found in {}",
            path.display()
        )));
    }

    let wanted = bid.trim().parse::<u64>().ok();
    let mut seen = Vec::new();
    let mut first_parse_error = None;
    for (index, name) in candidates {
        let bytes = read_entry_bytes(&mut archive, index, &name)?;
        let beatmap = match osu_beatmap_preview_core::parse_beatmap_bytes(&bytes) {
            Ok(beatmap) => beatmap,
            Err(error) => {
                // 单个损坏的 .osu 不影响其他难度参与匹配，只在全部失败时报出去。
                first_parse_error.get_or_insert_with(|| format!("{name}: {error}"));
                continue;
            }
        };
        let id = beatmap_id(&beatmap);
        seen.push(match id {
            Some(id) => format!("{name} (BeatmapID={id})"),
            None => format!("{name} (BeatmapID=none)"),
        });
        if wanted.is_some() && id == wanted {
            return Ok(bytes);
        }
    }

    if seen.is_empty() {
        return Err(PreviewError::new(format!(
            "failed to parse .osu beatmap files in {}: {}",
            path.display(),
            first_parse_error.unwrap_or_else(|| "unknown error".to_string())
        )));
    }
    Err(PreviewError::new(format!(
        "beatmap {bid} was not found in {} (.osu BeatmapID values: {})",
        path.display(),
        seen.join(", ")
    )))
}

/// 请求、产物与日志使用的谱面 ID。
///
/// 优先用户给的 `bid`；本地 `.osu` 可以没有 `bid`，此时用谱面的 `BeatmapID`，
/// 连它也没有就退回文件名主干（只保留文件名安全字符），保证不同文件的输出
/// 不会写进同一个产物名里。
pub(crate) fn effective_id(bid: &str, beatmap: &Beatmap, path: &Path) -> String {
    if !bid.is_empty() {
        return bid.to_string();
    }
    if let Some(id) = beatmap_id(beatmap) {
        return id.to_string();
    }
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let sanitized: String = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches(|character| character == '-' || character == '.');
    if sanitized.is_empty() {
        "local".to_string()
    } else {
        sanitized.to_string()
    }
}

/// 压缩包条目是否是可预览的 `.osu` 谱面文件（顶层、后缀 `.osu`）。
fn is_beatmap_entry(name: &str) -> bool {
    let Some(normalized) = normalize_entry_path(name) else {
        return false;
    };
    // stable 会忽略子目录里的 .osu，osu!lazer 同样只取顶层条目。
    if normalized.contains('/') {
        return false;
    }
    normalized.to_ascii_lowercase().ends_with(".osu")
}

/// 谱面的 `[Metadata] BeatmapID`；缺失或非正数时返回 `None`。
fn beatmap_id(beatmap: &Beatmap) -> Option<u64> {
    beatmap
        .metadata
        .get("BeatmapID")
        .and_then(|value| value.trim().parse().ok())
        .filter(|id| *id > 0)
}

fn read_file_bytes(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path).map_err(|error| {
        PreviewError::new(format!(
            "failed to read input file {}: {error}",
            path.display()
        ))
    })?;
    if bytes.len() as u64 > MAX_BEATMAP_BYTES {
        return Err(PreviewError::new(format!(
            "input file {} is too large to be a beatmap",
            path.display()
        )));
    }
    Ok(bytes)
}

/// 读出一个压缩包条目的全部字节；空条目或超限条目按非法谱面处理。
fn read_entry_bytes(
    archive: &mut zip::ZipArchive<std::fs::File>,
    index: usize,
    name: &str,
) -> Result<Vec<u8>> {
    let mut entry = archive
        .by_index(index)
        .map_err(|error| PreviewError::new(format!("failed to open .osz entry {name}: {error}")))?;
    if entry.is_dir() || entry.size() == 0 || entry.size() > MAX_BEATMAP_BYTES {
        return Err(PreviewError::new(format!(
            ".osz entry {name} has an invalid size: {} bytes",
            entry.size()
        )));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .by_ref()
        .take(MAX_BEATMAP_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| PreviewError::new(format!("failed to read .osz entry {name}: {error}")))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    /// 用给定条目构造一个内存 .osz，返回可当文件读的字节。
    fn osz_bytes(entries: &[(&str, String)]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            for (name, content) in entries {
                writer
                    .start_file(*name, SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(content.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn write_temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "osu-preview-local-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn osu_text(beatmap_id: Option<u32>, version: &str) -> String {
        let id = beatmap_id
            .map(|value| format!("BeatmapID:{value}\n"))
            .unwrap_or_default();
        format!(
            "osu file format v14\n\n[General]\nAudioFilename: audio.mp3\nMode: 0\n\n\
             [Metadata]\nTitle:Test\nArtist:Someone\nCreator:Me\nVersion:{version}\n{id}\n\
             [Difficulty]\nCircleSize:4\nSliderMultiplier:1.4\nSliderTickRate:1\n\n\
             [TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n"
        )
    }

    fn parse(bytes: &[u8]) -> Beatmap {
        osu_beatmap_preview_core::parse_beatmap_bytes(bytes).expect("fixture 必须可解析")
    }

    /// 扩展名判定接受大小写，其他后缀一律拒绝。
    #[test]
    fn input_kind_accepts_osu_and_osz_only() {
        assert_eq!(input_kind("map.osu").unwrap(), LocalInputKind::Osu);
        assert_eq!(input_kind(r"C:\maps\MAP.OSZ").unwrap(), LocalInputKind::Osz);
        assert!(input_kind("map.osk").is_err());
        assert!(input_kind("map").is_err());
        assert!(input_kind("").is_err());
    }

    /// 只认顶层 `.osu`：子目录里的谱面会被 stable 忽略（lazer 同样如此）。
    #[test]
    fn only_top_level_osu_entries_are_beatmaps() {
        assert!(is_beatmap_entry("map.osu"));
        assert!(is_beatmap_entry("MAP.OSU"));
        assert!(is_beatmap_entry(r"map.Osu"));
        assert!(!is_beatmap_entry("sub/map.osu"));
        assert!(!is_beatmap_entry("map.osz"));
        assert!(!is_beatmap_entry("../map.osu"));
        assert!(!is_beatmap_entry("map.osu.bak"));
    }

    /// 按 BeatmapID 在 .osz 内匹配难度。
    #[test]
    fn osz_selects_the_difficulty_matching_bid() {
        let bytes = osz_bytes(&[
            ("Easy.osu", osu_text(Some(100), "Easy")),
            ("Hard.osu", osu_text(Some(200), "Hard")),
        ]);
        let path = write_temp("fixture.osz", &bytes);
        let matched = load_local_osz(&path, "200").unwrap();
        assert_eq!(parse(&matched).metadata.get("Version"), Some("Hard"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// bid 匹配不到任何难度时报错，并列出压缩包内实际存在的 BeatmapID。
    #[test]
    fn osz_without_matching_bid_reports_available_ids() {
        let bytes = osz_bytes(&[("Easy.osu", osu_text(Some(100), "Easy"))]);
        let path = write_temp("fixture.osz", &bytes);
        let error = load_local_osz(&path, "999").unwrap_err().to_string();
        assert!(error.contains("beatmap 999 was not found"), "{error}");
        assert!(error.contains("BeatmapID=100"), "{error}");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// 压缩包里没有任何顶层 .osu 时报错；子目录里的 .osu 不算。
    #[test]
    fn osz_without_top_level_osu_is_rejected() {
        let bytes = osz_bytes(&[
            ("sub/Hard.osu", osu_text(Some(200), "Hard")),
            ("audio.mp3", "not-audio".to_string()),
        ]);
        let path = write_temp("fixture.osz", &bytes);
        let error = load_local_osz(&path, "200").unwrap_err().to_string();
        assert!(error.contains("no .osu beatmap file found"), "{error}");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// 本地 `.osu` 直接读取字节，不受 bid 影响。
    #[test]
    fn local_osu_file_is_read_directly() {
        let source = osu_text(Some(300), "Extra");
        let path = write_temp("plain.osu", source.as_bytes());
        assert_eq!(load_local_osu(&path).unwrap(), source.as_bytes());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// 谱面 ID：bid > BeatmapID > 文件名主干。
    #[test]
    fn effective_id_prefers_bid_then_beatmap_id_then_file_stem() {
        let path = Path::new(r"C:\maps\My Map [Hard].osu");
        let with_id = parse(osu_text(Some(300), "Hard").as_bytes());
        assert_eq!(effective_id("777", &with_id, path), "777");
        assert_eq!(effective_id("", &with_id, path), "300");

        let without_id = parse(osu_text(None, "Hard").as_bytes());
        assert_eq!(effective_id("", &without_id, path), "My-Map--Hard");
    }
}
