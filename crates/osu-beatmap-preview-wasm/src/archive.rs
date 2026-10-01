//! 单文件输入（`.osu` / `.osz`）的解包与难度选择。
//!
//! WASM 的输入只有**一份文件**：`.osu` 字节或 `.osz`（ZIP）整包字节。`.osz` 里的
//! 音乐、背景、自带音效全部在这里解开，宿主不再看压缩包内部。判定规则与 CLI 的
//! `application/local.rs` 是同一张用例表（`test/local-file.test.js` 迁移而来）：
//!
//! - 只认压缩包**顶层**的 `.osu`：stable 会忽略子目录里的谱面，osu!lazer 同样按
//!   `!f.Filename.Contains('/')` 过滤；
//! - 多难度按 `.osu` 的 `[Metadata] BeatmapID` 匹配，匹配不到直接报错并列出候选，
//!   绝不随便挑一个（猜错难度比报错更糟）；
//! - 音乐条目（`AudioFilename`）必需、背景可选、自带音效按候选名匹配，限额防炸包。
//!
//! 这个模块不依赖 wasm 运行时，可以直接 `cargo test`（用 zip 写能力构造夹具）。

use std::io::{Cursor, Read};

use osu_beatmap_preview_core::processing::media::{
    normalize_entry_path, sample_entry_matches, BeatmapMedia,
};
use osu_beatmap_preview_core::Beatmap;

/// 单个 `.osu` 允许的最大字节数；正常谱面只有几 MiB，超过它一定不是谱面文本。
const MAX_BEATMAP_BYTES: u64 = 32 * 1024 * 1024;
/// 音乐条目的最大字节数；解码后的 PCM 还要常驻内存，超大文件直接拒绝。
const MAX_AUDIO_BYTES: u64 = 128 * 1024 * 1024;
/// 背景图片的最大字节数。
const MAX_BACKGROUND_BYTES: u64 = 32 * 1024 * 1024;
/// 背景视频的最大字节数：osu! 素材规范建议 ≤1280×720，超大文件直接放弃
/// （宿主回退纯色/背景图），避免 WASM 线性内存被单个条目吃光。
const MAX_VIDEO_BYTES: u64 = 128 * 1024 * 1024;
/// 单个自带音效的最大字节数；超过它的一定不是打击音。
const MAX_SAMPLE_BYTES: u64 = 8 * 1024 * 1024;
/// 自带音效的条目数上限。
const MAX_SAMPLE_ENTRIES: usize = 64;
/// 自带音效的合计字节上限。
const MAX_SAMPLE_TOTAL_BYTES: u64 = 32 * 1024 * 1024;

/// 难度选择方式。
#[derive(Debug, Clone, Default)]
pub enum DifficultySelector {
    /// 没有指定时取第一个顶层 `.osu`。
    #[default]
    First,
    /// 按 `[Metadata] BeatmapID` 匹配（BID 在线模式、本地填了 BID 都走这里）。
    ById(u64),
    /// 按压缩包内的条目名匹配（加载页难度下拉）。
    ByEntry(String),
}

/// 压缩包里的一个难度（加载页难度清单的行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DifficultyEntry {
    /// 压缩包内的条目名（如 `Hard.osu`）。
    pub entry: String,
    /// `[Metadata] BeatmapID`；缺失或非正数时为 `None`。
    pub beatmap_id: Option<u64>,
    /// 显示名 `Artist - Title [Version]`，字段缺失时退回条目名。
    pub label: String,
}

/// 一份输入文件解出的全部内容。
#[derive(Debug, Clone, Default)]
pub struct ArchiveContent {
    /// 选中难度的 `.osu` 字节。
    pub beatmap: Vec<u8>,
    /// 全部顶层难度（压缩包内顺序，稳定可复现）。
    pub difficulties: Vec<DifficultyEntry>,
    /// 音乐文件字节；单独的 `.osu` 输入没有音乐（`None`）。
    pub audio: Option<Vec<u8>>,
    /// 音乐条目名（扩展名用作 symphonia 的格式提示；`.osu` 输入为空）。
    pub audio_name: String,
    /// 背景图片字节（可选；谱面未声明或条目缺失时为 `None`）。
    pub background: Option<Vec<u8>>,
    /// 背景视频字节（可选；供宿主交给浏览器 `<video>` 解码播放）。
    pub video: Option<Vec<u8>>,
    /// 自带打击音条目（条目名 + 字节），供样本库按候选名匹配。
    pub samples: Vec<(String, Vec<u8>)>,
}

