//! 帧文本按原顺序累计；历史小数间隔采用官方的逐帧舍入方式。

use crate::replay::model::ReplayFrame;
use crate::support::error::{PreviewError, Result};

pub(super) fn parse(bytes: &[u8]) -> Result<(Vec<ReplayFrame>, Option<i64>)> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("non-UTF-8 frames"))?;
    let mut frames = Vec::new();
    let mut time = 0_i64;
    let mut seed = None;
    for (sequence, token) in text
        .strip_suffix(',')
        .unwrap_or(text)
        .split(',')
        .enumerate()
    {
        let mut parts = token.split('|');
        let delta = delta(parts.next())?;
        let x: f32 = number(parts.next())?;
        let y: f32 = number(parts.next())?;
        let keys: i64 = number(parts.next())?;
        if parts.next().is_some() || !x.is_finite() || !y.is_finite() {
            return Err(invalid("invalid frame token"));
        }
        if delta == -12345 {
            if seed.replace(keys).is_some() {
                return Err(invalid("duplicate seed"));
            }
            continue;
        }
        if seed.is_some() {
            return Err(invalid("input after seed"));
        }
        time = time
            .checked_add(delta)
            .ok_or_else(|| invalid("frame time overflow"))?;
        // 首版最长接受一天的回放；负首帧仍在绝对时间域内保留。
        if time.unsigned_abs() > 86_400_000 || frames.len() >= 2_000_000 {
            return Err(invalid("frame limit exceeded"));
        }
        frames.push(ReplayFrame {
            delta_ms: delta,
            time_ms: time,
            cursor: [x, y],
            keys: u32::try_from(keys).map_err(|_| invalid("invalid key mask"))?,
            sequence,
        });
    }
    if frames.is_empty() {
        return Err(invalid("empty replay"));
    }
    Ok((crate::replay::normalize::frames(frames), seed))
}
fn delta(value: Option<&str>) -> Result<i64> {
    let value = value.ok_or_else(|| invalid("truncated frame"))?;
    if let Ok(integer) = value.parse::<i64>() {
        return Ok(integer);
    }
    // 旧 lazer 曾输出 float 间隔；先解析整数，避免大整数经过 f32 后丢失精度。
    let value = value
        .parse::<f32>()
        .map_err(|_| invalid("invalid frame number"))?;
    if !value.is_finite() || value.abs() > 86_400_000.0 {
        return Err(invalid("invalid frame delta"));
    }
    Ok(value.round_ties_even() as i64)
}
fn number<T: std::str::FromStr>(value: Option<&str>) -> Result<T> {
    value
        .ok_or_else(|| invalid("truncated frame"))?
        .parse()
        .map_err(|_| invalid("invalid frame number"))
}
fn invalid(message: &str) -> PreviewError {
    PreviewError::parse(format!("InvalidReplay: {message}"))
}
