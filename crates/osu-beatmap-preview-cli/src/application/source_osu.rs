//! 本地谱面文件（`.osu` / `.osz`）的读取与 `.osu` 查找规则。
//!
//! `.osz` 就是 ZIP。「压缩包里找 `.osu`」的判定对齐 osu! 的导入行为（参考
//! `E:\MyCodes\c#\PROJECTS\osu` 的 `osu.Game/Beatmaps/BeatmapImporter.cs`）：
//! 后缀 `.osu` 不区分大小写，且只认压缩包**顶层**的谱面——stable 会忽略子目录里的
//! `.osu`，osu!lazer 也是按 `!f.Filename.Contains('/')` 过滤的。
//!
//! 多难度 `.osz` 按 `[Metadata] BeatmapID` 选择谱面；只有一个顶层 `.osu` 时
//! 可以省略 `bid`。外部 `.osu` 优先用于渲染，搭配的 `.osz` 必须包含相同 ID，
//! 避免将其他难度的媒体资源与当前谱面混用。

use osu_beatmap_preview_core::model::Beatmap;
use osu_beatmap_preview_core::processing::media::normalize_entry_path;
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

/// 单个 `.osu` 允许的最大字节数；正常谱面只有几 MiB，超过它一定不是谱面文本。
const MAX_BEATMAP_BYTES: u64 = 32 * 1024 * 1024;

/// 路径扩展名必须与参数用途相符，不区分大小写。
pub(crate) fn validate_local_path(path: &str, extension: &str) -> Result<()> {
    if path.trim().is_empty() {
        return Err(PreviewError::new(format!(
            "--{extension}-file must not be empty"
        )));
    }
    let actual = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if !actual.eq_ignore_ascii_case(extension) {
        return Err(PreviewError::new(format!(
            "--{extension}-file must be a .{extension} file; got '{path}'"
        )));
    }
    Ok(())
}

/// 本地谱面文本与媒体来源；两者可以分别来自外部 `.osu` 和 `.osz`。
pub(crate) struct LocalBeatmap {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub osz_path: Option<PathBuf>,
}

/// 读取本地来源并校验所有 ID；必须在输出缓存判断前执行，避免缓存掩盖输入错误。
pub(crate) fn load_local_source(
    osu_file: Option<&str>,
    osz_file: Option<&str>,
    bid: &str,
) -> Result<LocalBeatmap> {
    let osz_path = osz_file.map(PathBuf::from);
    if let Some(osu_file) = osu_file {
        let path = PathBuf::from(osu_file);
        let bytes = read_file_bytes(&path)?;
        // 单独提供 .osu 且无 bid 时允许无 ID 谱面，沿用文件名作为产物 ID 的回退。
        if !bid.is_empty() || osz_path.is_some() {
            let beatmap =
                osu_beatmap_preview_core::parse_beatmap_bytes(&bytes).map_err(|error| {
                    PreviewError::parse(format!("failed to parse {}: {error}", path.display()))
                })?;
            let id = beatmap_id(&beatmap).ok_or_else(|| {
                PreviewError::new(format!(
                    "--osu-file {} must have a valid positive BeatmapID when --bid or --osz-file is provided",
                    path.display()
                ))
            })?;
            if !bid.is_empty() && bid.parse::<u64>().ok() != Some(id) {
                return Err(PreviewError::new(format!(
                    "--bid {bid} does not match --osu-file {} (BeatmapID={id})",
                    path.display()
                )));
            }
            if let Some(archive_path) = osz_path.as_deref() {
                // 只校验包内存在相同 ID，实际渲染字节始终来自外部 .osu。
                load_local_osz(archive_path, &id.to_string())?;
            }
        }
        return Ok(LocalBeatmap {
            path,
            bytes,
            osz_path,
        });
    }
    let path = osz_path
        .as_ref()
        .ok_or_else(|| PreviewError::new("--osu-file or --osz-file is required for local input"))?;
    let bytes = load_local_osz(path, bid)?;
    Ok(LocalBeatmap {
        path: path.clone(),
        bytes,
        osz_path,
    })
}

