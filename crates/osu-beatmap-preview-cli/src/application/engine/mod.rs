//! 单次渲染请求的应用服务入口。

use crate::application::request::RenderRequest;
use osu_beatmap_preview_core::domain::errors::{PreviewError, Result};

pub(crate) mod legacy;

pub(crate) fn execute(request: RenderRequest) -> Result<serde_json::Value> {
    let validated = request.validate()?;
    crate::infrastructure::config::initialize(
        validated.execution.config.as_deref(),
        validated.output.scale,
    )
    .map_err(|error| PreviewError::new(format!("configuration error: {error}")))?;
    if validated.execution.logging {
        crate::infrastructure::logging::config::init();
    }
    legacy::generate_preview(validated)
}
