//! standard → taiko 转换（模式 1）。
//! RNG 调用顺序和 float32 往返点必须与 Python 完全一致。

use crate::domain::errors::{PreviewError, Result};
use crate::domain::models::{Beatmap, HitObjects, StandardHitObject, TaikoHitObject, TimingPoint};
use crate::domain::mods::ModSettings;

use crate::domain::shared::conversion::{almost_equals, std_objects, TimingCursor};

// C# 常量为 1.4f，此处保持 float32 数值。
const VELOCITY_MULTIPLIER: f64 = 1.4f32 as f64;
const OSU_BASE_SCORING_DISTANCE: f64 = 100.0;
const DRUMROLL_FLAG: i32 = 2;
const SWELL_FLAG: i32 = 8;

/// 每条滑条预计算的值，在时长与音符转换检查间共享，
/// 避免 lazer 为兼容 stable 而进行的重复计算。
struct SliderConversionValues {
    taiko_duration: i64,
    tick_spacing: f64,
    distance: f64,
    /// 经 precision 调整后的拍长，用于 taiko 速度与时长。
    adjusted_beat_length: f64,
    /// 判据阈值使用的拍长：v8+ 为 timing point 的原始拍长，否则为 precision 调整值。
    beat_length: f64,
    taiko_velocity: f64,
}

pub fn taiko_convert(
    beatmap: &Beatmap,
    target_mode: i32,
    _mods: Option<&ModSettings>,
) -> Result<Beatmap> {
    if beatmap.mode() != 0 {
        return Err(PreviewError::new(
            "source beatmap must be osu!standard (mode=0)",
        ));
    }
    if target_mode != 1 {
        return Err(PreviewError::new(
            "only taiko (mode=1) conversion is supported here",
        ));
    }

    let objects = std_objects(beatmap);
    if objects.is_empty() {
        return Err(PreviewError::new(
            "standard beatmap has no hit objects to convert",
        ));
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
        background_filename: beatmap.background_filename.clone(),
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
            samples: hit_object.samples.clone(),
        }];
    }

    vec![TaikoHitObject {
        start_time: hit_object.start_time,
        end_time: hit_object.start_time,
        hit_type: 0,
        hitsound: hit_object.hitsound,
        samples: hit_object.samples.clone(),
    }]
}

fn taiko_convert_slider(
    hit_object: &StandardHitObject,
    beatmap: &Beatmap,
    cursor: &TimingCursor,
) -> Vec<TaikoHitObject> {
    let vals = slider_conversion_values(hit_object, beatmap, cursor);

    if should_convert_slider_to_hits(&vals) {
        let mut result: Vec<TaikoHitObject> = Vec::new();
        let all_hitsounds = taiko_slider_node_hitsounds(hit_object);
        let mut sample_index: usize = 0;
        let mut current_time = hit_object.start_time as f64;
        let end_time =
            (hit_object.start_time + vals.taiko_duration) as f64 + vals.tick_spacing / 8.0;

        while current_time <= end_time + 1e-7 {
            result.push(TaikoHitObject {
                start_time: current_time as i64,
                end_time: current_time as i64,
                hit_type: 0,
                hitsound: all_hitsounds[sample_index],
                samples: hit_object.samples.clone(),
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
        samples: hit_object.samples.clone(),
    }]
}

/// 滑条转 taiko 的几何量（时长与 tick 间距），供转换与打击音共用。
pub struct TaikoSliderGeometry {
    pub taiko_duration: i64,
    pub tick_spacing: f64,
}

/// standard 物件按 taiko 规则发声时的一次打击。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TaikoHitsoundEvent {
    /// 打击时间（绝对谱面毫秒）。
    pub time_ms: f64,
    /// 转换后的打击音位掩码：位 3（8）表示蓝音符（rim），其余为红音符（center）。
    pub hitsound: i32,
}

/// 按 taiko 转谱规则把 standard 物件展开为打击音事件。
///
/// 只处理「是否发声、何时发声、是红还是蓝」，与预览的时间轴共用同一套
/// tick 间距与滑条时长计算，因此画面与声音不会出现分歧。
pub fn taiko_hitsound_events(
    beatmap: &Beatmap,
    object: &StandardHitObject,
) -> Vec<TaikoHitsoundEvent> {
    let hitsound = object.hitsound;

    if object.hit_type & 2 != 0 {
        // 复用转谱实现保证「何时发声、是红还是蓝」与画面完全一致：鼓滚（间距过大的滑条）
        // 在转谱里只产生一个 DRUMROLL 对象，因此这里也只会发一次声。
        let mut cursor = TimingCursor::new(&beatmap.timing_points);
        cursor.advance_to(object.start_time);
        let mut converted = taiko_convert_slider(object, beatmap, &cursor);
        converted.sort_by_key(|hit| hit.start_time);
        return converted
            .into_iter()
            .map(|hit| TaikoHitsoundEvent {
                time_ms: hit.start_time as f64,
                hitsound: hit.hitsound,
            })
            .collect();
    }

    vec![TaikoHitsoundEvent {
        time_ms: object.start_time as f64,
        hitsound,
    }]
}

/// 滑条在 taiko 下的时长与 tick 间距。
///
/// `timing_beat_length` 必须是 timing point 的原始拍长：lazer 先按 precision 调整值
/// 算时长与 tick 间距，只有 v8+ 的阈值判定才换回原始拍长。这里不需要那个阈值，
/// 因此内部只使用调整后的值。
pub fn taiko_slider_geometry(
    hit_object: &StandardHitObject,
    beatmap: &Beatmap,
    timing_beat_length: f64,
    slider_velocity: f64,
) -> TaikoSliderGeometry {
    let spans = i32::max(1, hit_object.slider_repeats);

    let mut distance = hit_object.slider_pixel_length;
    distance *= VELOCITY_MULTIPLIER;
    distance *= spans as f64;

    let duration_beat_length = precision_adjusted_beat_length(timing_beat_length, slider_velocity);

    let slider_multiplier = taiko_slider_multiplier(beatmap);
    let slider_tick_rate = taiko_slider_tick_rate(beatmap);
    let slider_scoring_point_distance =
        OSU_BASE_SCORING_DISTANCE * (slider_multiplier * VELOCITY_MULTIPLIER) / slider_tick_rate;

    let taiko_velocity = slider_scoring_point_distance * slider_tick_rate;
    let taiko_duration = (distance / taiko_velocity * duration_beat_length) as i64;

    // 与 stable 一致：tick 间距取 beat/tickRate 与 每段时长 的较小值。
    // lazer 此处用的仍是 precision 调整后的拍长，原始拍长只参与判定阈值。
    let tick_spacing = f64::min(
        duration_beat_length / slider_tick_rate,
        taiko_duration as f64 / spans as f64,
    );

    TaikoSliderGeometry {
        taiko_duration,
        tick_spacing,
    }
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
    let adjusted_beat_length = precision_adjusted_beat_length(timing_beat_length, slider_velocity);

    let slider_multiplier = taiko_slider_multiplier(beatmap);
    let slider_tick_rate = taiko_slider_tick_rate(beatmap);
    let slider_scoring_point_distance =
        OSU_BASE_SCORING_DISTANCE * (slider_multiplier * VELOCITY_MULTIPLIER) / slider_tick_rate;

    let taiko_velocity = slider_scoring_point_distance * slider_tick_rate;
    // geometry 内部自行做 precision 调整；这里传原始拍长，与 lazer 保持一致。
    let geometry = taiko_slider_geometry(hit_object, beatmap, timing_beat_length, slider_velocity);
    let taiko_duration = geometry.taiko_duration;

    // stable 只在判定阈值里用原始拍长，v8 起才这样；osuV 仍要用调整后的拍长。
    let beat_length = if beatmap.format_version() >= 8 {
        timing_beat_length
    } else {
        adjusted_beat_length
    };

    SliderConversionValues {
        taiko_duration,
        tick_spacing: geometry.tick_spacing,
        distance,
        adjusted_beat_length,
        beat_length,
        taiko_velocity,
    }
}

fn should_convert_slider_to_hits(vals: &SliderConversionValues) -> bool {
    // 顺序必须与 lazer 一致：osuV 用的是 precision 调整后的拍长，
    // 只有阈值 2*beatLength 才换回原始拍长（v8+），否则判据会被整体放大。
    let osu_velocity = vals.taiko_velocity * (1000.0 / vals.adjusted_beat_length);
    let rate = vals.distance / osu_velocity * 1000.0;

    vals.tick_spacing > 0.0 && rate < 2.0 * vals.beat_length
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
                    omit_first_bar_line: point.omit_first_bar_line,
                    sample_set: 0,
                    sample_index: 0,
                    sample_volume: 100,
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
            omit_first_bar_line: false,
            sample_set: 0,
            sample_index: 0,
            sample_volume: 100,
        });
        last_scroll_speed = next_scroll_speed;
    }

    converted.extend(additions);
    converted.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
    converted
}

