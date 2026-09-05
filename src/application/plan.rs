//! 第二阶段业务校验及渲染计划。

use crate::application::request::ValidatedRequest;
use crate::domain::errors::{PreviewError, Result};
use crate::domain::mods::ModSettings;
use crate::domain::validate::{self, TimePoint, ValidateContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Png,
    Gif,
    Mp4,
}

impl OutputFormat {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Gif => "gif",
            Self::Mp4 => "mp4",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RenderPlan {
    pub bid: String,
    pub target_mode: i32,
    pub convert_used: bool,
    pub mods: Option<ModSettings>,
    pub time_points: Vec<TimePoint>,
    pub duration_seconds: Option<f64>,
    pub format: OutputFormat,
    pub fps: Option<u32>,
    pub requested_scale: Option<f64>,
    pub no_cache: bool,
}

impl RenderPlan {
    pub(crate) fn build(
        request: ValidatedRequest,
        target_mode: i32,
        convert_used: bool,
    ) -> Result<Self> {
        let format = resolve_format(request.output.format.as_deref(), target_mode);
        let format_name = format.as_str();
        if request.output.fps.is_some() && format == OutputFormat::Png {
            return Err(PreviewError::new(
                "--fps is only valid for GIF or MP4 output",
            ));
        }
        let context = ValidateContext {
            bid: &request.source.bid,
            fmt: format_name,
            target_mode,
        };
        let mods = validate::validate_with_context(
            &context,
            &request.view.time_points,
            request.view.duration_seconds,
            request.ruleset.mods,
        )?;
        Ok(Self {
            bid: request.source.bid,
            target_mode,
            convert_used,
            mods,
            time_points: request.view.time_points,
            duration_seconds: request.view.duration_seconds,
            format,
            fps: request.output.fps,
            requested_scale: request.output.scale,
            no_cache: request.execution.no_cache,
        })
    }
}

fn resolve_format(format: Option<&str>, target_mode: i32) -> OutputFormat {
    match format {
        Some("png") => OutputFormat::Png,
        Some("gif") => OutputFormat::Gif,
        Some("mp4") => OutputFormat::Mp4,
        None if target_mode == 0 => OutputFormat::Gif,
        None => OutputFormat::Png,
        Some(_) => unreachable!("输出格式已经通过第一阶段校验"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::request::RenderRequest;

    #[test]
    fn business_validation_runs_after_target_mode_is_known() {
        let mut request = RenderRequest::new("123");
        request.output.format = Some("png".to_string());
        request.view.duration_seconds = Some(2.0);
        assert!(RenderPlan::build(request.validate().unwrap(), 0, false).is_err());
    }

    #[test]
    fn default_format_depends_on_resolved_mode() {
        let standard =
            RenderPlan::build(RenderRequest::new("1").validate().unwrap(), 0, false).unwrap();
        let mania =
            RenderPlan::build(RenderRequest::new("1").validate().unwrap(), 3, false).unwrap();
        assert_eq!(standard.format, OutputFormat::Gif);
        assert_eq!(mania.format, OutputFormat::Png);
    }

    #[test]
    fn fps_is_rejected_when_resolved_output_is_static() {
        let mut request = RenderRequest::new("1");
        request.output.format = Some("png".to_string());
        request.output.fps = Some(30);
        assert!(RenderPlan::build(request.validate().unwrap(), 0, false).is_err());

        let mut implicit_mania = RenderRequest::new("1");
        implicit_mania.output.fps = Some(30);
        assert!(RenderPlan::build(implicit_mania.validate().unwrap(), 3, false).is_err());
    }
}
