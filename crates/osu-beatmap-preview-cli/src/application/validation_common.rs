//! 不读取文件内容的通用请求校验。

use super::source_osu;
use super::request::SourceOptions;
use osu_beatmap_preview_core::processing::validation as validate;
use osu_beatmap_preview_core::support::error::{PreviewError, Result};

/// 校验来源参数的格式；文件内容和 checksum 在来源加载阶段校验。
pub(crate) fn validate_source(source: &SourceOptions) -> Result<()> {
    if source.bid.is_empty() && source.osu_file.is_none() && source.osz_file.is_none() {
        return Err(PreviewError::new(
            "--bid is required unless --osu-file or --osz-file is provided",
        ));
    }
    if !source.bid.is_empty() {
        validate::validate_bid(&source.bid)?;
    }
    if let Some(path) = source.osu_file.as_deref() {
        source_osu::validate_local_path(path, "osu")?;
    }
    if let Some(path) = source.osz_file.as_deref() {
        source_osu::validate_local_path(path, "osz")?;
    }
    Ok(())
}
