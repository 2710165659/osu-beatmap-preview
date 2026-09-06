//! 视觉渲染模块：画布 / 文字 / 皮肤 / 输出编码器，以及四个游戏模式与视频封装。

pub(crate) mod canvas;
pub(crate) mod cpu;
pub(crate) mod geometry;
#[cfg(any(feature = "wgpu-renderer", test))]
pub(crate) mod scene;
pub(crate) mod text;
pub(crate) mod timing;
#[cfg(feature = "wgpu-renderer")]
pub(crate) mod wgpu;
