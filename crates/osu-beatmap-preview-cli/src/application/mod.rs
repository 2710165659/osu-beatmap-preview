//! 应用层：接收请求、生成不可变渲染计划并编排配置、资源与渲染器。

pub(crate) mod artifact;
mod execute;
pub(crate) mod legacy;
mod osr;
pub(crate) mod output_plan;
pub mod request;
mod selection;
mod source_osr;
mod source_osu;
mod validation_common;
mod validation_osr;

pub use request::{ExecutionOptions, OutputOptions, RulesetOptions, SourceOptions, ViewOptions};

pub(crate) use execute::execute;
