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

    /// 这个条目看起来是不是一个音频文件（用于筛选谱面自带打击音）。
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
/// - `samples`：谱面自带、且**不在内嵌打击音资源里**的候选样本名，供宿主去压缩包里
///   查找。候选名与内嵌资源的匹配规则见 [`crate::hitsound`]：解析器已经按
///   `{bank}-{name}` → `{name}` 的优先级收集，宿主只要把找到的条目填进样本库即可。
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

/// 收集「谱面自带且内嵌资源没有」的样本条目。
///
/// 大小写不同的同名候选只保留第一次出现的写法（压缩包匹配本来就不区分大小写）；
/// 输入来自 `referenced_names`，已排序，因此结果顺序稳定。
fn sample_entries(beatmap: &Beatmap) -> Vec<MediaEntry> {
    let mut seen = std::collections::BTreeSet::new();
    let mut entries = Vec::new();
    for name in crate::hitsound::referenced_names(beatmap) {
        if crate::hitsound::has_embedded_asset(&name) {
            continue;
        }
        let Some(entry) = MediaEntry::new(&name) else {
            continue;
        };
        if !entry.is_sample() || !seen.insert(entry.name.to_ascii_lowercase()) {
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

    #[test]
    fn 条目路径归一化与越界拒绝() {
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

    #[test]
    fn 扩展名统一小写并忽略非法字符() {
        assert_eq!(entry_extension("song.MP3").as_deref(), Some("mp3"));
        assert_eq!(entry_extension("a/b/Song.Ogg").as_deref(), Some("ogg"));
        assert_eq!(entry_extension("song.w a v"), Some("wav".to_string()));
        assert_eq!(entry_extension("song"), None);
        assert_eq!(entry_extension("song."), None);
        assert_eq!(entry_extension("song.  "), None);
    }

    #[test]
    fn 条目构造拒绝非法路径() {
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

    #[test]
    fn 音频背景与自带样本分别归类() {
        let beatmap = beatmap_with(
            vec![
                // 内嵌皮肤已有的音效：不进 samples。
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
        // `custom-hit` 与 `normal-custom-hit`（bank 前缀候选）都要收集，且顺序稳定。
        assert_eq!(names, vec!["Custom-Hit.OGG", "normal-Custom-Hit.OGG"]);
        assert!(!media.is_empty());
    }

    #[test]
    fn 缺少音频背景与样本时为空() {
        let mut beatmap = beatmap_with(Vec::new(), None);
        beatmap.general.insert("AudioFilename", String::new());
        let media = BeatmapMedia::from_beatmap(&beatmap);
        assert!(media.audio.is_none());
        assert!(media.background.is_none());
        assert!(media.samples.is_empty());
        assert!(media.is_empty());

        // 非法背景路径按「没有背景」处理，而不是让整张图加载失败。
        let beatmap = beatmap_with(Vec::new(), Some("../outside.jpg"));
        let media = BeatmapMedia::from_beatmap(&beatmap);
        assert!(media.background.is_none());
    }

    #[test]
    fn 非音频扩展名的候选不会当成样本文件() {
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
