//! 跟随区有迟滞：重新进入必须靠近球，持续追踪允许扩大至 2.4 倍半径。
use super::circle::in_range;
pub fn tracking(cursor: [f32; 2], ball: [f32; 2], radius: f32, held: bool, previous: bool) -> bool {
    held && in_range(cursor, ball, radius * if previous { 2.4 } else { 1.0 })
}
