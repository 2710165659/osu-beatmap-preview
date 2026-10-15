//! Standard 逻辑准备：堆叠、难度、滑条路径与嵌套节点均不依赖渲染。
#[cfg(feature = "replay")]
mod objects;
pub mod stacking;
#[cfg(feature = "replay")]
pub use objects::*;
