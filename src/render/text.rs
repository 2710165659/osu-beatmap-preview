//! 最小位图文字渲染（8x8 基础字体，按目标像素高度重采样）。
//! 字形会裁剪到实际宽度，使数字间距紧凑，效果类似 PIL 默认比例字体。
//!
//! 使用以（字符、尺寸、颜色）为键的线程局部惰性缓存，重复字形（数字、标点）
//! 只渲染一次，之后直接进行 alpha 合成。

use crate::render::canvas::{Img, Rgba};
use font8x8::legacy::BASIC_LEGACY;
use std::cell::RefCell;
use std::collections::HashMap;

// ─── 字形查找 ───

fn glyph(c: char) -> [u8; 8] {
    let idx = c as usize;
    if idx < BASIC_LEGACY.len() {
        BASIC_LEGACY[idx]
    } else {
        BASIC_LEGACY[b'?' as usize]
    }
}

/// 返回字形已使用列的最左列和宽度。
fn glyph_extent(g: &[u8; 8]) -> (u32, u32) {
    let mut min_col = 8u32;
    let mut max_col = 0u32;
    let mut any = false;
    for bits in g.iter() {
        for col in 0..8u32 {
            if bits >> col & 1 != 0 {
                any = true;
                min_col = min_col.min(col);
                max_col = max_col.max(col);
            }
        }
    }
    if any {
        (min_col, max_col - min_col + 1)
    } else {
        (0, 3) // space advance
    }
}

fn glyph_height(size: u32) -> u32 {
    size.max(1)
}

/// 将逻辑字号换算为位图字体的实际输出高度。
/// 基础字号先按旧版 8px 字形取整，再应用输出倍率，以保持 1x 外观不变。
pub(crate) fn scaled_bitmap_font_height(base_size: u32, output_scale: f64) -> u32 {
    let base_height = (base_size.max(8) / 8).max(1) * 8;
    crate::domain::parser::round_half_even(base_height as f64 * output_scale).max(1) as u32
}

fn scaled_glyph_width(base_width: u32, height: u32) -> u32 {
    crate::domain::parser::round_half_even(base_width as f64 * height as f64 / 8.0).max(1) as u32
}

fn glyph_spacing(height: u32) -> u32 {
    crate::domain::parser::round_half_even(height as f64 / 8.0).max(1) as u32
}

/// 返回按目标像素高度重采样后的字形包围盒。
pub fn text_size(text: &str, size: u32) -> (u32, u32) {
    let height = glyph_height(size);
    let spacing = glyph_spacing(height);
    let mut w = 0u32;
    for ch in text.chars() {
        let (_, gw) = glyph_extent(&glyph(ch));
        w += scaled_glyph_width(gw, height) + spacing;
    }
    (w.saturating_sub(spacing), height)
}

// ─── 惰性字形缓存 ───

type CacheKey = (char, u32, [u8; 4]);

thread_local! {
    static GLYPH_CACHE: RefCell<HashMap<CacheKey, Img>> = RefCell::new(HashMap::new());
}

/// 将单个字形按 `size`、`color` 渲染为独立 RGBA 精灵图。
fn build_glyph_sprite(ch: char, size: u32, color: Rgba) -> Img {
    let g = glyph(ch);
    let (min_col, gw) = glyph_extent(&g);
    let h = glyph_height(size);
    let w = scaled_glyph_width(gw, h);
    let mut sprite = Img::new(w, h, [0, 0, 0, 0]);
    for (row, bits) in g.iter().enumerate() {
        for col in min_col..min_col + gw {
            if bits >> col & 1 != 0 {
                let local_col = col - min_col;
                let x0 = local_col * w / gw;
                let x1 = ((local_col + 1) * w).div_ceil(gw).saturating_sub(1);
                let y0 = row as u32 * h / 8;
                let y1 = ((row as u32 + 1) * h).div_ceil(8).saturating_sub(1);
                sprite.fill_rect(x0 as i64, y0 as i64, x1 as i64, y1 as i64, color);
            }
        }
    }
    sprite
}

