//! OSR 在线来源的下载入口。
//!
//! 官方 OAuth、成绩选择和回放校验由 `osu_official` 维护；此文件只向应用层暴露
//! OSR 资源语义，避免上层依赖认证实现的目录结构。

pub(crate) use super::osu_official::scores::download;
pub(crate) use super::osu_official::{lookup, Lookup};
