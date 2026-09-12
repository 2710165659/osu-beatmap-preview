//! 谱面内部信息快照。
//!
//! 把已经解析好的 [`Beatmap`] 摊平成一份可直接序列化的结构：常用字段单独列出，
//! `[General]` / `[Metadata]` / `[Difficulty]` 三个键值区段全量保留，调用方按需取用。
//! 这里不做任何渲染相关的计算，CLI 的输出与 Web 前端的信息面板共用同一份数据。

use std::collections::BTreeMap;

use serde::Serialize;

use super::models::{Beatmap, HitObjects, KvSection};

/// 谱面内部信息快照。
///
/// 字段全部为原始数据与轻量派生值（时长、BPM、音符数），不含任何渲染结果；
/// 序列化后可以直接交给宿主展示或落盘。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BeatmapInfo {
    // ── [Metadata] 中的常用字段 ──
    pub title: Option<String>,
    pub title_unicode: Option<String>,
    pub artist: Option<String>,
    pub artist_unicode: Option<String>,
    pub creator: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
    pub tags: Option<String>,
    pub beatmap_id: Option<u64>,
    pub beatmap_set_id: Option<u64>,
    // ── 格式与 [General] ──
    pub mode: i32,
    pub mode_name: String,
    pub format_version: i32,
    pub audio_filename: Option<String>,
    pub audio_lead_in_ms: i64,
    pub stack_leniency: f64,
    pub background_filename: Option<String>,
    pub beat_divisor: i32,
    // ── 统计 ──
    pub hit_object_count: usize,
    pub first_object_ms: Option<i64>,
    pub last_object_end_ms: Option<i64>,
    pub chart_duration_ms: Option<i64>,
    pub bpm: Option<f64>,
    pub timing_point_count: usize,
    pub break_period_count: usize,
    /// `[Colours]` 中的连击色，按 Combo1..ComboN 顺序保留原始 RGB 三元组。
    pub combo_colors: Vec<[u8; 3]>,
    // ── [Difficulty] ──
    pub ar: Option<f64>,
    pub cs: Option<f64>,
    pub hp: Option<f64>,
    pub od: Option<f64>,
    // ── 全量键值区段 ──
    /// `[General]` 全量键值；用 BTreeMap 保证序列化结果稳定有序。
    pub general: BTreeMap<String, String>,
    pub metadata: BTreeMap<String, String>,
    pub difficulty: BTreeMap<String, String>,
}

impl BeatmapInfo {
    /// 从解析结果生成信息快照。
    pub fn from_beatmap(beatmap: &Beatmap) -> Self {
        let bounds = object_time_bounds(&beatmap.hit_objects);
        BeatmapInfo {
            title: text(&beatmap.metadata, "Title"),
            title_unicode: text(&beatmap.metadata, "TitleUnicode"),
            artist: text(&beatmap.metadata, "Artist"),
            artist_unicode: text(&beatmap.metadata, "ArtistUnicode"),
            creator: text(&beatmap.metadata, "Creator"),
            version: text(&beatmap.metadata, "Version"),
            source: text(&beatmap.metadata, "Source"),
            tags: text(&beatmap.metadata, "Tags"),
            beatmap_id: id(&beatmap.metadata, "BeatmapID"),
            beatmap_set_id: beatmap.beatmap_set_id(),
            mode: beatmap.mode(),
            mode_name: mode_name(beatmap.mode()),
            format_version: beatmap.format_version(),
            audio_filename: beatmap.audio_filename().map(str::to_string),
            audio_lead_in_ms: beatmap.audio_lead_in_ms(),
            stack_leniency: beatmap.stack_leniency(),
            background_filename: beatmap.background_filename.clone(),
            beat_divisor: beatmap.beat_divisor,
            hit_object_count: beatmap.hit_objects.len(),
            first_object_ms: bounds.map(|(first, _)| first),
            last_object_end_ms: bounds.map(|(_, last)| last),
            chart_duration_ms: bounds.map(|(first, last)| (last - first).max(0)),
            bpm: main_bpm(beatmap),
            timing_point_count: beatmap.timing_points.len(),
            break_period_count: beatmap.break_periods.len(),
            combo_colors: beatmap.combo_colors.clone(),
            ar: beatmap.difficulty.get_f64("ApproachRate"),
            cs: beatmap.difficulty.get_f64("CircleSize"),
            hp: beatmap.difficulty.get_f64("HPDrainRate"),
            od: beatmap.difficulty.get_f64("OverallDifficulty"),
            general: section_map(&beatmap.general),
            metadata: section_map(&beatmap.metadata),
            difficulty: section_map(&beatmap.difficulty),
        }
    }
}

