//! standard → taiko conversion (mode 1).
//! RNG call order and float32 round-trip points must match Python exactly.

use crate::errors::{PreviewError, Result};
use crate::models::{Beatmap, HitObjects, StandardHitObject, TaikoHitObject, TimingPoint};
use crate::mods::ModSettings;

use super::{almost_equals, std_objects, TimingCursor};

// C# constant is 1.4f; keep it as a float32 value.
const VELOCITY_MULTIPLIER: f64 = 1.4f32 as f64;
const OSU_BASE_SCORING_DISTANCE: f64 = 100.0;

const DRUMROLL_FLAG: i32 = 2;
const SWELL_FLAG: i32 = 8;

/// Pre-computed per-slider values shared between duration and hit-conversion
/// checks, avoiding the duplicate computation that lazer does for stable compat.
struct SliderConversionValues {
    taiko_duration: i64,
    tick_spacing: f64,
    distance: f64,
    timing_beat_length: f64,
    beat_length: f64,
    taiko_velocity: f64,
}

pub(crate) fn taiko_convert(
    beatmap: &Beatmap,
    target_mode: i32,
    _mods: Option<&ModSettings>,
) -> Result<Beatmap> {
    if beatmap.mode() != 0 {
        return Err(PreviewError::new("source beatmap must be osu!standard (mode=0)"));
    }
    if target_mode != 1 {
        return Err(PreviewError::new("only taiko (mode=1) conversion is supported here"));
    }

    let objects = std_objects(beatmap);
    if objects.is_empty() {
        return Err(PreviewError::new("standard beatmap has no hit objects to convert"));
    }

    let mut cursor = TimingCursor::new(&beatmap.timing_points);
    let mut taiko_objects: Vec<TaikoHitObject> = Vec::new();
    for hit_object in objects {
        cursor.advance_to(hit_object.start_time);
        taiko_objects.extend(taiko_convert_hit_object(hit_object, beatmap, &cursor));
    }
    taiko_objects.sort_by_key(|ho| (ho.start_time, ho.end_time));

    let mut new_general = beatmap.general.clone();
    new_general.insert("Mode", "1".to_string());

    Ok(Beatmap {
        metadata: beatmap.metadata.clone(),
        difficulty: beatmap.difficulty.clone(),
        general: new_general,
        timing_points: taiko_convert_timing_points(beatmap, objects),
        hit_objects: HitObjects::Taiko(taiko_objects),
        break_periods: beatmap.break_periods.clone(),
        combo_colors: beatmap.combo_colors.clone(),
        beat_divisor: beatmap.beat_divisor,
    })
}

fn taiko_convert_hit_object(
    hit_object: &StandardHitObject,
    beatmap: &Beatmap,
    cursor: &TimingCursor,
) -> Vec<TaikoHitObject> {
    if hit_object.hit_type & 2 != 0 {
        return taiko_convert_slider(hit_object, beatmap, cursor);
    }

    if hit_object.hit_type & 8 != 0 {
        return vec![TaikoHitObject {
            start_time: hit_object.start_time,
            end_time: hit_object.end_time,
            hit_type: SWELL_FLAG,
            hitsound: hit_object.hitsound,
        }];
    }

    vec![TaikoHitObject {
        start_time: hit_object.start_time,
        end_time: hit_object.start_time,
        hit_type: 0,
        hitsound: hit_object.hitsound,
    }]
}

fn taiko_convert_slider(
    hit_object: &StandardHitObject,
    beatmap: &Beatmap,
    cursor: &TimingCursor,
) -> Vec<TaikoHitObject> {
    let vals = slider_conversion_values(hit_object, beatmap, cursor);

    if should_convert_slider_to_hits(beatmap, &vals) {
        let mut result: Vec<TaikoHitObject> = Vec::new();
        let all_hitsounds = taiko_slider_node_hitsounds(hit_object);
        let mut sample_index: usize = 0;
        let mut current_time = hit_object.start_time as f64;
        let end_time = (hit_object.start_time + vals.taiko_duration) as f64 + vals.tick_spacing / 8.0;

        while current_time <= end_time + 1e-7 {
            result.push(TaikoHitObject {
                start_time: current_time as i64,
                end_time: current_time as i64,
                hit_type: 0,
                hitsound: all_hitsounds[sample_index],
            });
            sample_index = (sample_index + 1) % all_hitsounds.len();

            if almost_equals(vals.tick_spacing, 0.0) {
                break;
            }
            current_time += vals.tick_spacing;
        }

        return result;
    }

    vec![TaikoHitObject {
        start_time: hit_object.start_time,
        end_time: hit_object.start_time + vals.taiko_duration,
        hit_type: DRUMROLL_FLAG,
        hitsound: hit_object.hitsound,
    }]
}

