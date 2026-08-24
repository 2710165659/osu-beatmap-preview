//! Shared timing-label helpers used by all mode renderers.

use crate::core::models::TimingPoint;

pub(crate) const BPM_LABEL_COLOR: [u8; 4] = [255, 82, 82, 255];

/// Return the active uninherited BPM at `time`, carrying the first red line
/// backwards through leading silence just like osu! does.
pub(crate) fn bpm_at(timing_points: &[TimingPoint], time: i64) -> Option<f64> {
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

pub(crate) fn format_bpm(bpm: f64) -> String {
    if (bpm - bpm.round()).abs() < 0.05 {
        format!("{:.0} BPM", bpm)
    } else {
        format!("{:.1} BPM", bpm)
    }
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
