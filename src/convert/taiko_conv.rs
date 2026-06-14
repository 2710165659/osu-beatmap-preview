//! standard → taiko conversion (mode 1).
//! RNG call order and float32 round-trip points must match Python exactly.

use crate::errors::{PreviewError, Result};
use crate::models::{Beatmap, HitObjects, StandardHitObject, TaikoHitObject, TimingPoint};
use crate::mods::ModSettings;

use super::{almost_equals, kiai_at, std_objects};

// C# constant is 1.4f; keep it as a float32 value.
const VELOCITY_MULTIPLIER: f64 = 1.4f32 as f64;
const OSU_BASE_SCORING_DISTANCE: f64 = 100.0;

const DRUMROLL_FLAG: i32 = 2;
const SWELL_FLAG: i32 = 8;

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

    let mut taiko_objects: Vec<TaikoHitObject> = Vec::new();
    for hit_object in objects {
        taiko_objects.extend(taiko_convert_hit_object(hit_object, beatmap));
    }
    taiko_objects.sort_by_key(|ho| (ho.start_time, ho.end_time));

    let mut new_general = beatmap.general.clone();
    new_general.insert("Mode", "1".to_string());

    // EZ/HR difficulty mods are applied after Convert() in lazer; keep the
    // original difficulty here and let the render stage handle scroll speed.
    Ok(Beatmap {
        metadata: beatmap.metadata.clone(),
        difficulty: beatmap.difficulty.clone(),
        general: new_general,
        timing_points: taiko_convert_timing_points(beatmap, objects),
        hit_objects: HitObjects::Taiko(taiko_objects),
        break_periods: beatmap.break_periods.clone(),
        combo_colors: beatmap.combo_colors.clone(),
    })
}

