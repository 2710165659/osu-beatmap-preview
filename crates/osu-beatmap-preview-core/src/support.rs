//! 运行支持能力的公开门面。
//!
//! 这些类型服务于错误处理、请求生命周期等宿主集成需求，但不属于谱面数据模型
//! 或具体的渲染算法。

pub mod error {
    pub use crate::domain::errors::{ErrorKind, PreviewError, Result};
}

pub mod timeout {
    pub use crate::domain::timeout::RequestDeadline;
}

pub mod build {
    pub use crate::domain::build_time::build_time;
}
