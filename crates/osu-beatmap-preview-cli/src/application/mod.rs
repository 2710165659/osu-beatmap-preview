//! 应用层：接收请求、生成不可变渲染计划并编排配置、资源与渲染器。

pub(crate) mod artifact;
mod execute;
pub(crate) mod legacy;
pub(crate) mod plan;
pub(crate) mod ports;
pub mod request;

pub use request::{ExecutionOptions, OutputOptions, RulesetOptions, SourceOptions, ViewOptions};

pub(crate) use execute::execute;
