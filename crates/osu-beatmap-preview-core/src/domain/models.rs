use std::collections::BTreeMap;

/// 默认音效组（`.osu` 里自定义音效组的缺省值）。
pub const SAMPLE_SET_NORMAL: i32 = 1;
/// soft 音效组。
pub const SAMPLE_SET_SOFT: i32 = 2;
/// drum 音效组。
pub const SAMPLE_SET_DRUM: i32 = 3;

/// 音效组。`Custom` 对应 `.osu` 中通过文件名指定的自定义音效。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SampleBank {
    /// 未在物件中指定音效组，应用所在 timing point 的音效组。
    Auto,
    #[default]
    Normal,
    Soft,
    Drum,
    Custom,
}

impl SampleBank {
    pub fn from_set_id(set_id: i32) -> Self {
        match set_id {
            SAMPLE_SET_SOFT => SampleBank::Soft,
            SAMPLE_SET_DRUM => SampleBank::Drum,
            SAMPLE_SET_NORMAL => SampleBank::Normal,
            // 0 与非法值都按默认音效组处理，和 osu! stable 的解析一致。
            _ => SampleBank::Normal,
        }
    }

    /// 返回文件名中的音效组前缀，自定义音效不参与前缀拼接。
    pub fn prefix(self) -> Option<&'static str> {
        match self {
            SampleBank::Auto => None,
            SampleBank::Normal => Some("normal"),
            SampleBank::Soft => Some("soft"),
            SampleBank::Drum => Some("drum"),
            SampleBank::Custom => None,
        }
    }
}

/// 打击音加成类型（`.osu` 打击音位掩码中的 1/2/4 位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HitAddition {
    #[default]
    None,
    Whistle,
    Finish,
    Clap,
}

impl HitAddition {
    pub fn from_hitsound(hitsound: i32) -> Self {
        if hitsound & 2 != 0 {
            HitAddition::Whistle
        } else if hitsound & 4 != 0 {
            HitAddition::Finish
        } else if hitsound & 8 != 0 {
            HitAddition::Clap
        } else {
            HitAddition::None
        }
    }

    /// 返回 hitsound 位掩码中全部置位的加成音。
    pub fn all_from_hitsound(hitsound: i32) -> impl Iterator<Item = Self> {
        [
            (2, HitAddition::Whistle),
            (4, HitAddition::Finish),
            (8, HitAddition::Clap),
        ]
        .into_iter()
        .filter(move |(bit, _)| hitsound & bit != 0)
        .map(|(_, addition)| addition)
    }

    /// 返回文件名中的加成后缀；普通打击音没有后缀。
    pub fn suffix(self) -> &'static str {
        match self {
            HitAddition::None => "hitnormal",
            HitAddition::Whistle => "hitwhistle",
            HitAddition::Finish => "hitfinish",
            HitAddition::Clap => "hitclap",
        }
    }
}

/// 一个待播放的打击音。
///
/// `volume` 为 0～100 的谱面音量；`filename` 非空表示谱面自带的音效文件，
/// 此时忽略音效组（与 osu! 的 `HitSampleInfo` 语义一致）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HitSample {
    pub bank: SampleBank,
    pub addition: HitAddition,
    pub volume: i32,
    pub filename: Option<String>,
}

