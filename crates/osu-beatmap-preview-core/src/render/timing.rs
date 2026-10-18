//! 各模式共用的节奏标签辅助函数。

use crate::domain::models::TimingPoint;

/// 返回指定时间生效的非继承 BPM；首条红线之前的静音段沿用首条红线 BPM。
pub fn bpm_at(timing_points: &[TimingPoint], time: i64) -> Option<f64> {
    let mut first = None;
    let mut active = None;

    for point in timing_points.iter().filter(|point| {
        point.uninherited && point.beat_length.is_finite() && point.beat_length > 0.0
    }) {
        let bpm = 60_000.0 / point.beat_length;
        first.get_or_insert(bpm);
        if point.time <= time as f64 {
            active = Some(bpm);
        } else {
            break;
        }
    }

    active.or(first)
}

pub fn format_bpm(bpm: f64) -> String {
    if (bpm - bpm.round()).abs() < 0.05 {
        format!("{:.0} BPM", bpm)
    } else {
        format!("{:.1} BPM", bpm)
    }
}

/// 背景视频在两端各用 500ms 淡入淡出，短视频取两者的较低可见度。
pub const VIDEO_FADE_MS: i64 = 500;

pub fn video_visibility_alpha(video_ms: i64, duration_ms: i64) -> f64 {
    if video_ms < 0 || video_ms >= duration_ms.max(1) {
        return 0.0;
    }
    let fade_in = video_ms as f64 / VIDEO_FADE_MS as f64;
    let fade_out = (duration_ms - video_ms) as f64 / VIDEO_FADE_MS as f64;
    fade_in.min(fade_out).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(time: f64, beat_length: f64) -> TimingPoint {
        TimingPoint {
            time,
            beat_length,
            meter: 4,
            uninherited: true,
            kiai_mode: false,
            omit_first_bar_line: false,
            sample_set: 0,
            sample_index: 0,
            sample_volume: 100,
        }
    }

    #[test]
    fn bpm_lookup_carries_first_redline_through_leading_silence() {
        let points = [point(1_000.0, 500.0), point(3_000.0, 250.0)];
        assert_eq!(bpm_at(&points, 0), Some(120.0));
        assert_eq!(bpm_at(&points, 2_000), Some(120.0));
        assert_eq!(bpm_at(&points, 3_000), Some(240.0));
    }

    #[test]
    fn bpm_format_keeps_meaningful_fraction() {
        assert_eq!(format_bpm(180.0), "180 BPM");
        assert_eq!(format_bpm(179.94), "179.9 BPM");
    }
}