fn slider_conversion_values(
    hit_object: &StandardHitObject,
    beatmap: &Beatmap,
    cursor: &TimingCursor,
) -> SliderConversionValues {
    let spans = i32::max(1, hit_object.slider_repeats);

    let mut distance = hit_object.slider_pixel_length;
    distance *= VELOCITY_MULTIPLIER;
    distance *= spans as f64;

    let timing_beat_length = cursor.beat_length;
    let slider_velocity = cursor.slider_velocity;
    let mut beat_length = precision_adjusted_beat_length(timing_beat_length, slider_velocity);

    let slider_multiplier = taiko_slider_multiplier(beatmap);
    let slider_tick_rate = taiko_slider_tick_rate(beatmap);
    let slider_scoring_point_distance =
        OSU_BASE_SCORING_DISTANCE * (slider_multiplier * VELOCITY_MULTIPLIER) / slider_tick_rate;

    let taiko_velocity = slider_scoring_point_distance * slider_tick_rate;
    let taiko_duration = (distance / taiko_velocity * beat_length) as i64;

    if beatmap.format_version() >= 8 {
        beat_length = timing_beat_length;
    }

    let tick_spacing = f64::min(
        beat_length / slider_tick_rate,
        taiko_duration as f64 / spans as f64,
    );

    SliderConversionValues {
        taiko_duration,
        tick_spacing,
        distance,
        timing_beat_length,
        beat_length,
        taiko_velocity,
    }
}

fn should_convert_slider_to_hits(
    beatmap: &Beatmap,
    vals: &SliderConversionValues,
) -> bool {
    let osu_velocity = vals.taiko_velocity * (1000.0 / vals.beat_length);
    let mut beat_length = vals.beat_length;
    if beatmap.format_version() >= 8 {
        beat_length = vals.timing_beat_length;
    }

    vals.tick_spacing > 0.0
        && vals.distance / osu_velocity * 1000.0 < 2.0 * beat_length
}

fn taiko_slider_node_hitsounds(hit_object: &StandardHitObject) -> Vec<i32> {
    if hit_object.slider_edge_hitsounds.is_empty() {
        return vec![hit_object.hitsound];
    }
    hit_object.slider_edge_hitsounds.clone()
}

fn taiko_convert_timing_points(
    beatmap: &Beatmap,
    objects: &[StandardHitObject],
) -> Vec<TimingPoint> {
    let mut converted: Vec<TimingPoint> = beatmap
        .timing_points
        .iter()
        .map(|point| {
            if point.uninherited {
                *point
            } else {
                TimingPoint {
                    time: point.time,
                    beat_length: f64::NAN,
                    meter: point.meter,
                    uninherited: false,
                    kiai_mode: point.kiai_mode,
                }
            }
        })
        .collect();

    let mut cursor = TimingCursor::new(&beatmap.timing_points);
    let mut last_scroll_speed = 1.0;
    let mut additions: Vec<TimingPoint> = Vec::new();

    for hit_object in objects {
        if hit_object.hit_type & 2 == 0 {
            continue;
        }

        cursor.advance_to(hit_object.start_time);
        let next_scroll_speed = cursor.slider_velocity;
        if almost_equals(last_scroll_speed, next_scroll_speed) {
            continue;
        }

        additions.push(TimingPoint {
            time: hit_object.start_time as f64,
            beat_length: -100.0 / next_scroll_speed,
            meter: cursor.meter,
            uninherited: false,
            kiai_mode: cursor.kiai,
        });
        last_scroll_speed = next_scroll_speed;
    }

    converted.extend(additions);
    converted.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
    converted
}

fn precision_adjusted_beat_length(timing_beat_length: f64, slider_velocity: f64) -> f64 {
    let slider_velocity_as_beat_length = -100.0 / slider_velocity;
    let bpm_multiplier = f64::max(
        10.0,
        f64::min(10000.0, (-slider_velocity_as_beat_length) as f32 as f64),
    ) / 100.0;
    timing_beat_length * bpm_multiplier
}

fn taiko_slider_multiplier(beatmap: &Beatmap) -> f64 {
    f64::max(0.4, f64::min(3.6, beatmap.difficulty.get_f64_or("SliderMultiplier", 1.4)))
}

fn taiko_slider_tick_rate(beatmap: &Beatmap) -> f64 {
    f64::max(0.5, f64::min(8.0, beatmap.difficulty.get_f64_or("SliderTickRate", 1.0)))
}
