//! 回放文档、兼容性策略与只读输入时间线。

pub mod compatibility;
pub mod model;
pub mod mods;
pub(crate) mod normalize;
pub mod timeline;
pub use crate::domain::parser::osr::parse_osr;
pub use model::{ReplayDocument, ReplayFrame, ReplayHeader, ReplayMetadata};
pub use timeline::ReplayTimeline;
