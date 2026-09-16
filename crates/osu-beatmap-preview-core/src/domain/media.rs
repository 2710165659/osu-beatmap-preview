//! 媒体（压缩包）条目策略。
//!
//! 这里只做「谱面字段 + 纯字符串」的换算：宿主要去压缩包里找哪些条目、条目名怎么
//! 归一化、扩展名怎么取，全部由这里决定；不读文件、不认识 ZIP 实现，因此 CLI、
//! Web 后端（`backend/zip.js` 保持同一套规则，由契约测试钉住）与将来的 GUI/移动端
//! 共用同一份策略，不会各自走偏。
//!
//! 典型用法：宿主解析出 [`crate::Beatmap`] 后调用 [`BeatmapMedia::from_beatmap`]，
//! 得到 `audio` / `background` / `samples` 三类条目，再按自己的方式（zip crate、
//! Node 的 zlib、浏览器原生解压）把条目取出来。

use super::models::Beatmap;

/// 谱面自带打击音允许的扩展名（osu! 常见的三种）。
pub const SAMPLE_EXTENSIONS: [&str; 3] = ["ogg", "wav", "mp3"];

/// 归一化压缩包内的条目路径。
///
/// 规则与 `backend/zip.js` 的 `normalizeArchivePath` 一致：反斜杠转正斜杠、去掉空段
/// 与 `.`，并拒绝绝对路径、`..` 与含 `:` 的段（盘符/协议前缀），避免解包时越界写入。
pub fn normalize_entry_path(path: &str) -> Option<String> {
    let replaced = path.trim().replace('\\', "/");
    if replaced.starts_with('/') {
        return None;
    }
    let mut segments = Vec::new();
    for segment in replaced.split('/') {
        match segment {
            "" | "." => continue,
            ".." => return None,
            value if value.contains(':') => return None,
            value => segments.push(value),
        }
    }
    if segments.is_empty() {
        return None;
    }
    Some(segments.join("/"))
}

/// 条目扩展名：小写、只保留 ASCII 字母数字。
///
/// 没有扩展名（或扩展名里没有可用字符）时返回 `None`，由宿主决定兜底名
/// （CLI 的媒体缓存用 `audio`，Web 用 `bin`），这样缓存文件名不会被这里改变。
pub fn entry_extension(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (_, extension) = name.rsplit_once('.')?;
    let extension: String = extension
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    if extension.is_empty() {
        None
    } else {
        Some(extension)
    }
}

/// 压缩包里的一个媒体条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaEntry {
    /// 归一化后的条目名（`/` 分隔、已去掉空段与 `.`）。
    pub name: String,
    /// 小写扩展名；没有可用扩展名时为 `None`。
    pub extension: Option<String>,
}

impl MediaEntry {
    /// 由 `.osu` 里声明的文件名构造条目；路径非法或为空时返回 `None`。
    pub fn new(name: &str) -> Option<Self> {
        let name = normalize_entry_path(name)?;
        let extension = entry_extension(&name);
        Some(Self { name, extension })
    }

    /// 这个条目是否带音频扩展名（ogg / wav / mp3）。
    ///
    /// 注意：谱面自带的候选样本名也可能是**不带扩展名**的（`soft-hitnormal`，实际文件是
    /// `soft-hitnormal.ogg` 等），所以筛选候选条目时不能只看这个判断，见 [`sample_entries`]。
    pub fn is_sample(&self) -> bool {
        self.extension
            .as_deref()
            .is_some_and(|extension| SAMPLE_EXTENSIONS.contains(&extension))
    }
}

/// 谱面在媒体压缩包里需要的条目。
///
/// - `audio`：`[General] AudioFilename`，必需（预览/视频都要靠它出声）；
/// - `background`：`[Events]` 的第一张背景图，可选（缺失时宿主退化成纯色背景）；
/// - `samples`：谱面可能自带的候选打击音样本名，供宿主去压缩包里按
///   [`sample_entry_matches`] 查找同名条目。找到的条目由宿主解码后填进样本库，其优先级
///   高于内嵌皮肤（见 [`crate::hitsound::has_embedded_asset`]）：谱面自带音效是谱面
///   自定义的一部分，内嵌资源只是它缺失时的兜底。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BeatmapMedia {
    pub audio: Option<MediaEntry>,
    pub background: Option<MediaEntry>,
    pub samples: Vec<MediaEntry>,
}

impl BeatmapMedia {
    /// 按谱面字段算出需要的条目。
    pub fn from_beatmap(beatmap: &Beatmap) -> Self {
        Self {
            audio: beatmap.audio_filename().and_then(MediaEntry::new),
            background: beatmap
                .background_filename
                .as_deref()
                .and_then(MediaEntry::new),
            samples: sample_entries(beatmap),
        }
    }

    /// 三类条目都为空时返回 `true`。
    pub fn is_empty(&self) -> bool {
        self.audio.is_none() && self.background.is_none() && self.samples.is_empty()
    }
}