/// 取文本字段：缺失或只有空白时返回 None，避免前端显示空标签。
fn text(section: &KvSection, key: &str) -> Option<String> {
    section
        .get(key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// 取正整数 ID：0 与非法值一律视为缺失。
fn id(section: &KvSection, key: &str) -> Option<u64> {
    section
        .get(key)
        .and_then(|value| value.trim().parse().ok())
        .filter(|value| *value > 0)
}

fn mode_name(mode: i32) -> String {
    match mode {
        0 => "standard",
        1 => "taiko",
        2 => "catch",
        3 => "mania",
        _ => "unknown",
    }
    .to_string()
}

/// 主 BPM：第一个未继承且节拍长度有效的 timing point，保留两位小数。
fn main_bpm(beatmap: &Beatmap) -> Option<f64> {
    beatmap
        .timing_points
        .iter()
        .find(|point| point.uninherited && point.beat_length > 0.0)
        .map(|point| (60_000.0 / point.beat_length * 100.0).round() / 100.0)
}

/// 谱面首尾音符的 (开始时间, 结束时间)；没有任何音符时返回 None。
fn object_time_bounds(hit_objects: &HitObjects) -> Option<(i64, i64)> {
    let mut first = i64::MAX;
    let mut last = i64::MIN;
    let mut any = false;
    match hit_objects {
        HitObjects::Standard(objects) => {
            for object in objects {
                any = true;
                first = first.min(object.start_time);
                last = last.max(object.end_time);
            }
        }
        HitObjects::Taiko(objects) => {
            for object in objects {
                any = true;
                first = first.min(object.start_time);
                last = last.max(object.end_time);
            }
        }
        HitObjects::Catch(objects) => {
            for object in objects {
                any = true;
                first = first.min(object.start_time);
                last = last.max(object.end_time);
            }
        }
        HitObjects::Mania(objects) => {
            for object in objects {
                any = true;
                first = first.min(object.start_time);
                last = last.max(object.end_time);
            }
        }
    }
    any.then_some((first, last))
}

/// `KvSection` 内部已按 Key 去重，这里直接摊平成映射。
fn section_map(section: &KvSection) -> BTreeMap<String, String> {
    section.entries.iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{BreakPeriod, StandardHitObject, TimingPoint};

    fn sample_beatmap() -> Beatmap {
        let mut general = KvSection::default();
        general.insert("AudioFilename", "audio.mp3".to_string());
        general.insert("Mode", "0".to_string());
        general.insert("AudioLeadIn", "1500".to_string());
        general.insert("StackLeniency", "0.5".to_string());

        let mut metadata = KvSection::default();
        metadata.insert("Title", "Sample Song".to_string());
        metadata.insert("Artist", "Sample Artist".to_string());
        metadata.insert("Creator", "Mapper".to_string());
        metadata.insert("Version", "Insane".to_string());
        metadata.insert("BeatmapSetID", "1236927".to_string());
        metadata.insert("BeatmapID", "2628991".to_string());

        let mut difficulty = KvSection::default();
        difficulty.insert("ApproachRate", "9.3".to_string());
        difficulty.insert("CircleSize", "4".to_string());
        difficulty.insert("HPDrainRate", "5".to_string());
        difficulty.insert("OverallDifficulty", "8.5".to_string());

        Beatmap {
            metadata,
            difficulty,
            general,
            timing_points: vec![TimingPoint {
                time: 0.0,
                beat_length: 300.0,
                meter: 4,
                uninherited: true,
                kiai_mode: false,
                omit_first_bar_line: false,
            }],
            hit_objects: HitObjects::Standard(vec![
                StandardHitObject {
                    start_time: 1000,
                    end_time: 1200,
                    ..StandardHitObject::default()
                },
                StandardHitObject {
                    start_time: 2000,
                    end_time: 4000,
                    ..StandardHitObject::default()
                },
            ]),
            break_periods: vec![BreakPeriod {
                start_time: 2500,
                end_time: 3000,
            }],
            background_filename: Some("bg.jpg".to_string()),
            combo_colors: vec![[255, 128, 0]],
            beat_divisor: 4,
        }
    }

    #[test]
    fn 快照包含概览字段与派生统计() {
        let info = BeatmapInfo::from_beatmap(&sample_beatmap());
        assert_eq!(info.title.as_deref(), Some("Sample Song"));
        assert_eq!(info.artist.as_deref(), Some("Sample Artist"));
        assert_eq!(info.version.as_deref(), Some("Insane"));
        assert_eq!(info.beatmap_id, Some(2628991));
        assert_eq!(info.beatmap_set_id, Some(1236927));
        assert_eq!(info.mode_name, "standard");
        assert_eq!(info.audio_filename.as_deref(), Some("audio.mp3"));
        assert_eq!(info.audio_lead_in_ms, 1500);
        assert_eq!(info.background_filename.as_deref(), Some("bg.jpg"));
        assert_eq!(info.hit_object_count, 2);
        assert_eq!(info.first_object_ms, Some(1000));
        assert_eq!(info.last_object_end_ms, Some(4000));
        assert_eq!(info.chart_duration_ms, Some(3000));
        assert_eq!(info.bpm, Some(200.0));
        assert_eq!(info.timing_point_count, 1);
        assert_eq!(info.break_period_count, 1);
        assert_eq!(info.ar, Some(9.3));
        assert_eq!(info.od, Some(8.5));
    }

    #[test]
    fn 键值区段全量输出() {
        let info = BeatmapInfo::from_beatmap(&sample_beatmap());
        // 不只是常用字段：三个区段的每个键都要在快照里。
        assert_eq!(info.metadata.len(), 6);
        assert_eq!(
            info.metadata.get("Creator").map(String::as_str),
            Some("Mapper")
        );
        assert_eq!(info.metadata.get("TitleUnicode"), None);
        assert_eq!(info.difficulty.len(), 4);
        assert_eq!(
            info.general.get("StackLeniency").map(String::as_str),
            Some("0.5")
        );
    }

    #[test]
    fn 缺少可选字段时留空而不是给默认值() {
        let mut beatmap = sample_beatmap();
        beatmap.metadata.insert("Title", "   ".to_string());
        beatmap.hit_objects = HitObjects::Standard(Vec::new());
        beatmap.timing_points.clear();
        let info = BeatmapInfo::from_beatmap(&beatmap);
        assert_eq!(info.title, None);
        assert_eq!(info.chart_duration_ms, None);
        assert_eq!(info.first_object_ms, None);
        assert_eq!(info.bpm, None);
    }

    #[test]
    fn 模式名覆盖四种规则集与未知值() {
        assert_eq!(mode_name(0), "standard");
        assert_eq!(mode_name(1), "taiko");
        assert_eq!(mode_name(2), "catch");
        assert_eq!(mode_name(3), "mania");
        assert_eq!(mode_name(9), "unknown");
    }
}
