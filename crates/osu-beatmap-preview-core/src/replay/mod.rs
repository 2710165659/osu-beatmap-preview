//! OSR 回放领域：文档、兼容性策略、Standard/Taiko/Catch 判定准备与只读输入时间线。
//!
//! 各模式的计算与谱面准备分别组织，共享回放文档与输入时间线。

pub mod calculate;
pub mod compatibility;
pub mod model;
pub mod mods;
pub(crate) mod normalize;
pub mod preparation;
pub mod timeline;
pub use crate::domain::parser::osr::parse_osr;
pub use model::{ReplayDocument, ReplayFrame, ReplayHeader, ReplayMetadata};
pub use timeline::ReplayTimeline;