fn precision_adjusted_beat_length(timing_beat_length: f64, slider_velocity: f64) -> f64 {
    let slider_velocity_as_beat_length = -100.0 / slider_velocity;
    let raw_multiplier = (-slider_velocity_as_beat_length) as f32 as f64;
    // 原 min/max 链在 NaN 时回退到上界 10000.0。
    let bpm_multiplier = if raw_multiplier.is_nan() {
        10000.0
    } else {
        raw_multiplier.clamp(10.0, 10000.0)
    } / 100.0;
    timing_beat_length * bpm_multiplier
}

fn taiko_slider_multiplier(beatmap: &Beatmap) -> f64 {
    let multiplier = beatmap.difficulty.get_f64_or("SliderMultiplier", 1.4);
    if multiplier.is_nan() {
        3.6
    } else {
        multiplier.clamp(0.4, 3.6)
    }
}

fn taiko_slider_tick_rate(beatmap: &Beatmap) -> f64 {
    let tick_rate = beatmap.difficulty.get_f64_or("SliderTickRate", 1.0);
    if tick_rate.is_nan() {
        8.0
    } else {
        tick_rate.clamp(0.5, 8.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(
        distance: f64,
        taiko_velocity: f64,
        adjusted: f64,
        threshold: f64,
    ) -> SliderConversionValues {
        SliderConversionValues {
            taiko_duration: 160,
            tick_spacing: 160.0,
            distance,
            adjusted_beat_length: adjusted,
            beat_length: threshold,
            taiko_velocity,
        }
    }

    /// 判据里的 osuV 必须用 precision 调整后的拍长，只有阈值 2*beatLength 用原始拍长。
    /// `taiko_velocity` 取 1.0 时 rate = distance / (1000 / adjusted) * 1000，
    /// 两条用例分别落在阈值两侧，交换拍长会让期望整体反转。
    #[test]
    fn osu_velocity_uses_adjusted_beat_length() {
        // adjusted = 160：rate = 3 / (1000 / 160) * 1000 = 18.75
        // 阈值拍长 320 → limit = 640，18.75 < 640 → 拆分
        assert!(should_convert_slider_to_hits(&values(3.0, 1.0, 160.0, 320.0)));
        // 阈值拍长 8 → limit = 16，18.75 > 16 → 不拆分。
        // 若 osuV 误用拍长 8：rate = 375 > 16，期望反转，用例失败。
        assert!(!should_convert_slider_to_hits(&values(3.0, 1.0, 160.0, 8.0)));
    }
}
