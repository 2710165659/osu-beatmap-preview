use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimingPoint {
    pub time: f64,
    pub beat_length: f64,
    pub meter: i32,
    pub uninherited: bool,
    pub kiai_mode: bool,
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
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TaikoHitObject {
    pub start_time: i64,
    pub end_time: i64,
    pub hit_type: i32,
    pub hitsound: i32,
}

#[derive(Debug, Clone)]
pub struct CatchHitObject {
    pub x: i32,
    pub y: i32,
    pub start_time: i64,
    pub end_time: i64,
    pub hit_type: i32,
    pub new_combo: bool,
    pub combo_offset: i32,
    pub slider_type: Option<String>,
    pub slider_points: Vec<(i32, i32)>,
    pub slider_repeats: i32,
    pub slider_pixel_length: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct ManiaHitObject {
    pub lane: i32,
    pub start_time: i64,
    pub end_time: i64,
    pub is_long_note: bool,
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
}
