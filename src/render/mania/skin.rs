//! osu!mania 皮肤配置加载。
//!
//! 配置来自 `default_config.yml` 生成的运行时快照；每个 `KEYS_N` 块都
//! 显式保存列宽、列线宽、列颜色和判定线位置。

use crate::render::canvas::Rgba;

/// 单个键数对应的 mania 皮肤配置。
pub(crate) struct ManiaSkinConfig {
    /// 判定线距底部的逻辑距离（768 高坐标系）。
    pub(crate) hit_position: f64,
    /// 每列宽度（像素）。
    pub(crate) column_widths: Vec<i64>,
    /// 列分隔线宽度（keys + 1 个：最左、列间、最右）。
    pub(crate) column_line_widths: Vec<i64>,
    /// 每列背景色。
    pub(crate) column_colours: Vec<Rgba>,
}

/// 按键数加载 mania 皮肤配置；没有匹配块时返回默认值。
pub(crate) fn load_mania_skin_config(keys: i32) -> ManiaSkinConfig {
    macro_rules! block {
        ($name:ident) => {
            from_config(&crate::config::current().skin.MANIA.$name, keys as usize)
        };
    }

    match keys {
        1 => block!(KEYS_1),
        2 => block!(KEYS_2),
        3 => block!(KEYS_3),
        4 => block!(KEYS_4),
        5 => block!(KEYS_5),
        6 => block!(KEYS_6),
        7 => block!(KEYS_7),
        8 => block!(KEYS_8),
        9 => block!(KEYS_9),
        10 => block!(KEYS_10),
        11 => block!(KEYS_11),
        12 => block!(KEYS_12),
        13 => block!(KEYS_13),
        14 => block!(KEYS_14),
        15 => block!(KEYS_15),
        16 => block!(KEYS_16),
        17 => block!(KEYS_17),
        18 => block!(KEYS_18),
        _ => default_skin_config(keys),
    }
}

fn from_config<T>(block: &T, keys: usize) -> ManiaSkinConfig
where
    T: ManiaSkinBlock,
{
    let keys = keys.max(1);
    let column_widths = normalize_int_list(
        block.column_widths(),
        keys,
        crate::config::current().layout.mania.png.LANE_WIDTH,
    );
    let column_line_widths = normalize_int_list(block.column_line_widths(), keys + 1, 0);
    let column_colours = normalize_colours(
        block.column_colours(),
        keys,
        crate::config::current().layout.mania.png.LANE_BACKGROUND,
    );

    ManiaSkinConfig {
        hit_position: parse_hit_position(block.hit_position()),
        column_widths,
        column_line_widths,
        column_colours,
    }
}

/// Common view over generated `SkinMANIAKEYS_NConfig` structs.
trait ManiaSkinBlock {
    fn hit_position(&self) -> i64;
    fn column_widths(&self) -> &[i64];
    fn column_line_widths(&self) -> &[i64];
    fn column_colours(&self) -> &[[u8; 4]];
}

macro_rules! impl_mania_skin_block {
    ($($name:ident),+ $(,)?) => {
        $(
            impl ManiaSkinBlock for crate::config::$name {
                fn hit_position(&self) -> i64 { self.HIT_POSITION }
                fn column_widths(&self) -> &[i64] { &self.COLUMN_WIDTHS }
                fn column_line_widths(&self) -> &[i64] { &self.COLUMN_LINE_WIDTHS }
                fn column_colours(&self) -> &[[u8; 4]] { &self.COLUMN_COLORS }
            }
        )+
    };
}

impl_mania_skin_block!(
    SkinMANIAKEYS_1Config,
    SkinMANIAKEYS_2Config,
    SkinMANIAKEYS_3Config,
    SkinMANIAKEYS_4Config,
    SkinMANIAKEYS_5Config,
    SkinMANIAKEYS_6Config,
    SkinMANIAKEYS_7Config,
    SkinMANIAKEYS_8Config,
    SkinMANIAKEYS_9Config,
    SkinMANIAKEYS_10Config,
    SkinMANIAKEYS_11Config,
    SkinMANIAKEYS_12Config,
    SkinMANIAKEYS_13Config,
    SkinMANIAKEYS_14Config,
    SkinMANIAKEYS_15Config,
    SkinMANIAKEYS_16Config,
    SkinMANIAKEYS_17Config,
    SkinMANIAKEYS_18Config,
);

/// 规范化强类型数组：非负、不足时重复最后一个、超出时截断。
fn normalize_int_list(raw: &[i64], count: usize, default: i64) -> Vec<i64> {
    let mut values: Vec<i64> = raw.iter().map(|value| (*value).max(0)).collect();
    if values.is_empty() {
        values.push(default);
    }
    while values.len() < count {
        values.push(*values.last().unwrap());
    }
    values.truncate(count);
    values
}

fn normalize_colours(raw: &[[u8; 4]], count: usize, fallback: Rgba) -> Vec<Rgba> {
    let mut values: Vec<Rgba> = raw.to_vec();
    if values.is_empty() {
        values.push(fallback);
    }
    while values.len() < count {
        values.push(*values.last().unwrap());
    }
    values.truncate(count);
    values
}

/// osu! stable 的 HitPosition 基于 480 高坐标系；转换为 768 高 GIF/MP4
/// 坐标系中距底部的距离。
fn parse_hit_position(raw: i64) -> f64 {
    (480.0 - (raw as f64).clamp(240.0, 480.0)) * 1.6
}

/// 缺省配置：等宽列、无分隔线、默认判定线位置。
fn default_skin_config(keys: i32) -> ManiaSkinConfig {
    let keys = keys.max(0) as usize;
    ManiaSkinConfig {
        hit_position: crate::config::current()
            .layout
            .mania
            .png
            .HIT_TARGET_FROM_BOTTOM as f64,
        column_widths: vec![crate::config::current().layout.mania.png.LANE_WIDTH; keys],
        column_line_widths: vec![0; keys + 1],
        column_colours: vec![crate::config::current().layout.mania.png.LANE_BACKGROUND; keys],
    }
}

#[cfg(test)]
mod tests {
    use super::load_mania_skin_config;

    #[test]
    fn all_standard_keycounts_use_explicit_skin_blocks() {
        for keys in 1..=18 {
            let config = load_mania_skin_config(keys);
            assert_eq!(config.column_widths.len(), keys as usize);
            assert_eq!(config.column_line_widths.len(), keys as usize + 1);
            assert_eq!(config.column_colours.len(), keys as usize);
        }
    }

    #[test]
    fn odd_keycount_defaults_match_migrated_values() {
        let config_3 = load_mania_skin_config(3);
        assert_eq!(config_3.column_widths, vec![68, 68, 68]);
        assert!((config_3.hit_position - 11.2).abs() < 1e-9);

        let config_11 = load_mania_skin_config(11);
        assert_eq!(config_11.column_widths, vec![53; 11]);
        assert!((config_11.hit_position - 32.0).abs() < 1e-9);

        let config_17 = load_mania_skin_config(17);
        assert_eq!(config_17.column_widths, vec![48; 17]);
    }

    #[test]
    fn unknown_keycount_uses_fallback_layout() {
        let config = load_mania_skin_config(19);
        assert_eq!(config.column_widths.len(), 19);
        assert_eq!(config.column_line_widths, vec![0; 20]);
    }
}