/// 压缩包条目是否就是某个候选样本名对应的文件。
///
/// 候选名有两种写法：不带扩展名的样本名（`soft-hitnormal`，磁盘上对应
/// `soft-hitnormal.ogg` / `.wav` / `.mp3`）与 `hitSample` 里的自定义文件名
/// （`custom-hit.ogg`，可能带子目录）。因此依次比较「完整条目名」「文件名」「去掉扩展名
/// 的文件名」，全部不区分大小写；条目名或候选名非法（越界路径）时返回 `false`。
///
/// `src/hitsound.js` 里有同一套规则的 JS 版本，由
/// `test/hitsound-samples.test.js` 的用例表钉住。
pub fn sample_entry_matches(entry_name: &str, candidate: &str) -> bool {
    let Some(entry) = normalize_entry_path(entry_name) else {
        return false;
    };
    let Some(candidate) = normalize_entry_path(candidate) else {
        return false;
    };
    if entry.eq_ignore_ascii_case(&candidate) {
        return true;
    }
    let file_name = entry.rsplit('/').next().unwrap_or_default();
    if file_name.eq_ignore_ascii_case(&candidate) {
        return true;
    }
    match file_name.rsplit_once('.') {
        Some((stem, _)) => stem.eq_ignore_ascii_case(&candidate),
        None => false,
    }
}

