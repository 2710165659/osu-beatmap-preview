//! Standard 共享显示规则；后端仅负责执行绘制，不独立解释回放状态。

pub(crate) mod animation;
#[cfg(feature = "replay")]
pub mod replay;
pub mod style;
pub(crate) mod visibility;
