//! Hit-object parsing for all four game modes.
//!
//! Each mode's parser converts raw CSV lines into its typed hit-object vector.
//! Slider end-time computation (including timing-point lookups) is shared across
//! standard, taiko, and catch.

use crate::models::*;

pub(crate) struct SliderFields {
    pub slider_type: String,
    pub points: Vec<(i32, i32)>,
    pub repeats: i32,
    pub pixel_length: f64,
    pub edge_hitsounds: Vec<i32>,
}

pub(crate) fn parse_slider_fields(parts: &[&str]) -> Option<SliderFields> {
    let mut slider_parts = parts[5].split('|');
    let slider_type = slider_parts.next()?.to_string();
    let mut points = Vec::new();
    for p in slider_parts {
        let (x, y) = p.split_once(':')?;
        points.push((x.parse().ok()?, y.parse().ok()?));
    }
    let repeats: i32 = parts.get(6)?.parse().ok()?;
    let pixel_length: f64 = parts.get(7)?.parse().ok()?;
    let mut edge_hitsounds = Vec::new();
    if let Some(eh) = parts.get(8) {
        if !eh.is_empty() {
            for v in eh.split('|') {
                if !v.is_empty() {
                    edge_hitsounds.push(v.parse().ok()?);
                }
            }
        }
    }
    Some(SliderFields {
        slider_type,
        points,
        repeats,
        pixel_length,
        edge_hitsounds,
    })
}

pub(crate) fn parse_standard(
    lines: &[&str],
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<Vec<StandardHitObject>> {
    let mut objects = Vec::with_capacity(lines.len());
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 5 {
            continue;
        }
        let x: i32 = parts[0].parse::<f64>().ok()? as i32;
        let y: i32 = parts[1].parse::<f64>().ok()? as i32;
        let start_time: i64 = parts[2].parse().ok()?;
        let hit_type: i32 = parts[3].parse().ok()?;
        let hitsound: i32 = parts[4].parse().ok()?;
        let end_time = parse_end_time(&parts, start_time, hit_type, difficulty, timing_points)?;

        let mut obj = StandardHitObject {
            x,
            y,
            start_time,
            end_time,
            hit_type,
            hitsound,
            new_combo: hit_type & 4 != 0,
            combo_offset: (hit_type & 112) >> 4,
            ..Default::default()
        };
        if hit_type & 2 != 0 {
            let sf = parse_slider_fields(&parts)?;
            obj.slider_type = Some(sf.slider_type);
            obj.slider_points = sf.points;
            obj.slider_repeats = sf.repeats;
            obj.slider_pixel_length = sf.pixel_length;
            obj.slider_edge_hitsounds = sf.edge_hitsounds;
        }
        objects.push(obj);
    }
    objects.sort_by_key(|o| (o.start_time, o.end_time));
    Some(objects)
}

pub(crate) fn parse_taiko(
    lines: &[&str],
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<Vec<TaikoHitObject>> {
    let mut objects = Vec::with_capacity(lines.len());
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 5 {
            continue;
        }
        let start_time: i64 = parts[2].parse().ok()?;
        let hit_type: i32 = parts[3].parse().ok()?;
        let hitsound: i32 = parts[4].parse().ok()?;
        let end_time = parse_end_time(&parts, start_time, hit_type, difficulty, timing_points)?;
        objects.push(TaikoHitObject {
            start_time,
            end_time,
            hit_type,
            hitsound,
        });
    }
    objects.sort_by_key(|o| (o.start_time, o.end_time));
    Some(objects)
}

