//! 渲染产物文件名；请求差异在文件名中表达，不参与配置目录哈希。

use crate::application::plan::{OutputFormat, RenderPlan};
use crate::domain::mods::ModSettings;
use crate::domain::validate::TimePoint;

pub(crate) struct ArtifactName(String);

impl ArtifactName {
    pub(crate) fn from_plan(plan: &RenderPlan) -> Self {
        let mut parts = vec![mode_name(plan.target_mode).to_string(), plan.bid.clone()];
        if plan.convert_used {
            parts.push("convert".to_string());
        }
        if let Some(mods) = &plan.mods {
            if mods.has_any_mod() {
                parts.push(format_mod_suffix(mods));
            }
        }
        if plan.format == OutputFormat::Mp4 {
            if has_explicit_video_time_options(&plan.time_points, plan.duration_seconds) {
                parts.push(format_video_time_suffix(
                    plan.time_points.first().copied(),
                    plan.duration_seconds,
                ));
            }
        } else {
            if !plan.time_points.is_empty() {
                parts.push(format_time_points_suffix(&plan.time_points));
            }
            if plan.format == OutputFormat::Gif {
                if let Some(duration) = plan.duration_seconds {
                    parts.push(format_duration_suffix(duration));
                }
            }
        }
        if let Some(fps) = plan.fps {
            parts.push(format!("fps{fps}"));
        }
        let scale = plan
            .requested_scale
            .map(format_scale_suffix)
            .unwrap_or_default();
        Self(format!(
            "{}{}.{}",
            parts.join("_"),
            scale,
            plan.format.as_str()
        ))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

fn mode_name(mode: i32) -> &'static str {
    match mode {
        0 => "standard",
        1 => "taiko",
        2 => "catch",
        3 => "mania",
        _ => "unknown",
    }
}

fn format_mod_suffix(mods: &ModSettings) -> String {
    mods.tokens.join("-").to_lowercase()
}

fn format_time_points_suffix(points: &[TimePoint]) -> String {
    let values = points
        .iter()
        .map(|point| match point {
            TimePoint::Seconds(value) => sanitize_suffix(&value.to_string()),
            TimePoint::Preview => "preview".to_string(),
        })
        .collect::<Vec<_>>();
    format!("time-points{}", values.join("-"))
}

fn format_video_time_suffix(start: Option<TimePoint>, duration: Option<f64>) -> String {
    let start = match start.unwrap_or(TimePoint::Seconds(0.0)) {
        TimePoint::Seconds(value) => value.to_string(),
        TimePoint::Preview => "preview".to_string(),
    };
    format!(
        "video-start{}-duration{}",
        sanitize_suffix(&start),
        sanitize_suffix(&duration.unwrap_or(600.0).to_string())
    )
}

fn format_duration_suffix(duration: f64) -> String {
    format!("duration{}", sanitize_suffix(&duration.to_string()))
}

fn format_scale_suffix(scale: f64) -> String {
    format!("@{}x", sanitize_suffix(&scale.to_string()))
}

fn sanitize_suffix(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn has_explicit_video_time_options(points: &[TimePoint], duration: Option<f64>) -> bool {
    !points.is_empty() || duration.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::request::RenderRequest;

    fn plan(fps: Option<u32>, scale: Option<f64>, no_cache: bool, logging: bool) -> RenderPlan {
        let mut request = RenderRequest::new("123");
        request.output.format = Some("gif".to_string());
        request.output.fps = fps;
        request.output.scale = scale;
        request.execution.no_cache = no_cache;
        request.execution.logging = logging;
        RenderPlan::build(request.validate().unwrap(), 0, false).unwrap()
    }

    #[test]
    fn visual_request_options_change_file_name() {
        assert_eq!(
            ArtifactName::from_plan(&plan(None, None, false, true)).as_str(),
            "standard_123.gif"
        );
        assert_eq!(
            ArtifactName::from_plan(&plan(Some(30), None, false, true)).as_str(),
            "standard_123_fps30.gif"
        );
        assert_eq!(
            ArtifactName::from_plan(&plan(None, Some(1.5), false, true)).as_str(),
            "standard_123@1.5x.gif"
        );
        assert_eq!(
            ArtifactName::from_plan(&plan(None, Some(1.0), false, true)).as_str(),
            "standard_123@1x.gif"
        );
    }

    #[test]
    fn existing_time_suffixes_remain_compatible() {
        let mut gif_request = RenderRequest::new("123");
        gif_request.output.format = Some("gif".to_string());
        gif_request.view.time_points = vec![TimePoint::Seconds(12.5), TimePoint::Preview];
        gif_request.view.duration_seconds = Some(3.5);
        let gif_plan = RenderPlan::build(gif_request.validate().unwrap(), 0, false).unwrap();
        assert_eq!(
            ArtifactName::from_plan(&gif_plan).as_str(),
            "standard_123_time-points12.5-preview_duration3.5.gif"
        );

        let mut video_request = RenderRequest::new("123");
        video_request.output.format = Some("mp4".to_string());
        video_request.view.time_points = vec![TimePoint::Preview];
        let video_plan = RenderPlan::build(video_request.validate().unwrap(), 0, false).unwrap();
        assert_eq!(
            ArtifactName::from_plan(&video_plan).as_str(),
            "standard_123_video-startpreview-duration600.mp4"
        );
    }

    #[test]
    fn execution_options_do_not_change_file_name() {
        assert_eq!(
            ArtifactName::from_plan(&plan(None, None, false, true)).as_str(),
            ArtifactName::from_plan(&plan(None, None, true, false)).as_str()
        );
    }
}
