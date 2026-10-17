//! 与 CLI、库 API 共用的请求模型及第一阶段格式校验。

use osu_beatmap_preview_core::gameplay::GameplayOptions;
use osu_beatmap_preview_core::model::mods::ModSettings;
use osu_beatmap_preview_core::processing::validation as validate;
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
mod common;
mod osr;
mod osu;
pub use common::{ExecutionOptions, OutputOptions, SourceOptions, ViewOptions};
pub use osr::{ReplayMode, ReplaySource, ScoreSelection};
pub use osu::RulesetOptions;

#[derive(Debug, Clone)]
pub struct RenderRequest {
    pub kind: RequestKind,
    pub source: SourceOptions,
    pub ruleset: RulesetOptions,
    pub view: ViewOptions,
    pub output: OutputOptions,
    pub execution: ExecutionOptions,
    /// 预览叠加配置；回放执行由请求类型显式分派。
    pub gameplay: GameplayOptions,
}

impl RenderRequest {
    pub fn new(bid: impl Into<String>) -> Self {
        Self {
            kind: RequestKind::Osu,
            source: SourceOptions {
                bid: bid.into(),
                osu_file: None,
                osz_file: None,
            },
            ruleset: RulesetOptions::default(),
            view: ViewOptions::default(),
            output: OutputOptions::default(),
            execution: ExecutionOptions::default(),
            gameplay: GameplayOptions::default(),
        }
    }

    pub(crate) fn validate(self) -> Result<ValidatedRequest> {
        super::validation_osr::validate(&self)?;
        if matches!(self.kind, RequestKind::Osu) {
            super::validation_common::validate_source(&self.source)?;
        }
        // 单独的本地 `.osu` 没有媒体包，只允许 PNG / GIF；搭配 `.osz` 后可以生成 MP4。
        if matches!(self.kind, RequestKind::Osu)
            && self.source.osu_file.is_some()
            && self.source.osz_file.is_none()
            && self.output.format.as_deref() == Some("mp4")
        {
            return Err(PreviewError::new(
                "local .osu input supports only PNG or GIF output, not MP4 (video)",
            ));
        }
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
            Some(osu_beatmap_preview_core::model::mods::parse_mods(
                &self.ruleset.mods,
            )?)
        };
        if let Some(mods) = &mods {
            let errors = osu_beatmap_preview_core::model::mods::validate_mods(
                mods,
                None,
                Some(self.output.format.as_deref().unwrap_or("gif")),
            );
            if !errors.is_empty() {
                return Err(PreviewError::new(errors.join("; ")));
            }
        }
        Ok(ValidatedRequest {
            kind: self.kind,
            source: self.source,
            ruleset: ValidatedRulesetOptions {
                convert: self.ruleset.convert,
                mods,
            },
            view: self.view,
            output: self.output,
            execution: self.execution,
            gameplay: self.gameplay,
        })
    }
}

/// 一次请求只有一个分支；回放分支再以枚举约束三种互斥来源。
#[derive(Debug, Clone, Default)]
pub enum RequestKind {
    #[default]
    Osu,
    Osr {
        source: ReplaySource,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct ValidatedRequest {
    pub kind: RequestKind,
    pub source: SourceOptions,
    pub ruleset: ValidatedRulesetOptions,
    pub view: ViewOptions,
    pub output: OutputOptions,
    pub execution: ExecutionOptions,
    /// 预览叠加配置；回放执行由请求类型显式分派。
    #[allow(dead_code)]
    pub gameplay: GameplayOptions,
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
    fn realtime_only_mods_are_rejected_before_reading_or_downloading_maps() {
        for code in [
            "CL",
            "classic",
            "RX",
            "relax",
            "SO",
            "spun-out",
            "AP",
            "autopilot",
            "AL",
            "alternate",
            "SG",
            "single-tap",
            "MR",
            "mirror",
        ] {
            for fmt in [None, Some("png"), Some("gif"), Some("mp4")] {
                let mut request = RenderRequest::new("123");
                request.ruleset.mods = vec![code.into()];
                request.output.format = fmt.map(str::to_owned);
                assert!(
                    request
                        .validate()
                        .unwrap_err()
                        .to_string()
                        .contains("only supported for realtime"),
                    "{code}/{fmt:?}"
                );
            }
        }
        let mut request = RenderRequest::new("123");
        request.output.format = Some("realtime".into());
        assert!(request.validate().is_err());
        let mut request = RenderRequest::new("123");
        request.ruleset.mods = vec!["hard-rock".into(), "double-time1.25".into()];
        request.output.format = Some("gif".into());
        let mods = request.validate().unwrap().ruleset.mods.unwrap();
        assert!(mods.hard_rock);
        assert_eq!(mods.speed_multiplier, 1.25);
    }

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

    /// 来源格式校验允许省略本地输入的 bid，文件内容校验延后到读取阶段。
    #[test]
    fn local_sources_allow_bid_to_be_omitted() {
        let mut local_osu = RenderRequest::new("");
        local_osu.source.osu_file = Some("map.osu".to_string());
        local_osu.validate().unwrap();

        let mut osz_without_bid = RenderRequest::new("");
        osz_without_bid.source.osz_file = Some("map.osz".to_string());
        osz_without_bid.validate().unwrap();

        let mut osz_with_bid = RenderRequest::new("123");
        osz_with_bid.source.osz_file = Some("map.osz".to_string());
        osz_with_bid.validate().unwrap();

        // 没有本地文件时 bid 仍然是必填项。
        let error = RenderRequest::new("").validate().unwrap_err().to_string();
        assert!(error.contains("--bid is required"), "{error}");
        let mut non_numeric = RenderRequest::new("abc");
        non_numeric.source.osu_file = Some("map.osu".to_string());
        assert!(non_numeric.validate().is_err());
    }

    /// 本地 `.osu` 必须搭配媒体包才能输出 MP4，单独使用仍允许 GIF。
    #[test]
    fn local_osu_input_rejects_video_output() {
        let mut request = RenderRequest::new("");
        request.source.osu_file = Some("map.osu".to_string());
        request.output.format = Some("mp4".to_string());
        let error = request.validate().unwrap_err().to_string();
        assert!(error.contains("not MP4"), "{error}");

        let mut request = RenderRequest::new("");
        request.source.osu_file = Some("map.osu".to_string());
        request.output.format = Some("gif".to_string());
        request.validate().unwrap();

        let mut osz = RenderRequest::new("123");
        osz.source.osz_file = Some("map.osz".to_string());
        osz.output.format = Some("mp4".to_string());
        osz.clone().validate().unwrap();
        osz.source.osu_file = Some("map.osu".to_string());
        osz.validate().unwrap();
    }

    /// 本地文件只接受 .osu / .osz。
    #[test]
    fn local_paths_must_match_their_source_type() {
        let mut request = RenderRequest::new("123");
        request.source.osu_file = Some("song.mp3".to_string());
        let error = request.validate().unwrap_err().to_string();
        assert!(error.contains("--osu-file must be a .osu"), "{error}");

        let mut empty = RenderRequest::new("123");
        empty.source.osu_file = Some("  ".to_string());
        assert!(empty.validate().is_err());
        for (osu_file, osz_file) in [
            (Some("pack.osz"), None),
            (None, Some("map.osu")),
            (None, Some(" ")),
        ] {
            let mut request = RenderRequest::new("");
            request.source.osu_file = osu_file.map(str::to_string);
            request.source.osz_file = osz_file.map(str::to_string);
            assert!(request.validate().is_err());
        }
    }
}
