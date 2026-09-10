//! 历史内部模块集合。
//!
//! 这里仍保留解析、规则转换和基础工具的实现路径，以兼容现有宿主；新的外部调用
//! 应使用 crate 根部的 [`crate::api`]、[`crate::model`] 和 [`crate::processing`]。

pub mod build_time;
pub mod errors;
pub mod models;
pub mod mods;
pub mod parser;
pub mod rulesets;
pub mod shared;
pub mod timeout;
pub mod validate;
