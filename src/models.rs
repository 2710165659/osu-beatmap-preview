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

impl HitObjects {
    pub fn len(&self) -> usize {
        match self {
            HitObjects::Standard(v) => v.len(),
            HitObjects::Taiko(v) => v.len(),
            HitObjects::Catch(v) => v.len(),
            HitObjects::Mania(v) => v.len(),
        }
    }

}

// Key/value sections preserve insertion order via Vec; lookup helper provided.
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
    /// Combo colours from the beatmap's [Colours] section (Combo1..ComboN order).
    pub combo_colors: Vec<[u8; 3]>,
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
}