/// 从本地 `.osz` 中读取指定 ID 的谱面；省略 ID 时必须只有一个顶层 `.osu`。
///
/// 数量按条目统计，损坏谱面也计入，避免把多难度包误当作单难度包。
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
    if bid.is_empty() && candidates.len() != 1 {
        return Err(PreviewError::new(format!(
            "--bid is required when --osz-file {} contains multiple top-level .osu beatmap files ({})",
            path.display(), candidates.len()
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
        // 唯一谱面仍须解析成功；没有有效 BeatmapID 时允许按文件名回退命名。
        if bid.is_empty() || (wanted.is_some() && id == wanted) {
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
/// 优先已通过一致性校验的 `bid`；省略时用谱面的 `BeatmapID`，无有效 ID 则回退
/// 到输入文件名主干（只保留文件名安全字符），避免不同本地文件共用产物名。
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
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);
        // 系统时钟的分辨率不足以保证并行测试目录唯一，额外使用进程内原子序号。
        let dir = std::env::temp_dir().join(format!(
            "osu-preview-local-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
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
    fn local_path_extensions_are_case_insensitive() {
        assert!(validate_local_path("map.OSU", "osu").is_ok());
        assert!(validate_local_path(r"C:\maps\MAP.OSZ", "osz").is_ok());
        assert!(validate_local_path("map.osk", "osu").is_err());
        assert!(validate_local_path("map.osz", "osu").is_err());
        assert!(validate_local_path("map.osu", "osz").is_err());
        assert!(validate_local_path("map", "osu").is_err());
        assert!(validate_local_path("", "osz").is_err());
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

    /// 单独的本地 `.osu` 不需要 bid，读取结果保留原始字节。
    #[test]
    fn local_osu_file_is_read_directly() {
        let source = osu_text(Some(300), "Extra");
        let path = write_temp("plain.osu", source.as_bytes());
        let loaded = load_local_source(Some(path.to_str().unwrap()), None, "").unwrap();
        assert_eq!(loaded.bytes, source.as_bytes());
        assert_eq!(loaded.path, path);
        assert!(loaded.osz_path.is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// ID 按数值比较，且超过 u64 的纯数字不能绕过一致性校验。
    #[test]
    fn local_osu_bid_must_match_metadata_numerically() {
        let path = write_temp("plain.osu", osu_text(Some(300), "Extra").as_bytes());
        let input = Some(path.to_str().unwrap());
        assert!(load_local_source(input, None, "300").is_ok());
        assert!(load_local_source(input, None, "000300").is_ok());
        for bid in ["301", "0", "18446744073709551616"] {
            let error = load_local_source(input, None, bid)
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains("does not match"), "{error}");
            assert!(error.contains("BeatmapID=300"), "{error}");
        }
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// 无有效 ID 的谱面可以单独预览，但不能结合 bid 或媒体包。
    #[test]
    fn local_osu_without_valid_id_rejects_bid_or_archive() {
        for id in [None, Some(0)] {
            let source = osu_text(id, "Local");
            assert_missing_id_rules(&source);
        }
        assert_missing_id_rules(
            &osu_text(Some(300), "Local").replace("BeatmapID:300", "BeatmapID:invalid"),
        );
    }

    fn assert_missing_id_rules(source: &str) {
        let path = write_temp("plain.osu", source.as_bytes());
        let input = Some(path.to_str().unwrap());
        assert!(load_local_source(input, None, "").is_ok());
        for (osz, bid) in [
            (None, "300"),
            (Some("pack.osz"), ""),
            (Some("pack.osz"), "300"),
        ] {
            let error = load_local_source(input, osz, bid)
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains("valid positive BeatmapID"), "{error}");
        }
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// 唯一顶层谱面可以自动选择，即使没有有效 ID；子目录难度不参与计数。
    #[test]
    fn single_osz_beatmap_can_be_selected_without_bid() {
        for id in [None, Some(100)] {
            let bytes = osz_bytes(&[
                ("Hard.OSU", osu_text(id, "Hard")),
                ("sub/Easy.osu", osu_text(Some(200), "Easy")),
            ]);
            let path = write_temp("fixture.osz", &bytes);
            let loaded = load_local_source(None, Some(path.to_str().unwrap()), "").unwrap();
            assert_eq!(parse(&loaded.bytes).metadata.get("Version"), Some("Hard"));
            assert_eq!(loaded.path, path);
            assert_eq!(loaded.osz_path.as_deref(), Some(path.as_path()));
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }

    /// 多谱面时不尝试猜测难度，损坏条目也必须计入数量。
    #[test]
    fn multiple_osz_beatmaps_require_bid() {
        let path = write_temp(
            "fixture.osz",
            &osz_bytes(&[
                ("Hard.osu", osu_text(Some(100), "Hard")),
                ("Broken.osu", "invalid beatmap".to_string()),
            ]),
        );
        let error = load_local_osz(&path, "").unwrap_err().to_string();
        assert!(error.contains("--bid is required"), "{error}");
        assert!(load_local_osz(&path, "100").is_ok());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// 自动选择不能跳过损坏谱面的解析失败，也不能选择没有顶层谱面的包。
    #[test]
    fn automatic_osz_selection_rejects_invalid_or_missing_beatmaps() {
        let path = write_temp(
            "fixture.osz",
            &osz_bytes(&[("Broken.osu", "invalid beatmap".to_string())]),
        );
        assert!(load_local_osz(&path, "")
            .unwrap_err()
            .to_string()
            .contains("failed to parse"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        let path = write_temp(
            "fixture.osz",
            &osz_bytes(&[("sub/Hard.osu", osu_text(Some(100), "Hard"))]),
        );
        assert!(load_local_osz(&path, "")
            .unwrap_err()
            .to_string()
            .contains("no .osu beatmap"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// 媒体包只校验 ID，不得覆盖外部编辑过的谱面文本。
    #[test]
    fn combined_sources_keep_external_osu_and_archive_media_path() {
        let external = osu_text(Some(100), "External");
        let osu_path = write_temp("external.osu", external.as_bytes());
        let osz_path = osu_path.parent().unwrap().join("media.osz");
        std::fs::write(
            &osz_path,
            osz_bytes(&[
                ("Easy.osu", osu_text(Some(200), "Easy")),
                ("Hard.osu", osu_text(Some(100), "Archive")),
            ]),
        )
        .unwrap();
        let osu = Some(osu_path.to_str().unwrap());
        let osz = Some(osz_path.to_str().unwrap());
        for bid in ["", "100", "000100"] {
            let loaded = load_local_source(osu, osz, bid).unwrap();
            assert_eq!(loaded.bytes, external.as_bytes());
            assert_eq!(loaded.path, osu_path);
            assert_eq!(loaded.osz_path.as_deref(), Some(osz_path.as_path()));
        }
        assert!(load_local_source(osu, osz, "200")
            .err()
            .unwrap()
            .to_string()
            .contains("does not match"));
        std::fs::write(
            &osz_path,
            osz_bytes(&[("Other.osu", osu_text(Some(200), "Other"))]),
        )
        .unwrap();
        for bid in ["", "100"] {
            let error = load_local_source(osu, osz, bid).err().unwrap().to_string();
            assert!(error.contains("beatmap 100 was not found"), "{error}");
        }
        std::fs::remove_dir_all(osu_path.parent().unwrap()).unwrap();
    }

    /// 谱面 ID：bid > BeatmapID > 文件名主干。
    #[test]
    fn effective_id_prefers_bid_then_beatmap_id_then_file_stem() {
        // 正斜杠在各平台都能分隔目录，避免把 Windows 路径误当作 Unix 文件名。
        let path = Path::new("maps/My Map [Hard].osu");
        let with_id = parse(osu_text(Some(300), "Hard").as_bytes());
        assert_eq!(effective_id("000300", &with_id, path), "000300");
        assert_eq!(effective_id("", &with_id, path), "300");

        let without_id = parse(osu_text(None, "Hard").as_bytes());
        assert_eq!(effective_id("", &without_id, path), "My-Map--Hard");
    }
}
