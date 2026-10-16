//! 游玩契约与不可变回放结果；普通预览不构造判定引擎。
mod contracts;
pub mod input;
pub mod overlay;
pub use contracts::*;
pub use input::{InputSnapshot, InputSource};
pub use overlay::{GameplayOverlay, OverlayStyle};
#[cfg(feature = "replay")]
pub mod engine;
#[cfg(feature = "replay")]
pub mod events;
#[cfg(feature = "replay")]
pub mod state;
#[cfg(feature = "replay")]
pub mod timeline;
