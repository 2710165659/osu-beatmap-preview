//! 与 CLI、库 API 共用的请求模型及第一阶段格式校验。

use crate::application::local::{self, LocalInputKind};
use osu_beatmap_preview_core::gameplay::GameplayOptions;
use osu_beatmap_preview_core::model::mods::ModSettings;
use osu_beatmap_preview_core::processing::validation::{self as validate, TimePoint};
use osu_beatmap_preview_core::support::error::{PreviewError, Result};

#[derive(Debug, Clone)]
pub struct RenderRequest {
    pub source: SourceOptions,
    pub ruleset: RulesetOptions,
    pub view: ViewOptions,
    pub output: OutputOptions,
    pub execution: ExecutionOptions,
    /// 游玩/回放配置。目前只有 `Preview` 一种可用模式，CLI 也还没有对应参数；
    /// 保留字段是为了让「用回放导出、在画面上叠分数」这类功能接入时不必再改请求模型
    /// （接口见 core 的 `gameplay` 模块）。
    pub gameplay: GameplayOptions,
}

impl RenderRequest {
    pub fn new(bid: impl Into<String>) -> Self {
        Self {
            source: SourceOptions {
                bid: bid.into(),
                input_file: None,
            },
            ruleset: RulesetOptions::default(),
            view: ViewOptions::default(),
            output: OutputOptions::default(),
            execution: ExecutionOptions::default(),
            gameplay: GameplayOptions::default(),
        }
    }

    pub(crate) fn validate(self) -> Result<ValidatedRequest> {
        let input_kind = validate_source(&self.source)?;
        // 本地 `.osu` 没有音源可以合进音轨，视频（MP4）无从谈起，只允许 PNG / GIF。
        // 默认输出格式本来就只会是 PNG 或 GIF，因此这里只拦显式请求的 mp4。
        if input_kind == Some(LocalInputKind::Osu) && self.output.format.as_deref() == Some("mp4") {
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
        Ok(ValidatedRequest {
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

#[derive(Debug, Clone)]
pub struct SourceOptions {
    /// 谱面 ID。`--input-file` 为 `.osz` 时必填（在压缩包内按 `BeatmapID` 找 `.osu`），
    /// 为 `.osu` 时可留空（只用于产物命名），未提供 `--input-file` 时必填（按它下载）。
    pub bid: String,
    /// 本地谱面文件路径（`.osu` 或 `.osz`）；提供时不再从网络下载。
    pub input_file: Option<String>,
}

/// 校验谱面来源：`bid` 与本地文件的组合规则。
///
/// 返回本地输入类型（`None` 表示按 bid 下载）。规则：
/// - 不提供 `--input-file`：`--bid` 必填且为纯数字；
/// - 本地 `.osu`：`--bid` 可留空，给了也必须是纯数字；
/// - 本地 `.osz`：`--bid` 必填，用于在压缩包内查找对应的 `.osu` 难度。
///
/// CLI 在参数解析后、库入口在请求校验时都会调用它，规则只有这一份。
pub(crate) fn validate_source(source: &SourceOptions) -> Result<Option<LocalInputKind>> {
    let Some(input_file) = source.input_file.as_deref() else {
        if source.bid.is_empty() {
            return Err(PreviewError::new(
                "--bid is required unless --input-file is a .osu file",
            ));
        }
        validate::validate_bid(&source.bid)?;
        return Ok(None);
    };
    if input_file.trim().is_empty() {
        return Err(PreviewError::new("--input-file must not be empty"));
    }
    match local::input_kind(input_file)? {
        LocalInputKind::Osu => {
            if !source.bid.is_empty() {
                validate::validate_bid(&source.bid)?;
            }
            Ok(Some(LocalInputKind::Osu))
        }
        LocalInputKind::Osz => {
            if source.bid.is_empty() {
                return Err(PreviewError::new(
                    "--bid is required when --input-file is an .osz file",
                ));
            }
            validate::validate_bid(&source.bid)?;
            Ok(Some(LocalInputKind::Osz))
        }
    }
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
    /// 游玩/回放配置：目前只有 `Preview`（不改变任何行为），导出侧还没有读取它。
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

    /// bid 与本地文件的组合规则：本地 `.osu` 可以没有 bid，本地 `.osz` 必须有。
    #[test]
    fn source_rules_depend_on_input_file_kind() {
        let mut local_osu = RenderRequest::new("");
        local_osu.source.input_file = Some("map.osu".to_string());
        local_osu.validate().unwrap();

        let mut osz_without_bid = RenderRequest::new("");
        osz_without_bid.source.input_file = Some("map.osz".to_string());
        let error = osz_without_bid.validate().unwrap_err().to_string();
        assert!(error.contains("--bid is required"), "{error}");

        let mut osz_with_bid = RenderRequest::new("123");
        osz_with_bid.source.input_file = Some("map.osz".to_string());
        osz_with_bid.validate().unwrap();

        // 没有本地文件时 bid 仍然是必填项。
        let error = RenderRequest::new("").validate().unwrap_err().to_string();
        assert!(error.contains("--bid is required"), "{error}");
        let mut non_numeric = RenderRequest::new("abc");
        non_numeric.source.input_file = Some("map.osu".to_string());
        assert!(non_numeric.validate().is_err());
    }

    /// 本地 `.osu` 没有音源，只允许 PNG / GIF；本地 `.osz` 支持完整三种格式。
    #[test]
    fn local_osu_input_rejects_video_output() {
        let mut request = RenderRequest::new("");
        request.source.input_file = Some("map.osu".to_string());
        request.output.format = Some("mp4".to_string());
        let error = request.validate().unwrap_err().to_string();
        assert!(error.contains("not MP4"), "{error}");

        let mut request = RenderRequest::new("");
        request.source.input_file = Some("map.osu".to_string());
        request.output.format = Some("gif".to_string());
        request.validate().unwrap();

        let mut osz = RenderRequest::new("123");
        osz.source.input_file = Some("map.osz".to_string());
        osz.output.format = Some("mp4".to_string());
        osz.validate().unwrap();
    }

    /// 本地文件只接受 .osu / .osz。
    #[test]
    fn unsupported_input_file_extension_is_rejected() {
        let mut request = RenderRequest::new("123");
        request.source.input_file = Some("song.mp3".to_string());
        let error = request.validate().unwrap_err().to_string();
        assert!(error.contains(".osu or .osz"), "{error}");

        let mut empty = RenderRequest::new("123");
        empty.source.input_file = Some("  ".to_string());
        assert!(empty.validate().is_err());
    }
}