impl HitSample {
    pub fn new(bank: SampleBank, addition: HitAddition, volume: i32, filename: Option<String>) -> Self {
        Self {
            bank,
            addition,
            volume,
            filename,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimingPoint {
    pub time: f64,
    pub beat_length: f64,
    pub meter: i32,
    pub uninherited: bool,
    pub kiai_mode: bool,
    /// effects 位 3：省略该红线区段的第一条小节线。
    pub omit_first_bar_line: bool,
    /// `[TimingPoints]` 第 4 列的音效组；0 表示沿用谱面默认值。
    pub sample_set: i32,
    /// `[TimingPoints]` 第 5 列的自定义音效索引（预览不使用具体文件，仅保留原值）。
    pub sample_index: i32,
    /// `[TimingPoints]` 第 6 列的音效音量，0 表示静音。
    pub sample_volume: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct BreakPeriod {
    pub start_time: i64,
    pub end_time: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StandardHitObject {
    pub x: i32,
    pub y: i32,
    pub start_time: i64,
    pub end_time: i64,
    pub hit_type: i32,
    pub hitsound: i32,
    pub new_combo: bool,
    pub combo_offset: i32,
    pub slider_type: Option<String>,
    pub slider_points: Vec<(i32, i32)>,
    pub slider_repeats: i32,
    pub slider_pixel_length: f64,
    pub slider_edge_hitsounds: Vec<i32>,
    pub stack_height: i32,
    /// 物件头部的打击音；空表示使用谱面默认音效组。
    pub samples: Vec<HitSample>,
    /// 滑条重复/尾部节点各自的打击音，按 edgeSets 顺序排列（不含滑条头）。
    pub slider_edge_samples: Vec<Vec<HitSample>>,
}

impl Default for StandardHitObject {
    fn default() -> Self {
        StandardHitObject {
            x: 0,
            y: 0,
            start_time: 0,
            end_time: 0,
            hit_type: 0,
            hitsound: 0,
            new_combo: false,
            combo_offset: 0,
            slider_type: None,
            slider_points: Vec::new(),
            slider_repeats: 1,
            slider_pixel_length: 0.0,
            slider_edge_hitsounds: Vec::new(),
            stack_height: 0,
            samples: Vec::new(),
            slider_edge_samples: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TaikoHitObject {
    pub start_time: i64,
    pub end_time: i64,
    pub hit_type: i32,
    pub hitsound: i32,
    /// 物件头部的打击音；空表示使用谱面默认音效组。
    pub samples: Vec<HitSample>,
}

#[derive(Debug, Clone, Default)]
pub struct CatchHitObject {
    pub x: i32,
    pub y: i32,
    pub start_time: i64,
    pub end_time: i64,
    pub hit_type: i32,
    pub hitsound: i32,
    pub new_combo: bool,
    pub combo_offset: i32,
    pub slider_type: Option<String>,
    pub slider_points: Vec<(i32, i32)>,
    pub slider_repeats: i32,
    pub slider_pixel_length: f64,
    /// 物件头部的打击音；空表示使用谱面默认音效组。
    pub samples: Vec<HitSample>,
}

#[derive(Clone, Default)]
pub struct ManiaHitObject {
    pub lane: i32,
    pub start_time: i64,
    pub end_time: i64,
    pub is_long_note: bool,
    pub samples: Vec<HitSample>,
}

impl std::fmt::Debug for ManiaHitObject {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 保持既有转谱 golden 输出稳定；样本只供音频时间轴使用，不改变画面快照。
        formatter
            .debug_struct("ManiaHitObject")
            .field("lane", &self.lane)
            .field("start_time", &self.start_time)
            .field("end_time", &self.end_time)
            .field("is_long_note", &self.is_long_note)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub enum HitObjects {
    Standard(Vec<StandardHitObject>),
    Taiko(Vec<TaikoHitObject>),
    Catch(Vec<CatchHitObject>),
    Mania(Vec<ManiaHitObject>),
}

/// 对四种 `HitObjects` 变体统一应用表达式。
macro_rules! for_each_hit_variant {
    ($self:expr, |$v:ident| $body:expr) => {
        match $self {
            HitObjects::Standard($v) => $body,
            HitObjects::Taiko($v) => $body,
            HitObjects::Catch($v) => $body,
            HitObjects::Mania($v) => $body,
        }
    };
}

#[allow(dead_code)]
impl HitObjects {
    pub fn len(&self) -> usize {
        for_each_hit_variant!(self, |v| v.len())
    }

    pub fn is_empty(&self) -> bool {
        for_each_hit_variant!(self, |v| v.is_empty())
    }

    pub fn as_standard(&self) -> Option<&[StandardHitObject]> {
        match self {
            HitObjects::Standard(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_taiko(&self) -> Option<&[TaikoHitObject]> {
        match self {
            HitObjects::Taiko(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_catch(&self) -> Option<&[CatchHitObject]> {
        match self {
            HitObjects::Catch(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_mania(&self) -> Option<&[ManiaHitObject]> {
        match self {
            HitObjects::Mania(v) => Some(v),
            _ => None,
        }
    }
}

// 键值区段使用 Vec 保留插入顺序，并提供查找辅助方法。
#[derive(Debug, Clone, Default)]
pub struct KvSection {
    pub entries: Vec<(String, String)>,
    index: BTreeMap<String, usize>,
}

impl KvSection {
    pub fn insert(&mut self, key: &str, value: String) {
        if let Some(&i) = self.index.get(key) {
            self.entries[i].1 = value;
        } else {
            self.index.insert(key.to_string(), self.entries.len());
            self.entries.push((key.to_string(), value));
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.index.get(key).map(|&i| self.entries[i].1.as_str())
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|v| v.trim().parse::<f64>().ok())
    }

    pub fn get_f64_or(&self, key: &str, default: f64) -> f64 {
        self.get_f64(key).unwrap_or(default)
    }
}

#[derive(Debug, Clone)]
pub struct Beatmap {
    pub metadata: KvSection,
    pub difficulty: KvSection,
    pub general: KvSection,
    pub timing_points: Vec<TimingPoint>,
    pub hit_objects: HitObjects,
    pub break_periods: Vec<BreakPeriod>,
    /// `[Events]` 区段中的谱面背景文件名。
    pub background_filename: Option<String>,
    /// 谱面 [Colours] 区段中的连击颜色（按 Combo1..ComboN 顺序）。
    pub combo_colors: Vec<[u8; 3]>,
    /// [Editor] 区段中的 BeatDivisor，未设置时为 0。
    pub beat_divisor: i32,
}

impl Beatmap {
    pub fn mode(&self) -> i32 {
        self.general
            .get("Mode")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    pub fn format_version(&self) -> i32 {
        self.general
            .get("FormatVersion")
            .and_then(|v| v.parse().ok())
            .unwrap_or(14)
    }

    pub fn beatmap_set_id(&self) -> Option<u64> {
        self.metadata
            .get("BeatmapSetID")
            .and_then(|value| value.trim().parse().ok())
            .filter(|id| *id > 0)
    }

    pub fn audio_filename(&self) -> Option<&str> {
        self.general
            .get("AudioFilename")
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn audio_lead_in_ms(&self) -> i64 {
        self.general
            .get("AudioLeadIn")
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0)
    }

    pub fn stack_leniency(&self) -> f64 {
        self.general.get_f64_or("StackLeniency", 0.7)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beatmap_with_audio_fields(
        set_id: Option<&str>,
        filename: Option<&str>,
        lead_in: Option<&str>,
    ) -> Beatmap {
        let mut metadata = KvSection::default();
        if let Some(value) = set_id {
            metadata.insert("BeatmapSetID", value.to_string());
        }
        let mut general = KvSection::default();
        if let Some(value) = filename {
            general.insert("AudioFilename", value.to_string());
        }
        if let Some(value) = lead_in {
            general.insert("AudioLeadIn", value.to_string());
        }
        Beatmap {
            metadata,
            difficulty: KvSection::default(),
            general,
            timing_points: Vec::new(),
            hit_objects: HitObjects::Standard(Vec::new()),
            break_periods: Vec::new(),
            background_filename: None,
            combo_colors: Vec::new(),
            beat_divisor: 0,
        }
    }

    #[test]
    fn audio_metadata_getters_validate_and_default() {
        let valid = beatmap_with_audio_fields(Some("123"), Some(" song.mp3 "), Some("-250"));
        assert_eq!(valid.beatmap_set_id(), Some(123));
        assert_eq!(valid.audio_filename(), Some("song.mp3"));
        assert_eq!(valid.audio_lead_in_ms(), -250);

        let defaults = beatmap_with_audio_fields(Some("invalid"), Some("  "), None);
        assert_eq!(defaults.beatmap_set_id(), None);
        assert_eq!(defaults.audio_filename(), None);
        assert_eq!(defaults.audio_lead_in_ms(), 0);
    }

    #[test]
    fn stack_leniency_uses_general_value_or_game_default() {
        let mut beatmap = beatmap_with_audio_fields(None, None, None);
        assert_eq!(beatmap.stack_leniency(), 0.7);

        beatmap.general.insert("StackLeniency", "0.2".to_string());
        assert_eq!(beatmap.stack_leniency(), 0.2);

        beatmap
            .general
            .insert("StackLeniency", "invalid".to_string());
        assert_eq!(beatmap.stack_leniency(), 0.7);
    }
}
