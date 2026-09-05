//! 与 CLI、库 API 共用的请求模型及第一阶段格式校验。

use crate::domain::errors::{PreviewError, Result};
use crate::domain::mods::ModSettings;
use crate::domain::validate::{self, TimePoint};

#[derive(Debug, Clone)]
pub struct RenderRequest {
    pub source: SourceOptions,
    pub ruleset: RulesetOptions,
    pub view: ViewOptions,
    pub output: OutputOptions,
    pub execution: ExecutionOptions,
}

impl RenderRequest {
    pub fn new(bid: impl Into<String>) -> Self {
        Self {
            source: SourceOptions { bid: bid.into() },
            ruleset: RulesetOptions::default(),
            view: ViewOptions::default(),
            output: OutputOptions::default(),
            execution: ExecutionOptions::default(),
        }
    }

    pub(crate) fn validate(self) -> Result<ValidatedRequest> {
        validate::validate_bid(&self.source.bid)?;
        if let Some(convert) = &self.ruleset.convert {
            validate::validate_convert_value(convert)?;
        }
        if let Some(format) = &self.output.format {
            validate::validate_fmt_value(format)?;
        }
        if let Some(duration) = self.view.duration_seconds {
            validate::validate_positive_finite("duration time", duration)?;
        }
        if let Some(scale) = self.output.scale {
            validate::validate_positive_finite("scale", scale)?;
        }
        if self
            .output
            .output_dir
            .as_deref()
            .is_some_and(|directory| directory.is_empty())
        {
            return Err(PreviewError::new("--output-dir must not be empty"));
        }
        if let Some(fps) = self.output.fps {
            validate_fps(fps)?;
        }
        let mods = if self.ruleset.mods.is_empty() {
            None
        } else {
            Some(crate::domain::mods::parse_mods(&self.ruleset.mods)?)
        };
        Ok(ValidatedRequest {
            source: self.source,
            ruleset: ValidatedRulesetOptions {
                convert: self.ruleset.convert,
                mods,
            },
            view: self.view,
            output: self.output,
            execution: self.execution,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SourceOptions {
    pub bid: String,
}

#[derive(Debug, Clone, Default)]
pub struct RulesetOptions {
    pub convert: Option<String>,
    pub mods: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ViewOptions {
    pub time_points: Vec<TimePoint>,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct OutputOptions {
    pub format: Option<String>,
    pub fps: Option<u32>,
    pub scale: Option<f64>,
    pub output_dir: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExecutionOptions {
    pub no_cache: bool,
    pub logging: bool,
    pub config: Option<String>,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            no_cache: false,
            logging: true,
            config: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ValidatedRequest {
    pub source: SourceOptions,
    pub ruleset: ValidatedRulesetOptions,
    pub view: ViewOptions,
    pub output: OutputOptions,
    pub execution: ExecutionOptions,
}

#[derive(Debug, Clone)]
pub(crate) struct ValidatedRulesetOptions {
    pub convert: Option<String>,
    pub mods: Option<ModSettings>,
}

pub fn parse_positive_finite(name: &str, raw: &str) -> Result<f64> {
    validate::parse_positive_finite(name, raw)
}

pub fn parse_fps(raw: &str) -> Result<u32> {
    let fps = raw.parse::<u32>().map_err(|_| {
        PreviewError::new(format!(
            "--fps must be an integer from 1 to 60, got '{raw}'"
        ))
    })?;
    validate_fps(fps)?;
    Ok(fps)
}

fn validate_fps(fps: u32) -> Result<()> {
    if !(1..=60).contains(&fps) {
        return Err(PreviewError::new(format!(
            "--fps must be between 1 and 60, got {fps}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_validation_rejects_invalid_fps_before_business_planning() {
        for raw in ["0", "61", "1.5", "abc"] {
            assert!(parse_fps(raw).is_err(), "{raw} 应被拒绝");
        }
        assert_eq!(parse_fps("60").unwrap(), 60);
    }

    #[test]
    fn nested_request_is_validated_through_one_entry_point() {
        let mut request = RenderRequest::new("123");
        request.ruleset.mods = vec!["HD".to_string()];
        request.output.format = Some("gif".to_string());
        request.output.scale = Some(1.5);
        request.output.fps = Some(30);
        let validated = request.validate().unwrap();
        assert!(validated.ruleset.mods.unwrap().hidden);
    }

    #[test]
    fn empty_output_directory_is_rejected() {
        let mut request = RenderRequest::new("123");
        request.output.output_dir = Some(String::new());
        let error = request.validate().expect_err("空输出目录必须被拒绝");
        assert!(error.to_string().contains("--output-dir must not be empty"));
    }
}
