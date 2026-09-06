//! 运行时无关的实时场景与 WGPU 离屏渲染 API。

mod session;
mod types;

pub use crate::render::scene::FrameScene;
pub use crate::render::wgpu::OffscreenRenderer;
pub use session::RealtimeSession;
pub use types::{
    CancellationToken, FrameRequest, MediaPolicy, OffscreenConfig, RealtimeError, RealtimeMode,
    RealtimeRequest, RenderedFrame, RendererInfo, RgbaFrame, TimelineInfo,
};

pub(crate) fn configured_offscreen(mode: crate::render::geometry::GameMode) -> OffscreenConfig {
    let _ = mode;
    crate::infrastructure::config::wgpu_config(crate::infrastructure::config::current())
}

#[cfg(feature = "wgpu-player")]
pub mod player;
