//! WGPU 离屏与实时显示后端。

pub(crate) mod composition;
pub(crate) mod modes;
mod offscreen;
mod rasterizer;
mod realtime;

pub use offscreen::OffscreenRenderer;
pub(crate) use realtime::RealtimeFrameSource;
