//! 输入快照与输入来源契约。

/// 某一时刻的输入快照。
///
/// `cursor` 使用 osu! 游玩区域坐标（512×384），键盘玩法可以没有光标。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct InputSnapshot {
    pub cursor: Option<[f32; 2]>,
    /// 当前按住的所有键，位含义由 ruleset 定义。
    pub keys: u32,
    /// 这一帧相对上一帧新按下的键。
    pub pressed: u32,
    /// 这一帧松开的键。
    pub released: u32,
}

/// 输入来源：回放时间轴、实况输入，或没有输入。
pub trait InputSource: Send + Sync {
    /// 返回指定谱面绝对时刻的输入快照。
    fn snapshot_at(&self, absolute_ms: i64) -> Option<InputSnapshot>;
}