/// 使用线程局部的预渲染字形精灵缓存绘制文字。
/// 重复字符（数字、':'、'.' 等）只渲染一次后进行 alpha 合成，
/// 避免内部 scale×scale 的 blend_px 循环。
pub fn draw_text(img: &mut Img, x: i64, y: i64, text: &str, size: u32, color: Rgba) {
    let spacing = glyph_spacing(glyph_height(size)) as i64;

    GLYPH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();

        // 限制缓存项数量；所有模式通常少于 50 项。
        if cache.len() > 512 {
            cache.clear();
        }

        let mut cx = x;
        for ch in text.chars() {
            let key: CacheKey = (ch, size, color);
            let sprite = cache
                .entry(key)
                .or_insert_with(|| build_glyph_sprite(ch, size, color));

            img.alpha_composite(sprite, cx, y);
            cx += sprite.w as i64 + spacing;
        }
    });
}

/// 将文字渲染为独立 RGBA 精灵图（透明背景）。
///
/// 与直接合成到目标画布的 `draw_text` 不同，此函数生成紧凑的 `Img`，
/// 可重复执行 `alpha_composite`，无需每帧重新运行字形缓存和格式化逻辑。
/// 用于热点循环（例如每段重复 150 次的 mania GIF 时间标签）。
pub fn render_text_sprite(text: &str, size: u32, color: Rgba) -> Img {
    let spacing = glyph_spacing(glyph_height(size)) as i64;
    let (tw, th) = text_size(text, size);
    let mut img = Img::new(tw.max(1), th.max(1), [0, 0, 0, 0]);
    let mut cx = 0i64;
    GLYPH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > 512 {
            cache.clear();
        }
        for ch in text.chars() {
            let key: CacheKey = (ch, size, color);
            let sprite = cache
                .entry(key)
                .or_insert_with(|| build_glyph_sprite(ch, size, color));
            img.alpha_composite(sprite, cx, 0);
            cx += sprite.w as i64 + spacing;
        }
    });
    img
}

pub fn format_mmssmmm(ms: i64) -> String {
    let sign = if ms < 0 { "-" } else { "" };
    let magnitude = ms.unsigned_abs();
    let minutes = magnitude / 60000;
    let seconds = (magnitude % 60000) / 1000;
    let millis = magnitude % 1000;
    format!("{sign}{minutes:02}:{seconds:02}:{millis:03}")
}

/// 按 osu! 游戏进度组件格式化整秒。格式化前先向下取整，
/// 因此 -0.5 秒会显示为 -0:01。
pub fn format_mmss_floor(ms: i64) -> String {
    let total_seconds = ms.div_euclid(1000);
    let sign = if total_seconds < 0 { "-" } else { "" };
    let magnitude = total_seconds.unsigned_abs();
    format!("{sign}{}:{:02}", magnitude / 60, magnitude % 60)
}

pub fn format_seconds_tenths(ms: i64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

#[cfg(test)]
mod time_format_tests {
    use super::*;

    #[test]
    fn formats_signed_millisecond_times() {
        assert_eq!(format_mmssmmm(62_500), "01:02:500");
        assert_eq!(format_mmssmmm(-2_500), "-00:02:500");
        assert_eq!(format_mmssmmm(i64::MIN), "-153722867280912:55:808");
    }

    #[test]
    fn formats_signed_gameplay_seconds() {
        assert_eq!(format_mmss_floor(62_500), "1:02");
        assert_eq!(format_mmss_floor(-500), "-0:01");
        assert_eq!(format_seconds_tenths(-500), "-0.5s");
    }

    #[test]
    fn bitmap_text_uses_the_requested_pixel_height() {
        for height in [4, 8, 12, 16] {
            assert_eq!(text_size("1.0x", height).1, height);
            assert_eq!(
                render_text_sprite("1.0x", height, [255, 255, 255, 255]).h,
                height
            );
        }
    }

    #[test]
    fn logical_font_size_scales_from_its_actual_bitmap_height() {
        assert_eq!(scaled_bitmap_font_height(10, 0.5), 4);
        assert_eq!(scaled_bitmap_font_height(10, 1.0), 8);
        assert_eq!(scaled_bitmap_font_height(10, 1.5), 12);
        assert_eq!(scaled_bitmap_font_height(10, 2.0), 16);
        assert_eq!(scaled_bitmap_font_height(33, 1.0), 32);
    }
}
