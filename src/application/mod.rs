//! 应用层：接收请求、生成不可变渲染计划并编排基础设施与渲染器。

pub(crate) mod artifact;
pub(crate) mod engine;
pub(crate) mod plan;
pub(crate) mod ports;
pub mod request;

pub use request::{
    ExecutionOptions, OutputOptions, RenderRequest, RulesetOptions, SourceOptions, ViewOptions,
};
