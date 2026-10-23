//! 各模式回放共用的字体、HUD 与 CPU/WGPU 叠加层契约。
pub(crate) mod feedback;
pub mod font;
pub(crate) mod hud;
pub(crate) use super::draw as overlay;
