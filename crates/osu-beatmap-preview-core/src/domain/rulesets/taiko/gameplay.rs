//! 太鼓命中窗口、滚奏节点和转盘次数；判定、绘制与音频共享同一份规则。

use crate::domain::models::{TaikoHitObject, TimingPoint};

pub fn difficulty_range(value: f64, low: f64, middle: f64, high: f64) -> f64 {
    if value > 5.0 {
        middle + (high - middle) * (value - 5.0) / 5.0
    } else {
        middle + (middle - low) * (value - 5.0) / 5.0
    }
}

pub fn hit_windows(od: f64) -> [f64; 3] {
    [(50.0, 35.0, 20.0), (120.0, 80.0, 50.0), (135.0, 95.0, 70.0)]
        .map(|(low, middle, high)| difficulty_range(od, low, middle, high).floor() - 0.5)
}

pub fn required_swell_hits(duration_ms: i64, od: f64) -> u32 {
    let od = if od.is_finite() { od } else { 5.0 };
    (duration_ms.max(0) as f64 / 1000.0 * difficulty_range(od, 3.0, 5.0, 7.5) * 1.65).max(1.0)
        as u32
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumRollTick {
    pub time: f64,
}

pub fn drum_roll_tick_spacing(object: &TaikoHitObject, points: &[TimingPoint], rate: f64) -> f64 {
    let beat = points
        .iter()
        .rev()
        .find(|p| p.uninherited && p.time <= object.start_time as f64)
        .or_else(|| points.iter().find(|p| p.uninherited))
        .map_or(500.0, |p| p.beat_length);
    beat / if rate == 3.0 { 3.0 } else { 4.0 }
}

pub fn generate_drum_roll_ticks(
    object: &TaikoHitObject,
    points: &[TimingPoint],
    rate: f64,
) -> Vec<DrumRollTick> {
    if object.hit_type & 2 == 0 || object.end_time < object.start_time {
        return Vec::new();
    }
    let spacing = drum_roll_tick_spacing(object, points, rate);
    if !spacing.is_finite() || spacing <= 0.0 {
        return Vec::new();
    }
    let mut time = object.start_time as f64;
    let limit = object.end_time as f64 + spacing / 2.0;
    let mut ticks = Vec::new();
    while time < limit && ticks.len() < 200_000 {
        ticks.push(DrumRollTick { time });
        time += spacing;
    }
    ticks
}