/// 解析一份输入文件（`.osu` 字节或 `.osz` 整包字节）。
///
/// `want_media = false` 时只解析难度结构（宿主取难度清单/谱面信息用），
/// 不解压音乐、背景与音效；`true` 时全部解出。
pub fn read_input(
    bytes: &[u8],
    selector: &DifficultySelector,
    want_media: bool,
) -> Result<ArchiveContent, String> {
    if is_zip(bytes) {
        read_archive(bytes, selector, want_media)
    } else {
        read_plain_osu(bytes)
    }
}

/// 单独的 `.osu` 字节：没有音乐、背景与自带音效，直接作为谱面。
fn read_plain_osu(bytes: &[u8]) -> Result<ArchiveContent, String> {
    if bytes.len() as u64 > MAX_BEATMAP_BYTES {
        return Err(".osu 文件过大，无法作为谱面解析".to_string());
    }
    let beatmap = parse_beatmap(bytes)?;
    let label = difficulty_label(&beatmap, "beatmap.osu");
    Ok(ArchiveContent {
        difficulties: vec![DifficultyEntry {
            entry: "beatmap.osu".to_string(),
            beatmap_id: beatmap_id(&beatmap),
            label,
        }],
        beatmap: bytes.to_vec(),
        ..ArchiveContent::default()
    })
}

fn read_archive(
    bytes: &[u8],
    selector: &DifficultySelector,
    want_media: bool,
) -> Result<ArchiveContent, String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| format!(".osz 打开失败：{error}"))?;

    // 顶层 `.osu` 清单：保持压缩包内顺序，同一压缩包的结果稳定可复现。
    let mut candidates: Vec<(usize, String)> = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!(".osz 条目读取失败：{error}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if is_beatmap_entry(&name) {
            candidates.push((index, name));
        }
    }
    if candidates.is_empty() {
        return Err(".osz 里找不到顶层 .osu 谱面文件".to_string());
    }

    // 逐个解析候选谱面（都很小），同时构建难度清单。
    let mut difficulties = Vec::new();
    let mut parsed: Vec<(usize, String, Vec<u8>, Beatmap)> = Vec::new();
    let mut first_parse_error = None;
    for (index, name) in candidates {
        let raw = read_entry(&mut archive, index, MAX_BEATMAP_BYTES)
            .map_err(|error| format!("读取 {name} 失败：{error}"))?;
        let beatmap = match parse_beatmap(&raw) {
            Ok(beatmap) => beatmap,
            Err(error) => {
                // 单个损坏的 .osu 不影响其他难度参与选择，只在全部失败时报出去。
                first_parse_error.get_or_insert(format!("{name}: {error}"));
                continue;
            }
        };
        difficulties.push(DifficultyEntry {
            entry: name.clone(),
            beatmap_id: beatmap_id(&beatmap),
            label: difficulty_label(&beatmap, &name),
        });
        parsed.push((index, name, raw, beatmap));
    }
    if parsed.is_empty() {
        return Err(format!(
            ".osz 里的 .osu 全部解析失败：{}",
            first_parse_error.unwrap_or_else(|| "未知错误".to_string())
        ));
    }

    let selected = select_difficulty(&parsed, selector)?;
    let selected_beatmap = &parsed[selected].3;
    let media = BeatmapMedia::from_beatmap(selected_beatmap);

    let mut content = ArchiveContent {
        beatmap: parsed[selected].2.clone(),
        difficulties,
        ..ArchiveContent::default()
    };
    if !want_media {
        return Ok(content);
    }

    // 音乐必需：没有声音的整包预览没有意义，找不到直接报错（与 CLI/旧后端一致）。
    let audio_entry = media
        .audio
        .as_ref()
        .ok_or_else(|| ".osz 中找不到音乐：谱面未指定 AudioFilename".to_string())?;
    let audio_index = find_entry(&mut archive, &audio_entry.name)
        .ok_or_else(|| format!(".osz 中找不到音乐：{}", audio_entry.name))?;
    content.audio_name = audio_entry.name.clone();
    content.audio = Some(
        read_entry(&mut archive, audio_index, MAX_AUDIO_BYTES)
            .map_err(|error| format!("读取音乐失败：{error}"))?,
    );

    // 背景可选：缺失或读取失败只退化成纯色背景。
    if let Some(background) = media.background.as_ref() {
        if let Some(index) = find_entry(&mut archive, &background.name) {
            content.background = read_entry(&mut archive, index, MAX_BACKGROUND_BYTES).ok();
        }
    }

    // 背景视频可选：缺失或读取失败只退化成静态背景（与 osu! 的降级一致）。
    if let Some(video) = media.video.as_ref() {
        if let Some(index) = find_entry(&mut archive, &video.name) {
            content.video = read_entry(&mut archive, index, MAX_VIDEO_BYTES).ok();
        }
    }

    // 自带音效：按候选名匹配同名条目（`sample_entry_matches`），限额防炸包；
    // 单个条目损坏只影响这一个音效（解码阶段按静音处理）。
    let audio_name = audio_entry.name.to_ascii_lowercase();
    let mut total = 0_u64;
    for candidate in &media.samples {
        if content.samples.len() >= MAX_SAMPLE_ENTRIES || total >= MAX_SAMPLE_TOTAL_BYTES {
            break;
        }
        for index in 0..archive.len() {
            // 先只取名字与大小（`ZipFile` 借用在此结束），再决定要不要读字节。
            let (name, size) = match archive.by_index(index) {
                Ok(entry) if !entry.is_dir() => (entry.name().to_string(), entry.size()),
                Ok(_) => continue,
                Err(error) => return Err(format!(".osz 条目读取失败：{error}")),
            };
            if name.to_ascii_lowercase() == audio_name {
                continue;
            }
            if size > MAX_SAMPLE_BYTES || !sample_entry_matches(&name, &candidate.name) {
                continue;
            }
            if content.samples.iter().any(|(seen, _)| seen == &name) {
                continue;
            }
            let raw = match read_entry(&mut archive, index, MAX_SAMPLE_BYTES) {
                Ok(raw) => raw,
                Err(_) => continue,
            };
            total += raw.len() as u64;
            content.samples.push((name, raw));
            break;
        }
    }
    Ok(content)
}

