//! osu!mania 皮肤配置加载。
//!
//! 从统一的 `assets/skin.ini`（经 [`crate::skin`] 解析）中按键数选取
//! 对应的 [Mania] 配置块，提取列宽 / 分隔线宽 / 判定线位置 / 列颜色。

use crate::canvas::Rgba;
use crate::parser::round_half_even;

use super::{GIF_HIT_TARGET_FROM_BOTTOM, LANE_BACKGROUND, LANE_WIDTH};

/// 单个键数对应的 mania 皮肤配置。
pub(crate) struct ManiaSkinConfig {
    /// 判定线距底部的逻辑距离（768 高坐标系）。
    pub(crate) hit_position: f64,
    /// 每列宽度（像素）。
    pub(crate) column_widths: Vec<i64>,
    /// 列分隔线宽度（keys + 1 个：最左、列间、最右）。
    pub(crate) column_line_widths: Vec<i64>,
    /// 每列背景色（skin.ini Colour1..N）。
    pub(crate) column_colours: Vec<Rgba>,
}

/// 按键数加载 mania 皮肤配置；没有匹配块时返回默认值。
pub(crate) fn load_mania_skin_config(keys: i32) -> ManiaSkinConfig {
    for block in &crate::skin::skin().mania_blocks {
        let block_keys = block_get(block, "Keys").and_then(|v| v.trim().parse::<i32>().ok());
        if block_keys == Some(keys) {
            return parse_skin_block(block, keys);
        }
    }
    default_skin_config(keys)
}

/// 在键值块中查找指定键（取最后一次出现的值）。
fn block_get<'a>(block: &'a [(String, String)], key: &str) -> Option<&'a str> {
    block
        .iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn parse_skin_block(block: &[(String, String)], keys: i32) -> ManiaSkinConfig {
    let column_widths = parse_int_list(
        block_get(block, "ColumnWidth").unwrap_or(""),
        keys as usize,
        LANE_WIDTH,
    );
    let column_line_widths = parse_int_list(
        block_get(block, "ColumnLineWidth").unwrap_or(""),
        keys as usize + 1,
        0,
    );
    let column_colours = (0..keys)
        .map(|index| {
            parse_colour(
                block_get(block, &format!("Colour{}", index + 1)).unwrap_or(""),
                LANE_BACKGROUND,
            )
        })
        .collect();
    let hit_position = parse_hit_position(block_get(block, "HitPosition"));

    ManiaSkinConfig {
        hit_position,
        column_widths,
        column_line_widths,
        column_colours,
    }
}

/// 解析逗号分隔的整数列表；不足 count 个时用最后一个值补齐。
fn parse_int_list(raw: &str, count: usize, default: i64) -> Vec<i64> {
    let mut values: Vec<i64> = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Ok(v) = part.parse::<f64>() {
            values.push(round_half_even(v).max(0));
        }
    }
    if values.is_empty() {
        values.push(default);
    }
    while values.len() < count {
        values.push(*values.last().unwrap());
    }
    values.truncate(count);
    values
}

/// 解析 "R,G,B[,A]" 颜色，格式非法时返回 fallback。
fn parse_colour(raw: &str, fallback: Rgba) -> Rgba {
    let mut values: Vec<u8> = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.parse::<f64>() {
            Ok(v) => values.push(round_half_even(v).clamp(0, 255) as u8),
            Err(_) => return fallback,
        }
    }
    if values.len() == 3 {
        values.push(255);
    }
    if values.len() != 4 {
        return fallback;
    }
    [values[0], values[1], values[2], values[3]]
}

/// osu! stable 的 HitPosition 基于 480 高坐标系；转换为 768 高 GIF
/// 坐标系中距底部的距离。
fn parse_hit_position(raw: Option<&str>) -> f64 {
    let Some(raw) = raw else {
        return GIF_HIT_TARGET_FROM_BOTTOM as f64;
    };
    match raw.trim().parse::<f64>() {
        Ok(v) => (480.0 - v.clamp(240.0, 480.0)) * 1.6,
        Err(_) => GIF_HIT_TARGET_FROM_BOTTOM as f64,
    }
}

/// 缺省配置：等宽列、无分隔线、默认判定线位置。
fn default_skin_config(keys: i32) -> ManiaSkinConfig {
    ManiaSkinConfig {
        hit_position: GIF_HIT_TARGET_FROM_BOTTOM as f64,
        column_widths: vec![LANE_WIDTH; keys as usize],
        column_line_widths: vec![0; keys as usize + 1],
        column_colours: vec![LANE_BACKGROUND; keys as usize],
    }
}
