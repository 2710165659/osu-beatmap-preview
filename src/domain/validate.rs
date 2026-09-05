//! 集中的参数校验。
//!
//! 校验分为两个阶段：
//! 1. CLI 阶段：参数解析时执行的值格式校验。
//! 2. 上下文阶段：需要谱面模式和最终输出格式的校验。

use crate::domain::errors::{PreviewError, Result};
use crate::domain::mods::{mods_for_mode, validate_mods, ModSettings};

/// 校验 Beatmap ID。
pub fn validate_bid(v: &str) -> Result<()> {
    if v.is_empty() || !v.chars().all(|c| c.is_ascii_digit()) {
        return Err(PreviewError::new("bid must be numeric"));
    }
    Ok(())
}

/// 校验 `--convert` 参数值。
pub fn validate_convert_value(v: &str) -> Result<()> {
    match v {
        "mania" | "ctb" | "taiko" | "standard" | "std" => Ok(()),
        _ => Err(PreviewError::new(format!(
            "--convert must be one of mania, ctb, taiko, standard; got '{v}'"
        ))),
    }
}

/// 校验 `--fmt` 参数值。
pub fn validate_fmt_value(v: &str) -> Result<()> {
    match v {
        "png" | "gif" | "mp4" => Ok(()),
        _ => Err(PreviewError::new(format!(
            "--fmt must be png, gif, or mp4; got '{v}'"
        ))),
    }
}

/// 将 CLI 数值参数解析为有限正数。
// 库和二进制目标分别编译本模块；该函数由二进制入口使用，库目标中会被视为未使用。
#[allow(dead_code)]
pub fn parse_positive_finite(name: &str, raw: &str) -> Result<f64> {
    let value = raw
        .parse::<f64>()
        .map_err(|_| PreviewError::new(format!("{name} must be a number, got '{raw}'")))?;
    validate_positive_finite(name, value)?;
    Ok(value)
}

/// 校验已经解析的 CLI 有限正数。
pub fn validate_positive_finite(name: &str, value: f64) -> Result<()> {
    if !value.is_finite() || value <= 0.0 {
        return Err(PreviewError::new(format!(
            "{name} must be a positive finite number"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimePoint {
    Seconds(f64),
    Preview,
}

pub fn parse_time_point(raw: &str) -> Result<TimePoint> {
    if raw.eq_ignore_ascii_case("preview") {
        return Ok(TimePoint::Preview);
    }
    let value: f64 = raw
        .parse()
        .map_err(|_| PreviewError::new(format!("invalid time point: '{raw}'")))?;
    if !value.is_finite() {
        return Err(PreviewError::new("time point must be finite"));
    }
    let milliseconds = value * 1000.0;
    if !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&milliseconds) {
        return Err(PreviewError::new(
            "time point is outside the supported range",
        ));
    }
    Ok(TimePoint::Seconds(value))
}

/// 与模式相关的校验上下文。
pub struct ValidateContext<'a> {
    pub bid: &'a str,
    pub fmt: &'a str,
    pub target_mode: i32,
}

/// 校验依赖目标模式和输出格式的参数。
///
/// 返回经过模式调整的模组设置，未指定模组时返回 `None`。
pub fn validate_with_context(
    ctx: &ValidateContext,
    time_points: &[TimePoint],
    duration_time: Option<f64>,
    mods: Option<ModSettings>,
) -> Result<Option<ModSettings>> {
    // --- 谱面 ID ---
    validate_bid(ctx.bid)?;

    if duration_time.is_some() && !matches!(ctx.fmt, "gif" | "mp4") {
        return Err(PreviewError::new(
            "--duration-time is only valid for GIF or MP4 output",
        ));
    }
    if ctx.fmt == "mp4" && time_points.len() > 1 {
        return Err(PreviewError::new(
            "mp4 accepts at most one --time-points value",
        ));
    }
    if !time_points.is_empty()
        && ctx.fmt != "gif"
        && !(ctx.fmt == "png" && ctx.target_mode == 0)
        && ctx.fmt != "mp4"
    {
        return Err(PreviewError::new(
            "--time-points is only valid for GIF, Standard PNG, or MP4 output",
        ));
    }
    if let Some(duration) = duration_time {
        validate_positive_finite("duration time", duration)?;
    }

    // --- 模组 ---
    let mods = match mods {
        Some(m) if m.has_any_mod() => {
            let mode_errors = validate_mods(&m, Some(ctx.target_mode), Some(ctx.fmt));
            if !mode_errors.is_empty() {
                return Err(PreviewError::new(format!(
                    "mod conflict: {}",
                    mode_errors.join("; ")
                )));
            }
            Some(mods_for_mode(&m, ctx.target_mode))
        }
        _ => None,
    };

    Ok(mods)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(fmt: &str, target_mode: i32) -> ValidateContext<'_> {
        ValidateContext {
            bid: "123",
            fmt,
            target_mode,
        }
    }

    #[test]
    fn parses_preview_and_numeric_video_start() {
        assert_eq!(parse_time_point("preview").unwrap(), TimePoint::Preview);
        assert_eq!(parse_time_point("-2.5").unwrap(), TimePoint::Seconds(-2.5));
        assert!(parse_time_point("NaN").is_err());
    }

    #[test]
    fn duration_time_is_available_to_gif_and_mp4() {
        validate_with_context(&ctx("mp4", 0), &[TimePoint::Preview], Some(30.0), None).unwrap();
        validate_with_context(&ctx("gif", 0), &[], Some(30.0), None).unwrap();
        assert!(validate_with_context(&ctx("png", 0), &[], Some(30.0), None).is_err());
        assert!(validate_with_context(&ctx("mp4", 0), &[], Some(0.0), None).is_err());
    }

    #[test]
    fn da_range_is_checked_after_target_mode_is_known() {
        let mania_da = crate::domain::mods::parse_mods(&["DAOD-15".into()]).unwrap();
        validate_with_context(&ctx("gif", 3), &[], None, Some(mania_da)).unwrap_err();

        let standard_da = crate::domain::mods::parse_mods(&["DAAR-10".into()]).unwrap();
        validate_with_context(&ctx("gif", 0), &[], None, Some(standard_da)).unwrap();

        let invalid_standard_da = crate::domain::mods::parse_mods(&["DAAR-10.1".into()]).unwrap();
        let error = validate_with_context(&ctx("gif", 0), &[], None, Some(invalid_standard_da))
            .unwrap_err();
        assert!(error.to_string().contains("DA AR must be in [-10.0, 11.0]"));
    }
}
