//! Standard 逻辑准备：堆叠、难度、滑条路径与嵌套节点均不依赖渲染。
#[cfg(feature = "replay")]
mod objects;
pub use crate::domain::rulesets::standard as stacking;
#[cfg(feature = "replay")]
pub use objects::*;

/// HR/Mirror 先翻转物件和滑条控制点，再计算堆叠；录制的光标已经处于游玩坐标。
pub use crate::domain::rulesets::standard::apply_object_mods;
