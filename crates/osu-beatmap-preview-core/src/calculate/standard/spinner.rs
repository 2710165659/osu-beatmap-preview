//! 转盘方向反转必须先回到当前圈起点；不能用绝对角度累加作弊。
use crate::gameplay::events::HitResult;
#[derive(Default)]
pub struct Rotation {
    accumulated: f32,
    completed_at: f32,
    peak: f32,
    spins: u32,
    pub angle: f32,
}
impl Rotation {
    pub fn add(&mut self, delta: f32) {
        self.angle += delta;
        self.accumulated += delta;
        let current = self.accumulated - self.completed_at;
        self.peak = self.peak.max(current.abs());
        if self.peak >= 360.0 {
            self.spins += 1;
            self.completed_at += current.signum() * 360.0;
            self.peak = (self.accumulated - self.completed_at).abs();
        }
    }
    pub fn total(&self) -> f32 {
        self.spins as f32 * 360.0 + self.peak
    }
}
pub fn required(duration: f64, od: f64) -> (u32, u32) {
    let clear = if od > 5.0 {
        150.0 + (od - 5.0) * 15.0
    } else {
        90.0 + od * 12.0
    };
    let complete = if od > 5.0 {
        380.0 + (od - 5.0) * 10.0
    } else {
        250.0 + od * 26.0
    };
    let minimum = (clear / 60.0 * duration / 1000.0 + 0.0001) as u32;
    let maximum = (complete / 60.0 * duration / 1000.0 + 0.0001) as u32;
    (minimum, maximum.max(minimum + 2))
}
pub fn result(rotation: f32, required: u32) -> HitResult {
    let progress = if required == 0 {
        1.0
    } else {
        rotation / 360.0 / required as f32
    };
    if progress >= 1.0 {
        HitResult::Great
    } else if progress > 0.9 {
        HitResult::Ok
    } else if progress > 0.75 {
        HitResult::Meh
    } else {
        HitResult::Miss
    }
}