fn taiko_convert_hit_object(
    hit_object: &StandardHitObject,
    beatmap: &Beatmap,
) -> Vec<TaikoHitObject> {
    if hit_object.hit_type & 2 != 0 {
        return taiko_convert_slider(hit_object, beatmap);
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
) -> Vec<TaikoHitObject> {
    let (taiko_duration, tick_spacing) = taiko_slider_conversion_values(hit_object, beatmap);

    if taiko_should_convert_slider_to_hits(hit_object, beatmap, tick_spacing) {
        let mut result: Vec<TaikoHitObject> = Vec::new();
        let all_hitsounds = taiko_slider_node_hitsounds(hit_object);
        let mut sample_index: usize = 0;
        let mut current_time = hit_object.start_time as f64;
        // stable/lazer add tickSpacing / 8 of tolerance so float drift doesn't
        // swallow the last subdivided hit.
        let end_time = (hit_object.start_time + taiko_duration) as f64 + tick_spacing / 8.0;

        while current_time <= end_time + 1e-7 {
            result.push(TaikoHitObject {
                start_time: current_time as i64,
                end_time: current_time as i64,
                hit_type: 0,
                hitsound: all_hitsounds[sample_index],
            });
            sample_index = (sample_index + 1) % all_hitsounds.len();

            if almost_equals(tick_spacing, 0.0) {
                break;
            }
            current_time += tick_spacing;
        }

        return result;
    }

    vec![TaikoHitObject {
        start_time: hit_object.start_time,
        end_time: hit_object.start_time + taiko_duration,
        hit_type: DRUMROLL_FLAG,
        hitsound: hit_object.hitsound,
    }]
}

fn taiko_slider_conversion_values(
    hit_object: &StandardHitObject,
    beatmap: &Beatmap,
) -> (i64, f64) {
    let spans = i32::max(1, hit_object.slider_repeats);

    // Do not merge these three steps; lazer deliberately keeps the
    // intermediate float error to match stable.
    let mut distance = hit_object.slider_pixel_length;
    distance *= VELOCITY_MULTIPLIER;
    distance *= spans as f64;

    let timing_beat_length = timing_beat_length_at(hit_object.start_time, &beatmap.timing_points);
    let slider_velocity = taiko_slider_velocity_at(hit_object.start_time, &beatmap.timing_points);
    let mut beat_length = precision_adjusted_beat_length(timing_beat_length, slider_velocity);

    let slider_multiplier = taiko_slider_multiplier(beatmap);
    let slider_tick_rate = taiko_slider_tick_rate(beatmap);
    let slider_scoring_point_distance =
        OSU_BASE_SCORING_DISTANCE * (slider_multiplier * VELOCITY_MULTIPLIER) / slider_tick_rate;

    let taiko_velocity = slider_scoring_point_distance * slider_tick_rate;
    let taiko_duration = (distance / taiko_velocity * beat_length) as i64;

    // v8+ maps only use the SV-precision-adjusted beatLength for the duration
    // above; tickSpacing falls back to the current red-line BPM.
    if beatmap.format_version() >= 8 {
        beat_length = timing_beat_length;
    }

    let tick_spacing = f64::min(
        beat_length / slider_tick_rate,
        taiko_duration as f64 / spans as f64,
    );
    (taiko_duration, tick_spacing)
}

fn taiko_should_convert_slider_to_hits(
    hit_object: &StandardHitObject,
    beatmap: &Beatmap,
    tick_spacing: f64,
) -> bool {
    let spans = i32::max(1, hit_object.slider_repeats);
    // Deliberately recomputed (not reusing the values above); osu!lazer does
    // the same to preserve stable-compatible float behaviour.
    let mut distance = hit_object.slider_pixel_length;
    distance *= VELOCITY_MULTIPLIER;
    distance *= spans as f64;

    let timing_beat_length = timing_beat_length_at(hit_object.start_time, &beatmap.timing_points);
    let slider_velocity = taiko_slider_velocity_at(hit_object.start_time, &beatmap.timing_points);
    let mut beat_length = precision_adjusted_beat_length(timing_beat_length, slider_velocity);

    let slider_multiplier = taiko_slider_multiplier(beatmap);
    let slider_tick_rate = taiko_slider_tick_rate(beatmap);
    let slider_scoring_point_distance =
        OSU_BASE_SCORING_DISTANCE * (slider_multiplier * VELOCITY_MULTIPLIER) / slider_tick_rate;
    let taiko_velocity = slider_scoring_point_distance * slider_tick_rate;
    let osu_velocity = taiko_velocity * (1000.0 / beat_length);

    if beatmap.format_version() >= 8 {
        beat_length = timing_beat_length;
    }

    tick_spacing > 0.0 && distance / osu_velocity * 1000.0 < 2.0 * beat_length
}

fn taiko_slider_node_hitsounds(hit_object: &StandardHitObject) -> Vec<i32> {
    if hit_object.slider_edge_hitsounds.is_empty() {
        return vec![hit_object.hitsound];
    }

    // An edge hitsound of 0 is a plain don in osu!; it does not inherit the slider head.
    hit_object.slider_edge_hitsounds.clone()
}

fn taiko_convert_timing_points(
    beatmap: &Beatmap,
    objects: &[StandardHitObject],
) -> Vec<TimingPoint> {
    // standard green lines are slider velocity; after converting to taiko they
    // must not directly become scroll speed. lazer instead emits an
    // EffectControlPoint.ScrollSpeed at each slider's own SV. The inherited
    // points keep only kiai/ordering info via a NaN beat length placeholder.
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

    let mut last_scroll_speed = 1.0;
    let mut additions: Vec<TimingPoint> = Vec::new();

    for hit_object in objects {
        if hit_object.hit_type & 2 == 0 {
            continue;
        }

        let next_scroll_speed =
            taiko_slider_velocity_at(hit_object.start_time, &beatmap.timing_points);
        if almost_equals(last_scroll_speed, next_scroll_speed) {
            continue;
        }

        additions.push(TimingPoint {
            time: hit_object.start_time as f64,
            beat_length: -100.0 / next_scroll_speed,
            meter: meter_at(hit_object.start_time, &beatmap.timing_points),
            uninherited: false,
            kiai_mode: kiai_at(hit_object.start_time, &beatmap.timing_points),
        });
        last_scroll_speed = next_scroll_speed;
    }

    converted.extend(additions);
    converted.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
    converted
}

fn precision_adjusted_beat_length(timing_beat_length: f64, slider_velocity: f64) -> f64 {
    // LegacyRulesetExtensions.GetPrecisionAdjustedBeatLength(..., "taiko").
    // The f32 round-trip mirrors C# clamping at float before going back to double.
    let slider_velocity_as_beat_length = -100.0 / slider_velocity;
    let bpm_multiplier = f64::max(
        10.0,
        f64::min(10000.0, (-slider_velocity_as_beat_length) as f32 as f64),
    ) / 100.0;
    timing_beat_length * bpm_multiplier
}

fn timing_beat_length_at(time: i64, timing_points: &[TimingPoint]) -> f64 {
    let mut beat_length = 500.0;
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

fn taiko_slider_velocity_at(time: i64, timing_points: &[TimingPoint]) -> f64 {
    let mut slider_velocity = 1.0;
    for point in timing_points {
        if point.time > time as f64 {
            break;
        }
        if point.uninherited {
            slider_velocity = 1.0;
        } else if point.beat_length < -0.001 {
            slider_velocity = -100.0 / point.beat_length;
        }
    }
    slider_velocity
}

fn meter_at(time: i64, timing_points: &[TimingPoint]) -> i32 {
    let mut meter = 4;
    for point in timing_points {
        if point.time > time as f64 {
            break;
        }
        if point.uninherited {
            meter = point.meter;
        }
    }
    meter
}

fn taiko_slider_multiplier(beatmap: &Beatmap) -> f64 {
    // The legacy decoder clamps difficulty values into the stable range.
    f64::max(0.4, f64::min(3.6, beatmap.difficulty.get_f64_or("SliderMultiplier", 1.4)))
}

fn taiko_slider_tick_rate(beatmap: &Beatmap) -> f64 {
    f64::max(0.5, f64::min(8.0, beatmap.difficulty.get_f64_or("SliderTickRate", 1.0)))
}
