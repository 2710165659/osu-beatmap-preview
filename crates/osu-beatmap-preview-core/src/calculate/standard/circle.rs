//! Lazer 命中窗采用向下取整减 0.5ms；超时与点击边界分别处理。
use crate::gameplay::events::HitResult;
pub fn windows(od: f64) -> [f64; 3] {
    [
        (80.0 - 6.0 * od).floor() - 0.5,
        (140.0 - 8.0 * od).floor() - 0.5,
        (200.0 - 10.0 * od).floor() - 0.5,
    ]
}
pub fn result(error: f64, od: f64) -> Option<HitResult> {
    let windows = windows(od);
    let absolute = error.abs();
    if absolute <= windows[0] {
        Some(HitResult::Great)
    } else if absolute <= windows[1] {
        Some(HitResult::Ok)
    } else if absolute <= windows[2] {
        Some(HitResult::Meh)
    } else if (0.0..=400.0).contains(&error) {
        Some(HitResult::Miss)
    } else {
        None
    }
}
pub fn in_range(cursor: [f32; 2], target: [f32; 2], radius: f32) -> bool {
    (cursor[0] - target[0]).powi(2) + (cursor[1] - target[1]).powi(2) <= radius * radius
}