/// 收集谱面可能自带的打击音样本条目。
///
/// 候选名来自 [`crate::hitsound::referenced_names`]，包含两类：`{bank}-{name}` 这类不带
/// 扩展名的样本名，以及 `hitSample` 里的自定义文件名（自带扩展名）。前者不能按扩展名
/// 过滤，否则 `soft-hitnormal` 这些最常见的候选会被整批丢掉；后者必须是音频扩展名，
/// 免得把 `.osu` 里写错的非音频文件名也拿去解包。
///
/// 大小写不同的同名候选只保留第一次出现的写法（压缩包匹配本来就不区分大小写）；
/// 输入已排序，因此结果顺序稳定。
fn sample_entries(beatmap: &Beatmap) -> Vec<MediaEntry> {
    let mut seen = std::collections::BTreeSet::new();
    let mut entries = Vec::new();
    for name in crate::hitsound::referenced_names(beatmap) {
        let Some(entry) = MediaEntry::new(&name) else {
            continue;
        };
        if entry
            .extension
            .as_deref()
            .is_some_and(|extension| !SAMPLE_EXTENSIONS.contains(&extension))
        {
            continue;
        }
        if !seen.insert(entry.name.to_ascii_lowercase()) {
            continue;
        }
        entries.push(entry);
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{
        Beatmap, HitAddition, HitObjects, HitSample, KvSection, SampleBank, StandardHitObject,
        TimingPoint,
    };

    fn normalize(value: &str) -> Option<String> {
        normalize_entry_path(value)
    }

    /// 条目路径归一化，并拒绝越界路径。
    #[test]
    fn entry_paths_are_normalized_and_traversal_rejected() {
        // 与 web 后端 zip.js::normalizeArchivePath 用同一张用例表（契约测试会再跑一遍）。
        assert_eq!(
            normalize(r"audio\\song.mp3").as_deref(),
            Some("audio/song.mp3")
        );
        assert_eq!(normalize("a/./b//c").as_deref(), Some("a/b/c"));
        assert_eq!(normalize(" bg.jpg ").as_deref(), Some("bg.jpg"));
        assert_eq!(
            normalize("./hitnormal.ogg").as_deref(),
            Some("hitnormal.ogg")
        );
        assert_eq!(normalize("../song.mp3"), None);
        assert_eq!(normalize("a/../../b"), None);
        assert_eq!(normalize("C:/song.mp3"), None);
        assert_eq!(normalize("/song.mp3"), None);
        assert_eq!(normalize("a/b:c"), None);
        assert_eq!(normalize("   "), None);
        assert_eq!(normalize("."), None);
    }

    /// 扩展名统一小写并忽略非法字符。
    #[test]
    fn extensions_lowercased_and_invalid_characters_ignored() {
        assert_eq!(entry_extension("song.MP3").as_deref(), Some("mp3"));
        assert_eq!(entry_extension("a/b/Song.Ogg").as_deref(), Some("ogg"));
        assert_eq!(entry_extension("song.w a v"), Some("wav".to_string()));
        assert_eq!(entry_extension("song"), None);
        assert_eq!(entry_extension("song."), None);
        assert_eq!(entry_extension("song.  "), None);
    }

    /// 条目构造拒绝非法路径。
    #[test]
    fn entry_construction_rejects_invalid_paths() {
        assert_eq!(
            MediaEntry::new("audio/song.ogg"),
            Some(MediaEntry {
                name: "audio/song.ogg".to_string(),
                extension: Some("ogg".to_string()),
            })
        );
        assert!(MediaEntry::new("../song.ogg").is_none());
        assert!(MediaEntry::new("").is_none());
    }

    /// 构造一个「有音频、有背景、带自定义打击音」的谱面。
    fn beatmap_with(samples: Vec<HitSample>, background: Option<&str>) -> Beatmap {
        let mut general = KvSection::default();
        general.insert("Mode", "0".to_string());
        general.insert("AudioFilename", r"audio\song.mp3".to_string());
        Beatmap {
            metadata: KvSection::default(),
            difficulty: KvSection::default(),
            general,
            timing_points: vec![TimingPoint {
                time: 0.0,
                beat_length: 500.0,
                meter: 4,
                uninherited: true,
                kiai_mode: false,
                omit_first_bar_line: false,
                sample_set: 1,
                sample_index: 0,
                sample_volume: 100,
            }],
            hit_objects: HitObjects::Standard(vec![StandardHitObject {
                start_time: 1000,
                end_time: 1000,
                hit_type: 1,
                hitsound: 0,
                samples,
                ..Default::default()
            }]),
            break_periods: Vec::new(),
            background_filename: background.map(str::to_string),
            combo_colors: Vec::new(),
            beat_divisor: 0,
        }
    }

    /// 音频、背景与自带样本分别归类。
    #[test]
    fn audio_background_and_custom_samples_are_classified() {
        let beatmap = beatmap_with(
            vec![
                // 内嵌皮肤已有的音效同样要列出来：谱面自带同名文件时它优先。
                HitSample::new(SampleBank::Normal, HitAddition::None, 100, None),
                // 自定义文件名：要宿主去压缩包里找。
                HitSample::new(
                    SampleBank::Normal,
                    HitAddition::None,
                    100,
                    Some("Custom-Hit.OGG".to_string()),
                ),
            ],
            Some("bg.jpg"),
        );
        let media = BeatmapMedia::from_beatmap(&beatmap);
        // 声明里的反斜杠会归一化成 `/`。
        assert_eq!(
            media.audio.as_ref().map(|entry| entry.name.as_str()),
            Some("audio/song.mp3")
        );
        assert_eq!(
            media.background.as_ref().map(|entry| entry.name.as_str()),
            Some("bg.jpg")
        );
        let names: Vec<&str> = media
            .samples
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        // 自定义文件名（含 bank 前缀候选）与不带扩展名的样本名都要收集，且顺序稳定。
        assert_eq!(
            names,
            vec![
                "Custom-Hit.OGG",
                "hitnormal",
                "normal-Custom-Hit.OGG",
                "normal-hitnormal",
            ]
        );
        assert!(!media.is_empty());
    }

    /// 样本条目按候选名匹配。
    #[test]
    fn sample_entries_match_candidate_names() {
        // 不带扩展名的样本名对应压缩包里的音频文件。
        assert!(sample_entry_matches("soft-hitnormal.ogg", "soft-hitnormal"));
        assert!(sample_entry_matches("Soft-Hitnormal.WAV", "soft-hitnormal"));
        assert!(sample_entry_matches("hitnormal.mp3", "hitnormal"));
        // `hitSample` 的自定义文件名可以带子目录。
        assert!(sample_entry_matches("sub/custom-hit.ogg", "custom-hit.ogg"));
        assert!(sample_entry_matches("Custom-Hit.OGG", "custom-hit.ogg"));
        // bank 前缀是候选名的一部分，不能只按文件名后缀命中。
        assert!(!sample_entry_matches(
            "custom-hit.ogg",
            "normal-custom-hit.ogg"
        ));
        assert!(!sample_entry_matches("hitnormal.ogg", "soft-hitnormal"));
        assert!(!sample_entry_matches("spinnerbonus.ogg", "spinnerbonus-max"));
        // 越界路径一律不匹配（避免拿 `..` 去压缩包里翻文件）。
        assert!(!sample_entry_matches("../soft-hitnormal.ogg", "soft-hitnormal"));
        assert!(!sample_entry_matches("soft-hitnormal.ogg", "../soft-hitnormal"));
    }

    /// 缺少音频与背景时留空，但保留样本候选。
    #[test]
    fn missing_audio_and_background_keep_sample_candidates() {
        let mut beatmap = beatmap_with(Vec::new(), None);
        beatmap.general.insert("AudioFilename", String::new());
        let media = BeatmapMedia::from_beatmap(&beatmap);
        assert!(media.audio.is_none());
        assert!(media.background.is_none());
        // 物件没有自带 hitSample 时音效参数来自 timing point，谱面仍可能自带这些同名文件，
        // 因此候选必须照常列出（否则「谱面自带 soft-hitnormal」这类覆盖会失效）。
        assert_eq!(
            media
                .samples
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["hitnormal", "normal-hitnormal"]
        );
        assert!(!media.is_empty());

        // 非法背景路径按「没有背景」处理，而不是让整张图加载失败。
        let beatmap = beatmap_with(Vec::new(), Some("../outside.jpg"));
        let media = BeatmapMedia::from_beatmap(&beatmap);
        assert!(media.background.is_none());
    }

    /// 非音频扩展名的候选不会当成样本文件。
    #[test]
    fn non_audio_extensions_are_not_samples() {
        let beatmap = beatmap_with(
            vec![HitSample::new(
                SampleBank::Normal,
                HitAddition::None,
                100,
                Some("custom-hit.ogg.bak".to_string()),
            )],
            None,
        );
        let media = BeatmapMedia::from_beatmap(&beatmap);
        assert!(media.samples.is_empty(), "samples={:?}", media.samples);
    }
}