/// 从候选里选出要预览的难度。
fn select_difficulty(
    parsed: &[(usize, String, Vec<u8>, Beatmap)],
    selector: &DifficultySelector,
) -> Result<usize, String> {
    match selector {
        DifficultySelector::First => Ok(0),
        DifficultySelector::ById(wanted) => parsed
            .iter()
            .position(|(_, _, _, beatmap)| beatmap_id(beatmap) == Some(*wanted))
            .ok_or_else(|| {
                let seen = parsed
                    .iter()
                    .map(|(_, name, _, beatmap)| {
                        format!(
                            "{name} (BeatmapID={})",
                            beatmap_id(beatmap)
                                .map(|id| id.to_string())
                                .unwrap_or_else(|| "none".to_string())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("找不到 bid 为 {wanted} 的 .osu（压缩包内 BeatmapID：{seen}）")
            }),
        DifficultySelector::ByEntry(wanted) => {
            let wanted = normalize_entry_path(wanted).unwrap_or_default();
            parsed
                .iter()
                .position(|(_, name, _, _)| name == &wanted)
                .ok_or_else(|| format!("压缩包里没有难度条目：{wanted}"))
        }
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

/// 字节是否是 ZIP（`.osz`）；否则按 `.osu` 文本处理。
fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") || bytes.starts_with(b"PK\x07\x08")
}

fn parse_beatmap(bytes: &[u8]) -> Result<Beatmap, String> {
    osu_beatmap_preview_core::parse_beatmap_bytes(bytes)
        .map_err(|error| format!("谱面解析失败：{error}"))
}

/// 谱面的 `[Metadata] BeatmapID`；缺失或非正数时返回 `None`。
fn beatmap_id(beatmap: &Beatmap) -> Option<u64> {
    beatmap
        .metadata
        .get("BeatmapID")
        .and_then(|value| value.trim().parse().ok())
        .filter(|id| *id > 0)
}

/// 难度显示名 `Artist - Title [Version]`，字段缺失时退回条目名。
fn difficulty_label(beatmap: &Beatmap, entry_name: &str) -> String {
    let artist = beatmap.metadata.get("Artist").map(str::trim).unwrap_or("");
    let title = beatmap.metadata.get("Title").map(str::trim).unwrap_or("");
    let version = beatmap.metadata.get("Version").map(str::trim).unwrap_or("");
    if title.is_empty() && version.is_empty() {
        return entry_name.to_string();
    }
    let artist = if artist.is_empty() {
        String::new()
    } else {
        format!("{artist} - ")
    };
    format!("{artist}{title} [{version}]")
}

/// 按归一化后的条目名定位压缩包条目；找不到返回 `None`。
fn find_entry<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    wanted: &str,
) -> Option<usize> {
    let wanted = normalize_entry_path(wanted)?;
    (0..archive.len()).find(|&index| {
        archive
            .by_index(index)
            .ok()
            .and_then(|entry| normalize_entry_path(entry.name()))
            .is_some_and(|name| name == wanted)
    })
}

/// 读出一个压缩包条目的全部字节；空条目或超限条目报错。
fn read_entry<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    index: usize,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    let mut entry = archive
        .by_index(index)
        .map_err(|error| format!("条目打开失败：{error}"))?;
    if entry.is_dir() {
        return Err("条目是目录".to_string());
    }
    if entry.size() > max_bytes {
        return Err(format!("条目过大：{} 字节", entry.size()));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .by_ref()
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("条目读取失败：{error}"))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!("条目超过上限：{max_bytes} 字节"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    /// 用给定条目构造一个内存 .osz。
    fn osz_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            for (name, content) in entries {
                writer
                    .start_file(*name, SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(content).unwrap();
            }
            writer.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn osu_text(beatmap_id: Option<u32>, version: &str, audio: &str, background: &str) -> String {
        let id = beatmap_id
            .map(|value| format!("BeatmapID:{value}\n"))
            .unwrap_or_default();
        let background = if background.is_empty() {
            String::new()
        } else {
            format!("0,0,\"{background}\",0,0\n")
        };
        format!(
            "osu file format v14\n\n[General]\nAudioFilename: {audio}\nMode: 0\n\n\
             [Metadata]\nTitle:Test\nArtist:Someone\nCreator:Me\nVersion:{version}\n{id}\n\
             [Events]\n{background}\n\
             [Difficulty]\nCircleSize:4\nSliderMultiplier:1.4\nSliderTickRate:1\n\n\
             [TimingPoints]\n0,500,4,1,0,100,1,0\n\n\
             [HitObjects]\n256,192,1000,1,2,0:0:0:60:custom-hit.wav\n256,192,2000,1,0,0:0:0:0:\n"
        )
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

    /// 难度选择：默认第一个、按 bid、按条目名；匹配不到报错并列出候选。
    #[test]
    fn difficulty_selection_covers_default_bid_and_entry() {
        let bytes = osz_bytes(&[
            (
                "Easy.osu",
                osu_text(Some(100), "Easy", "audio.mp3", "").as_bytes(),
            ),
            (
                "Hard.osu",
                osu_text(Some(200), "Hard", "audio.mp3", "").as_bytes(),
            ),
        ]);

        let first = read_input(&bytes, &DifficultySelector::First, false).unwrap();
        assert_eq!(first.difficulties.len(), 2);
        assert_eq!(first.difficulties[1].label, "Someone - Test [Hard]");
        assert!(parse_beatmap(&first.beatmap).unwrap().metadata.get("Version") == Some("Easy"));

        let by_id = read_input(&bytes, &DifficultySelector::ById(200), false).unwrap();
        assert!(parse_beatmap(&by_id.beatmap).unwrap().metadata.get("Version") == Some("Hard"));

        let by_entry = read_input(
            &bytes,
            &DifficultySelector::ByEntry("Hard.osu".to_string()),
            false,
        )
        .unwrap();
        assert!(parse_beatmap(&by_entry.beatmap).unwrap().metadata.get("Version") == Some("Hard"));

        let error = read_input(&bytes, &DifficultySelector::ById(999), false).unwrap_err();
        assert!(error.contains("找不到 bid 为 999"), "{error}");
        assert!(error.contains("BeatmapID=100"), "{error}");
    }

    /// `.osz` 里没有顶层 `.osu` 时拒绝；单独的 `.osu` 直接可用且没有媒体。
    #[test]
    fn osz_without_top_level_osu_is_rejected_and_plain_osu_works() {
        let bad = osz_bytes(&[(
            "sub/Hard.osu",
            osu_text(Some(200), "Hard", "audio.mp3", "").as_bytes(),
        )]);
        let error = read_input(&bad, &DifficultySelector::First, true).unwrap_err();
        assert!(error.contains("找不到顶层 .osu"), "{error}");

        let plain = osu_text(Some(300), "Extra", "audio.mp3", "bg.jpg").into_bytes();
        let content = read_input(&plain, &DifficultySelector::First, true).unwrap();
        assert!(content.audio.is_none(), ".osu 单文件没有音乐");
        assert!(content.background.is_none());
        assert!(content.samples.is_empty());
        assert_eq!(content.difficulties.len(), 1);
    }

    /// 媒体解包：音乐必需、背景可选、自带音效按候选名匹配。
    #[test]
    fn media_extraction_follows_osu_rules() {
        let bytes = osz_bytes(&[
            (
                "Hard.osu",
                osu_text(Some(200), "Hard", "audio.mp3", "bg.jpg").as_bytes(),
            ),
            ("audio.mp3", b"fake-mp3"),
            ("bg.jpg", b"fake-jpeg"),
            ("custom-hit.wav", b"fake-wav"),
            ("normal-hitnormal.ogg", b"fake-ogg"),
            ("unrelated.ogg", b"fake"),
        ]);

        let content = read_input(&bytes, &DifficultySelector::ById(200), true).unwrap();
        assert_eq!(content.audio.as_deref(), Some(&b"fake-mp3"[..]));
        assert_eq!(content.background.as_deref(), Some(&b"fake-jpeg"[..]));

        // 只有候选名命中的条目被取出：`custom-hit.wav` 来自第一个物件的 hitSample
        // 自定义文件名（它覆盖普通层），`normal-hitnormal.ogg` 是第二个物件普通层的
        // 同名文件；无关条目（`unrelated.ogg`）不带。
        let names: Vec<&str> = content
            .samples
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert!(names.contains(&"custom-hit.wav"), "{names:?}");
        assert!(names.contains(&"normal-hitnormal.ogg"), "{names:?}");
        assert!(!names.contains(&"unrelated.ogg"), "{names:?}");
    }

    /// 音乐缺失直接报错；背景缺失只退化成纯色。
    #[test]
    fn missing_audio_is_fatal_and_missing_background_is_not() {
        let bytes = osz_bytes(&[(
            "Hard.osu",
            osu_text(Some(200), "Hard", "missing.mp3", "gone.jpg").as_bytes(),
        )]);
        let error = read_input(&bytes, &DifficultySelector::First, true).unwrap_err();
        assert!(error.contains("找不到音乐"), "{error}");

        let bytes = osz_bytes(&[
            (
                "Hard.osu",
                osu_text(Some(200), "Hard", "audio.mp3", "gone.jpg").as_bytes(),
            ),
            ("audio.mp3", b"fake-mp3"),
        ]);
        let content = read_input(&bytes, &DifficultySelector::First, true).unwrap();
        assert!(content.background.is_none());
    }

    /// `want_media = false` 只解析难度结构，不解压媒体。
    #[test]
    fn structure_only_mode_skips_media() {
        let bytes = osz_bytes(&[
            (
                "Hard.osu",
                osu_text(Some(200), "Hard", "audio.mp3", "bg.jpg").as_bytes(),
            ),
            ("audio.mp3", b"fake-mp3"),
        ]);
        let content = read_input(&bytes, &DifficultySelector::First, false).unwrap();
        assert!(content.audio.is_none());
        assert!(content.samples.is_empty());
        assert_eq!(content.difficulties.len(), 1);
    }
}
