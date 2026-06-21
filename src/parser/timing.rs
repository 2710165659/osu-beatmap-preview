//! Timing point and break-period parsing.

use crate::models::{BreakPeriod, TimingPoint};

/// Parse `[TimingPoints]` lines into a sorted `Vec<TimingPoint>`.
/// Returns `None` if the section is empty.
pub fn parse_timing_points(lines: &[&str]) -> Option<Vec<TimingPoint>> {
    let mut points: Vec<TimingPoint> = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 2 {
            continue;
        }
        let mut meter = if parts.len() > 2 && !parts[2].is_empty() {
            parts[2].parse::<i32>().ok()?
        } else {
            4
        };
        if meter <= 0 {
            meter = 4;
        }
        let uninherited = parts.len() < 7 || parts[6] == "1";
        let effects = if parts.len() > 7 && !parts[7].is_empty() {
            parts[7].parse::<i32>().ok()?
        } else {
            0
        };
        points.push(TimingPoint {
            time: parts[0].parse().ok()?,
            beat_length: parts[1].parse().ok()?,
            meter,
            uninherited,
            kiai_mode: effects & 1 != 0,
        });
    }
    // Stable sort keeps file order for equal times (red/green at same time).
    points.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
    if points.is_empty() {
        return None;
    }
    Some(points)
}

/// Parse break periods from `[Events]` lines (type-2 events).
pub fn parse_break_periods(lines: Option<&Vec<&str>>) -> Vec<BreakPeriod> {
    let Some(lines) = lines else {
        return Vec::new();
    };
    let mut breaks = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 3 || parts[0] != "2" {
            continue;
        }
        let (Ok(s), Ok(e)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) else {
            continue;
        };
        let (start_time, end_time) = (s as i64, e as i64);
        if end_time > start_time {
            breaks.push(BreakPeriod {
                start_time,
                end_time,
            });
        }
    }
    breaks
}
