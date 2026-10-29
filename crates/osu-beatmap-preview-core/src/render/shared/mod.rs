//! CPU 与 WGPU 共用的渲染规则、布局和显示数据。

#[path = "replay/blend.rs"]
pub(crate) mod blend;
#[path = "replay/overlay.rs"]
pub(crate) mod draw;
#[cfg(feature = "replay")]
pub mod replay;
pub mod standard;
#[cfg(feature = "replay")]
pub mod taiko;

#[cfg(feature = "replay")]
pub mod catch;
