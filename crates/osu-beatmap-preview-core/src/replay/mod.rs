//! OSR 回放领域：文档、兼容性策略、Standard 判定准备与只读输入时间线。
//!
//! 当前只实现原生 osu!standard 回放；其它 ruleset 仍由普通谱面渲染链路处理，
//! 因此 Standard 专用计算和准备代码集中在这里，而不会伪装成四种模式共用实现。

pub mod compatibility;
pub mod calculate;
pub mod model;
pub mod mods;
pub(crate) mod normalize;
pub mod preparation;
pub mod timeline;
pub use crate::domain::parser::osr::parse_osr;
pub use model::{ReplayDocument, ReplayFrame, ReplayHeader, ReplayMetadata};
pub use timeline::ReplayTimeline;
