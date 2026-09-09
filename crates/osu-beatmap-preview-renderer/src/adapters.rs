//! 各宿主平台可复用的生命周期接口骨架。

/// 平台窗口或 Canvas 的尺寸变化通知。
pub trait SurfaceHost {
    fn resize(&mut self, width: u32, height: u32);
}
/// 平台输入事件的最小抽象，具体 GUI/移动端自行映射。
pub trait InputHost {
    fn pointer_down(&mut self, x: f32, y: f32);
    fn key_down(&mut self, key: &str);
}
/// 音频时钟由平台实现，核心只消费绝对毫秒时间。
pub trait AudioClock {
    fn position_ms(&self) -> i64;
    fn playing(&self) -> bool;
}
