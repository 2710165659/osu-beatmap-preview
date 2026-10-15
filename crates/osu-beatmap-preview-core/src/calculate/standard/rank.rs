//! Lazer 评级按 ACC 阈值计算，存在普通 Miss 时最高 A；未判定不展示评级。
pub fn rank(accuracy: f64, misses: u32) -> &'static str {
    if accuracy == 1.0 && misses == 0 {
        "X"
    } else if accuracy >= 0.95 && misses == 0 {
        "S"
    } else if accuracy >= 0.9 {
        "A"
    } else if accuracy >= 0.8 {
        "B"
    } else if accuracy >= 0.7 {
        "C"
    } else {
        "D"
    }
}
