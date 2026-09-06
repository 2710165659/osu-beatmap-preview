//! osu!taiko 动画物件的生成与生命周期计算。

use crate::domain::models::{TaikoHitObject, TimingPoint};

use super::constants::{DEFAULT_BEAT_LENGTH, DRUMROLL_FLAG};

/// 连打点成功判定后的缩放与淡出时长。
pub(crate) const DRUM_ROLL_TICK_FADE_MS: f64 = 200.0;
/// 小节线越过判定位置后的淡出时长。
pub(crate) const MEASURE_LINE_FADE_MS: f64 = 150.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DrumRollTick {
    pub(crate) time: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MeasureLine {
    pub(crate) time: f64,
}

/// 按 osu!taiko 的 `DrumRoll.CreateNestedHitObjects()` 规则生成连打点。
pub(crate) fn generate_drum_roll_ticks(
    hit_object: &TaikoHitObject,
    timing_points: &[TimingPoint],
    slider_tick_rate: f64,
) -> Vec<DrumRollTick> {
    if hit_object.hit_type & DRUMROLL_FLAG == 0 || hit_object.end_time < hit_object.start_time {
        return Vec::new();
    }

    let beat_length = timing_point_at(timing_points, hit_object.start_time as f64)
        .map(|point| point.beat_length)
        .unwrap_or(DEFAULT_BEAT_LENGTH);
    // lazer 仅为 SliderTickRate=3 保留三等分，其余值统一按四等分处理。
    let tick_rate = if slider_tick_rate == 3.0 { 3.0 } else { 4.0 };
    let tick_spacing = beat_length / tick_rate;
    if !tick_spacing.is_finite() || tick_spacing <= 0.0 {
        return Vec::new();
    }

    let limit = hit_object.end_time as f64 + tick_spacing / 2.0;
    let mut time = hit_object.start_time as f64;
    let mut ticks = Vec::new();
    while time < limit {
        ticks.push(DrumRollTick { time });
        time += tick_spacing;
    }
    ticks
}

/// 按 osu! 的 `BarLineGenerator` 生成每个红线区段的小节线。
pub(crate) fn generate_measure_lines(
    timing_points: &[TimingPoint],
    first_hit_time: i64,
    last_hit_time: i64,
) -> Vec<MeasureLine> {
    let redlines: Vec<&TimingPoint> = timing_points
        .iter()
        .filter(|point| point.uninherited)
        .collect();
    if redlines.is_empty() {
        return Vec::new();
    }

    let generation_start = first_hit_time.min(0) as f64;
    let last_hit_time = last_hit_time as f64 + 1.0;
    let mut lines = Vec::new();

    for (index, point) in redlines.iter().enumerate() {
        let meter = point.meter.max(1) as f64;
        let bar_length = point.beat_length * meter;
        if !bar_length.is_finite() || bar_length <= 0.0 {
            continue;
        }

        let end_time = redlines
            .get(index + 1)
            .map(|next| next.time)
            .unwrap_or(last_hit_time + bar_length);
        let mut time = if point.time > generation_start {
            point.time
        } else {
            point.time + ((generation_start - point.time) / bar_length).ceil() * bar_length
        };
        if point.omit_first_bar_line {
            time += bar_length;
        }

        // 损坏谱面的极端拍长不能让预览生成无限数量的物件。
        while time < end_time && lines.len() < 100_000 {
            let rounded = time.round();
            if (time - rounded).abs() <= 1e-7 {
                time = rounded;
            }
            lines.push(MeasureLine { time });
            time += bar_length;
        }
    }

    lines
}

/// 成功判定后沿用 `DrawableDrumRollTick` 的 `OutQuint` 动画。
pub(crate) fn drum_roll_tick_transform(tick_time: f64, snapshot_time: i64) -> (f64, f64) {
    let elapsed = snapshot_time as f64 - tick_time;
    if elapsed <= 0.0 {
        return (1.0, 1.0);
    }
    if elapsed >= DRUM_ROLL_TICK_FADE_MS {
        return (0.0, 1.4);
    }

    let progress = (elapsed / DRUM_ROLL_TICK_FADE_MS).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - progress).powi(5);
    (1.0 - eased, 1.0 + 0.4 * eased)
}

/// 小节线在开始时间之后线性淡出，行为对应 `DrawableBarLine`。
pub(crate) fn measure_line_alpha(line_time: f64, snapshot_time: i64) -> f64 {
    let elapsed = snapshot_time as f64 - line_time;
    if elapsed <= 0.0 {
        1.0
    } else {
        (1.0 - elapsed / MEASURE_LINE_FADE_MS).clamp(0.0, 1.0)
    }
}

fn timing_point_at(timing_points: &[TimingPoint], time: f64) -> Option<&TimingPoint> {
    let index = timing_points.partition_point(|point| point.time <= time);
    timing_points[..index]
        .iter()
        .rev()
        .find(|point| point.uninherited)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing(time: f64, beat_length: f64, meter: i32) -> TimingPoint {
        TimingPoint {
            time,
            beat_length,
            meter,
            uninherited: true,
            kiai_mode: false,
            omit_first_bar_line: false,
        }
    }

    fn drum_roll(start_time: i64, end_time: i64) -> TaikoHitObject {
        TaikoHitObject {
            start_time,
            end_time,
            hit_type: DRUMROLL_FLAG,
            hitsound: 0,
        }
    }

    #[test]
    fn drum_roll_ticks_use_three_or_four_divisions() {
        let point = timing(0.0, 600.0, 4);
        let three = generate_drum_roll_ticks(&drum_roll(0, 600), &[point], 3.0);
        assert_eq!(
            three.iter().map(|tick| tick.time).collect::<Vec<_>>(),
            vec![0.0, 200.0, 400.0, 600.0]
        );

        let four = generate_drum_roll_ticks(&drum_roll(0, 600), &[point], 1.0);
        assert_eq!(
            four.iter().map(|tick| tick.time).collect::<Vec<_>>(),
            vec![0.0, 150.0, 300.0, 450.0, 600.0]
        );
    }

    #[test]
    fn successful_tick_uses_out_quint_fade_and_scale() {
        assert_eq!(drum_roll_tick_transform(1000.0, 1000), (1.0, 1.0));
        let (alpha, scale) = drum_roll_tick_transform(1000.0, 1100);
        assert!((alpha - 0.03125).abs() < 1e-9);
        assert!((scale - 1.3875).abs() < 1e-9);
        assert_eq!(drum_roll_tick_transform(1000.0, 1200), (0.0, 1.4));
    }

    #[test]
    fn measure_lines_align_to_redlines_and_honor_omit_first() {
        let mut first = timing(-1000.0, 500.0, 4);
        first.omit_first_bar_line = true;
        let second = timing(5000.0, 250.0, 3);
        let lines = generate_measure_lines(&[first, second], 1000, 7000);
        let times: Vec<f64> = lines.iter().map(|line| line.time).collect();
        assert_eq!(times, vec![3000.0, 5000.0, 5750.0, 6500.0, 7250.0]);
    }

    #[test]
    fn measure_line_fades_after_crossing_judgement() {
        assert_eq!(measure_line_alpha(1000.0, 1000), 1.0);
        assert_eq!(measure_line_alpha(1000.0, 1075), 0.5);
        assert_eq!(measure_line_alpha(1000.0, 1150), 0.0);
    }
}
