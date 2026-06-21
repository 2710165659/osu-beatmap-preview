//! Shared mania helpers: key-count resolution, mod applications, SV changes,
//! lane palette, and timing utilities.

use crate::canvas::Rgba;
use crate::errors::PreviewError;
use crate::models::{Beatmap, ManiaHitObject, TimingPoint};
use crate::parser::round_half_even;
use std::collections::BTreeMap;

const MAX_KEY_COUNT: i32 = 18;

pub(crate) fn lane_palette(key_count: i32) -> Vec<Rgba> {
    const W: Rgba = [0xe9, 0xee, 0xf4, 255]; // #e9eef4
    const B: Rgba = [0xbc, 0xdb, 0xf1, 255]; // #bcdbf1
    const G: Rgba = [0xcc, 0xfc, 0xb2, 255]; // #ccfcb2
    const Y: Rgba = [0xff, 0xe2, 0x74, 255]; // #ffe274
    const R: Rgba = [0xff, 0x7a, 0x5c, 255]; // #ff7a5c
    match key_count {
        1 => vec![W],
        2 => vec![W, W],
        3 => vec![W, B, W],
        4 => vec![W, B, B, W],
        5 => vec![W, G, Y, G, W],
        6 => vec![W, G, W, W, G, W],
        7 => vec![W, G, W, Y, W, G, W],
        8 => vec![B, W, G, W, W, G, W, B],
        9 => vec![B, W, G, W, Y, W, G, W, B],
        10 => vec![B, W, G, W, Y, Y, W, G, W, B],
        11 => vec![B, W, G, W, Y, R, Y, W, G, W, B],
        12 => vec![Y, B, W, G, W, Y, Y, W, G, W, B, Y],
        13 => vec![Y, B, W, G, W, Y, R, Y, W, G, W, B, Y],
        14 => vec![W, Y, B, W, G, W, Y, Y, W, G, W, B, Y, W],
        15 => vec![W, Y, B, W, G, W, Y, R, Y, W, G, W, B, Y, W],
        16 => vec![G, W, Y, B, W, G, W, Y, Y, W, G, W, B, Y, W, G],
        17 => vec![G, W, Y, B, W, G, W, Y, R, Y, W, G, W, B, Y, W, G],
        18 => vec![B, G, W, Y, B, W, G, W, Y, Y, W, G, W, B, Y, W, G, B],
        _ => unreachable!("key count clamped to [1, 18]"),
    }
}

pub(crate) fn darken(color: Rgba, ratio: f64) -> Rgba {
    [
        (color[0] as f64 * (1.0 - ratio)) as u8,
        (color[1] as f64 * (1.0 - ratio)) as u8,
        (color[2] as f64 * (1.0 - ratio)) as u8,
        255,
    ]
}

pub(crate) fn resolve_key_count(beatmap: &Beatmap) -> Result<i32, PreviewError> {
    let cs = beatmap
        .difficulty
        .get_f64("CircleSize")
        .ok_or_else(|| PreviewError::new("beatmap difficulty missing CircleSize"))?;
    Ok((cs.trunc() as i32).clamp(1, MAX_KEY_COUNT))
}

pub(crate) fn mania_objects(beatmap: &Beatmap) -> Vec<ManiaHitObject> {
    beatmap.hit_objects.as_mania().map(|v| v.to_vec()).unwrap_or_default()
}

pub(crate) fn is_native_mania(beatmap: &Beatmap) -> bool {
    let source_mode = beatmap
        .general
        .get("PreviewSourceMode")
        .or_else(|| beatmap.general.get("Mode"))
        .unwrap_or("3");
    source_mode.trim() == "3"
}

pub(crate) fn beat_length_at(time: i64, timing_points: &[TimingPoint]) -> f64 {
    let mut beat_length = timing_points.first().map_or(500.0, |p| p.beat_length);
    for point in timing_points {
        if point.time > time as f64 {
            break;
        }
        if point.uninherited {
            beat_length = point.beat_length;
        }
    }
    beat_length
}

/// IN mod: convert gaps between adjacent same-lane objects into holds; last object dropped.
pub(crate) fn apply_inverse_mod(
    hit_objects: &[ManiaHitObject],
    timing_points: &[TimingPoint],
) -> Vec<ManiaHitObject> {
    if hit_objects.is_empty() {
        return Vec::new();
    }
    let mut by_lane: BTreeMap<i32, Vec<ManiaHitObject>> = BTreeMap::new();
    for ho in hit_objects {
        by_lane.entry(ho.lane).or_default().push(*ho);
    }

    let mut result: Vec<ManiaHitObject> = Vec::new();
    for (lane, mut lane_objects) in by_lane {
        lane_objects.sort_by_key(|ho| (ho.start_time, ho.end_time));
        for pair in lane_objects.windows(2) {
            let (current, next_object) = (&pair[0], &pair[1]);
            let gap = (next_object.start_time - current.start_time) as f64;
            let beat_length = beat_length_at(next_object.start_time, timing_points);
            let duration = (gap / 2.0).max(gap - beat_length / 4.0);
            let end_time = current
                .start_time
                .max(round_half_even(current.start_time as f64 + duration));
            result.push(ManiaHitObject {
                lane,
                start_time: current.start_time,
                end_time,
                is_long_note: end_time > current.start_time,
            });
        }
    }
    result.sort_by_key(|ho| (ho.start_time, ho.end_time, ho.lane));
    result
}

/// HO mod: holds become taps at the head; plain notes preserved.
pub(crate) fn apply_hold_off_mod(hit_objects: &[ManiaHitObject]) -> Vec<ManiaHitObject> {
    let mut result: Vec<ManiaHitObject> = hit_objects
        .iter()
        .map(|ho| ManiaHitObject {
            lane: ho.lane,
            start_time: ho.start_time,
            end_time: ho.start_time,
            is_long_note: false,
        })
        .collect();
    result.sort_by_key(|ho| (ho.start_time, ho.end_time, ho.lane));
    result
}

pub(crate) fn build_sv_changes(timing_points: &[TimingPoint], chart_end_time: i64) -> Vec<(i64, f64)> {
    let mut changes: Vec<(i64, f64)> = Vec::new();
    let mut prev_sv: Option<f64> = None;
    for point in timing_points {
        if point.uninherited
            || !(point.beat_length < 0.0)
            || point.time < 0.0
            || point.time > chart_end_time as f64
        {
            continue;
        }
        let sv = -100.0 / point.beat_length;
        if prev_sv.is_none_or(|prev| (sv - prev).abs() > 0.001) {
            changes.push((point.time.trunc() as i64, sv));
            prev_sv = Some(sv);
        }
    }
    changes
}

pub(crate) fn format_sv_label(sv: f64) -> String {
    let rounded = (sv * 10.0).round() / 10.0;
    if sv == rounded {
        format!("{sv:.1}x")
    } else {
        format!("{sv:.2}x")
    }
}
