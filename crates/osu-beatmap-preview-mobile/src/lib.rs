//! Android/iOS 平台适配骨架。
//!
//! 移动端应用负责创建 native surface、提供音频时钟并管理生命周期；渲染引擎
//! 只接收宿主准备好的 WGPU 设备和目标视图。

use osu_beatmap_preview_core::{FrameScene, Result};

/// 移动端 surface 生命周期回调。
pub trait SurfaceLifecycle {
    fn resize(&mut self, width: u32, height: u32) -> Result<()>;
    fn draw(&mut self, scene: &FrameScene) -> Result<()>;
    fn suspend(&mut self);
    fn resume(&mut self) -> Result<()>;
}

/// 移动端音频实现向渲染会话提供的单调时钟。
pub trait AudioClock {
    fn position_ms(&self) -> i64;
    fn is_playing(&self) -> bool;
}

/// 统一移动端输入事件，具体触摸手势由应用层转换。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputEvent {
    Pointer { x: f32, y: f32, pressed: bool },
    Seek { time_ms: i64 },
    TogglePlayback,
}