pub(crate) fn parse_catch(
    lines: &[&str],
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<Vec<CatchHitObject>> {
    let mut objects = Vec::with_capacity(lines.len());
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 5 {
            continue;
        }
        let x: i32 = parts[0].parse::<f64>().ok()? as i32;
        let y: i32 = parts[1].parse::<f64>().ok()? as i32;
        let start_time: i64 = parts[2].parse().ok()?;
        let hit_type: i32 = parts[3].parse().ok()?;
        let end_time = parse_end_time(&parts, start_time, hit_type, difficulty, timing_points)?;

        let mut obj = CatchHitObject {
            x,
            y,
            start_time,
            end_time,
            hit_type,
            new_combo: hit_type & 4 != 0,
            combo_offset: (hit_type & 112) >> 4,
            slider_type: None,
            slider_points: Vec::new(),
            slider_repeats: 1,
            slider_pixel_length: 0.0,
        };
        if hit_type & 2 != 0 {
            let sf = parse_slider_fields(&parts)?;
            obj.slider_type = Some(sf.slider_type);
            obj.slider_points = sf.points;
            obj.slider_repeats = sf.repeats;
            obj.slider_pixel_length = sf.pixel_length;
        }
        objects.push(obj);
    }
    objects.sort_by_key(|o| (o.start_time, o.end_time));
    Some(objects)
}

pub(crate) fn parse_mania(lines: &[&str], difficulty: &KvSection) -> Option<Vec<ManiaHitObject>> {
    let key_count = difficulty.get_f64("CircleSize")? as i64;
    let mut objects = Vec::with_capacity(lines.len());
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 5 {
            continue;
        }
        let x: i64 = parts[0].parse::<f64>().ok()? as i64;
        let start_time: i64 = parts[2].parse().ok()?;
        let hit_type: i32 = parts[3].parse().ok()?;
        let lane = (x * key_count).div_euclid(512).clamp(0, key_count - 1) as i32;
        let is_long_note = hit_type & 128 != 0;
        let mut end_time = start_time;
        if is_long_note {
            let head = parts.get(5)?.split(':').next()?;
            end_time = head.parse().ok()?;
        }
        objects.push(ManiaHitObject {
            lane,
            start_time,
            end_time,
            is_long_note,
        });
    }
    objects.sort_by_key(|o| (o.start_time, o.end_time));
    Some(objects)
}

fn parse_end_time(
    parts: &[&str],
    start_time: i64,
    hit_type: i32,
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<i64> {
    if hit_type & 8 != 0 {
        return parts.get(5)?.parse::<f64>().ok().map(|v| v as i64);
    }
    if hit_type & 2 != 0 {
        return parse_slider_end_time(parts, start_time, difficulty, timing_points);
    }
    Some(start_time)
}

fn parse_slider_end_time(
    parts: &[&str],
    start_time: i64,
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<i64> {
    let slides: f64 = parts.get(6)?.parse::<i32>().ok()? as f64;
    let pixel_length: f64 = parts.get(7)?.parse().ok()?;
    let slider_multiplier = difficulty.get_f64("SliderMultiplier")?;
    let (beat_length, slider_velocity) = resolve_slider_timing(start_time, timing_points);
    let duration = pixel_length / (slider_multiplier * 100.0 * slider_velocity) * beat_length * slides;
    Some(start_time + round_half_even(duration))
}

/// Resolve the effective beat_length and slider_velocity at a given start_time
/// by scanning the timing points up to that time.
pub fn resolve_slider_timing(start_time: i64, timing_points: &[TimingPoint]) -> (f64, f64) {
    let mut beat_length = timing_points[0].beat_length;
    let mut slider_velocity = 1.0;
    for point in timing_points {
        if point.time > start_time as f64 {
            break;
        }
        if point.uninherited {
            beat_length = point.beat_length;
            slider_velocity = 1.0;
        } else if point.beat_length < 0.0 {
            slider_velocity = -100.0 / point.beat_length;
        }
    }
    (beat_length, slider_velocity)
}

// Python's round() = banker's rounding.
pub fn round_half_even(v: f64) -> i64 {
    let floor = v.floor();
    let diff = v - floor;
    if diff > 0.5 {
        floor as i64 + 1
    } else if diff < 0.5 {
        floor as i64
    } else {
        let f = floor as i64;
        if f % 2 == 0 {
            f
        } else {
            f + 1
        }
    }
}
