//! Consolidated parameter validation.
//!
//! Split into two phases:
//! 1. CLI-phase: value-format checks that run during argument parsing.
//! 2. Context-phase: checks that need the beatmap mode and resolved format.

use crate::errors::{PreviewError, Result};
use crate::mods::{mods_for_mode, validate_mods, ModSettings};

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

/// Validate `--bpm` raw value (range check).
pub fn validate_bpm_value(v: f64) -> Result<()> {
    if v <= 0.0 || v >= 500.0 {
        return Err(PreviewError::new(format!(
            "--bpm must be between 0 and 500, got {v}"
        )));
    }
    Ok(())
}

/// Parse `--time` string: `T1+T2+...` seconds → `Vec<f64>` seconds.
pub fn parse_times(raw: &str) -> Result<Vec<f64>> {
    let parts: Vec<&str> = raw
        .split('+')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() > 4 {
        return Err(PreviewError::new("--time accepts at most 4 time points"));
    }
    let mut result = Vec::with_capacity(parts.len());
    for p in parts {
        let val: f64 = p
            .parse()
            .map_err(|_| PreviewError::new(format!("invalid time value: '{p}'")))?;
        if val < 0.0 {
            return Err(PreviewError::new(format!(
                "time must be non-negative, got {val}"
            )));
        }
        result.push(val);
    }
    Ok(result)
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
    times: Option<&[f64]>,
    bpm: Option<f64>,
    mods: Option<ModSettings>,
) -> Result<Option<ModSettings>> {
    // --- bid ---
    if ctx.bid.is_empty() || !ctx.bid.chars().all(|c| c.is_ascii_digit()) {
        return Err(PreviewError::new("bid must be numeric"));
    }

    // --- --times rules ---
    // mp4: 0 values (full chart ±2s) or exactly 2 (explicit [t1, t2]); else reject.
    // gif: any (≤4 by parse_times) time points.
    // standard png: time points allowed; other png modes: reject.
    if ctx.fmt == "mp4" {
        if let Some(ts) = times {
            if ts.len() != 2 {
                return Err(PreviewError::new(
                    "--time for mp4 needs exactly 2 values t1+t2 (or omit for the full chart)",
                ));
            }
        }
    } else if times.is_some() && ctx.fmt != "gif" && !(ctx.fmt == "png" && ctx.target_mode == 0) {
        return Err(PreviewError::new(
            "--times is only valid for GIF, standard PNG, or mp4 output",
        ));
    }

    // --- --bpm only for taiko PNG ---
    if bpm.is_some() && !(ctx.fmt == "png" && ctx.target_mode == 1) {
        return Err(PreviewError::new(
            "--bpm is only valid for taiko PNG output",
        ));
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
