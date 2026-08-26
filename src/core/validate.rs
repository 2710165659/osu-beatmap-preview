//! Consolidated parameter validation.
//!
//! Split into two phases:
//! 1. CLI-phase: value-format checks that run during argument parsing.
//! 2. Context-phase: checks that need the beatmap mode and resolved format.

use crate::core::errors::{PreviewError, Result};
use crate::core::mods::{mods_for_mode, validate_mods, ModSettings};

/// Validate `--convert` value.
pub fn validate_convert_value(v: &str) -> Result<()> {
    match v {
        "mania" | "ctb" | "taiko" | "standard" | "std" => Ok(()),
        _ => Err(PreviewError::new(format!(
            "--convert must be one of mania, ctb, taiko, standard; got '{v}'"
        ))),
    }
}

/// Validate `--fmt` value.
pub fn validate_fmt_value(v: &str) -> Result<()> {
    match v {
        "png" | "gif" | "mp4" => Ok(()),
        _ => Err(PreviewError::new(format!(
            "--fmt must be png, gif, or mp4; got '{v}'"
        ))),
    }
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

/// Context for mode-aware validation.
pub struct ValidateContext<'a> {
    pub bid: &'a str,
    pub fmt: &'a str,
    pub target_mode: i32,
}

/// Validate parameters that depend on the resolved target mode and format.
///
/// Returns validated mod settings (mode-adjusted), or `None`.
pub fn validate_with_context(
    ctx: &ValidateContext,
    time_points: &[TimePoint],
    duration_time: Option<f64>,
    mods: Option<ModSettings>,
) -> Result<Option<ModSettings>> {
    // --- bid ---
    if ctx.bid.is_empty() || !ctx.bid.chars().all(|c| c.is_ascii_digit()) {
        return Err(PreviewError::new("bid must be numeric"));
    }

    if duration_time.is_some() && ctx.fmt != "mp4" {
        return Err(PreviewError::new(
            "--duration-time is only valid for mp4 output",
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
        if !duration.is_finite() || duration <= 0.0 {
            return Err(PreviewError::new(
                "duration time must be a positive finite number",
            ));
        }
    }

    // --- mods ---
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
    fn video_time_options_are_format_scoped_and_positive() {
        validate_with_context(&ctx("mp4", 0), &[TimePoint::Preview], Some(30.0), None).unwrap();
        assert!(validate_with_context(&ctx("gif", 0), &[], Some(30.0), None).is_err());
        assert!(validate_with_context(&ctx("mp4", 0), &[], Some(0.0), None).is_err());
    }
}
