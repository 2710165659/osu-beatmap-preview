//! 桌面 GUI 平台适配骨架。
//!
//! 具体窗口库、输入系统和音频后端由桌面应用决定；本 crate 只固定与渲染引擎
//! 对接所需的生命周期边界，避免把平台句柄带入 core 或 renderer。

use std::sync::Arc;

use osu_beatmap_preview_core::{FrameScene, PreviewError, Result};

/// 桌面宿主为渲染器提供的目标 surface。
pub trait SurfaceHost {
    /// 返回当前窗口尺寸，尺寸为零时宿主应等待下一次 resize。
    fn size(&self) -> (u32, u32);
    /// 在宿主完成 surface 获取后提交一帧场景。
    fn present(&mut self, scene: &FrameScene) -> Result<()>;
}

/// GUI 播放会话的最小生命周期接口，供具体窗口应用组合输入和音频时钟。
pub trait PlaybackController {
    fn seek(&mut self, time_ms: i64) -> Result<()>;
    fn set_playing(&mut self, playing: bool);
    fn render(&mut self, time_ms: i64) -> Result<FrameScene>;
}

/// 预留给桌面实现共享的线程安全渲染句柄。
pub type SharedScene = Arc<FrameScene>;

/// 将平台错误转换为统一核心错误，避免 GUI 层泄漏具体窗口库类型。
pub fn platform_error(message: impl Into<String>) -> PreviewError {
    PreviewError::render(message.into())
}
