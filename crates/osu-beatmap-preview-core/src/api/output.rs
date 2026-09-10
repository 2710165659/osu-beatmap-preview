//! 实时预览会话产生的模式和时间线信息。

/// 谱面在会话中使用的游戏模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeMode {
    Standard,
    Taiko,
    Catch,
    Mania,
}

/// 会话时间轴的边界和速度信息，单位为毫秒。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineInfo {
    pub first_object_ms: i64,
    pub last_object_ms: i64,
    pub absolute_start_ms: i64,
    pub duration_ms: i64,
    pub beatmap_speed: f64,
}
