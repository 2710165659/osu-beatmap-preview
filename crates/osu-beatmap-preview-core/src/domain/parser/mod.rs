//! 谱面与回放格式解析门面；不执行资源获取和规则判定。

mod osu;
pub use osu::*;
#[cfg(feature = "replay")]
pub mod osr;
