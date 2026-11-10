//! 帧文本按 LegacyScoreDecoder 的宽容规则读取；原始位掩码与同刻帧顺序保持不变。
use crate::replay::model::ReplayFrame;
use crate::support::error::{PreviewError, Result};

pub(crate) const MAX_COORDINATE: f32 = 131072.0;
pub(crate) const MAX_MANIA_MASK: f32 = ((1 << 20) - 1) as f32;

pub(super) fn parse(bytes: &[u8], mode: u8) -> Result<(Vec<ReplayFrame>, Option<i64>)> {
    // StreamReader 默认以替代字符处理无效 UTF-8，并识别 UTF-8 BOM。
    let text = super::decode::text(bytes);
    let mut frames = Vec::new();
    let mut time = 0_i64;
    let mut seed = None;
    for (sequence, token) in text.split(',').enumerate() {
        let mut parts = token.split('|');
        let (Some(delta_text), Some(x), Some(y), Some(keys)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        // 游戏先跳过 seed 行，不解析其坐标、按键或后续内容；seed 不限制出现位置或次数。
        if delta_text == "-12345" {
            seed = keys.trim().parse().ok();
            continue;
        }
        let delta = delta(delta_text)?;
        let x = coordinate(
            x,
            if mode == 3 {
                MAX_MANIA_MASK
            } else {
                MAX_COORDINATE
            },
        )?;
        let y = coordinate(y, MAX_COORDINATE)?;
        let keys = keys
            .trim()
            .parse::<i32>()
            .map_err(|_| invalid("invalid frame buttons"))?;
        if keys == i32::MIN {
            return Err(invalid("frame buttons outside Int32 parse limit"));
        }
        time = time
            .checked_add(delta)
            .ok_or_else(|| invalid("frame time overflow"))?;
        // 预览的资源预算独立于游戏格式校验，保留明确限制以防无界编译。
        if time.unsigned_abs() > 86_400_000 || frames.len() >= 2_000_000 {
            return Err(invalid("preview frame resource limit exceeded"));
        }
        frames.push(ReplayFrame {
            delta_ms: delta,
            time_ms: time,
            cursor: [x, y],
            keys: keys as u32,
            sequence,
        });
    }
    Ok((crate::replay::normalize::frames(frames), seed))
}
fn delta(value: &str) -> Result<i64> {
    let value = value.trim();
    if let Ok(integer) = value.parse::<i32>() {
        return Ok(i64::from(integer));
    }
    // 旧 lazer 的 float 间隔逐帧以 ties-to-even 舍入，整数优先避免 f32 精度损失。
    let value = coordinate(value, i32::MAX as f32)?;
    // 当前 lazer 使用 .NET 10，越界的浮点到 Int32 转换会饱和。
    Ok(i64::from(value.round_ties_even() as i32))
}
fn coordinate(value: &str, limit: f32) -> Result<f32> {
    let value = value
        .trim()
        .replace(',', "")
        .parse::<f32>()
        .map_err(|_| invalid("invalid frame number"))?;
    if !value.is_finite() || value.abs() > limit {
        return Err(invalid(
            "frame coordinate or delta outside game parse limit",
        ));
    }
    Ok(value)
}
fn invalid(message: &str) -> PreviewError {
    PreviewError::parse(format!("InvalidReplay: {message}"))
}
