//! 最小位图文字渲染（8x8 基础字体，最近邻缩放）。
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

fn scale_for(size: u32) -> u32 {
    (size.max(8) / 8).max(1)
}

/// 近似 PIL load_default(size=N)：字形单元高度约等于 size。
pub fn text_size(text: &str, size: u32) -> (u32, u32) {
    let scale = scale_for(size);
    let mut w = 0u32;
    for ch in text.chars() {
        let (_, gw) = glyph_extent(&glyph(ch));
        w += (gw + 1) * scale;
    }
    (w.saturating_sub(scale), 8 * scale)
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
    let scale = scale_for(size) as i64;
    let w = gw as i64 * scale;
    let h = 8i64 * scale;
    let mut sprite = Img::new(w.max(1) as u32, h.max(1) as u32, [0, 0, 0, 0]);
    for (row, bits) in g.iter().enumerate() {
        for col in 0..8u32 {
            if bits >> col & 1 != 0 {
                let px = (col as i64 - min_col as i64) * scale;
                let py = row as i64 * scale;
                sprite.fill_rect(px, py, px + scale - 1, py + scale - 1, color);
            }
        }
    }
    sprite
}

/// 使用线程局部的预渲染字形精灵缓存绘制文字。
/// 重复字符（数字、':'、'.' 等）只渲染一次后进行 alpha 合成，
/// 避免内部 scale×scale 的 blend_px 循环。
pub fn draw_text(img: &mut Img, x: i64, y: i64, text: &str, size: u32, color: Rgba) {
    let scale = scale_for(size) as i64;

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
            cx += sprite.w as i64 + scale; // advance = glyph width + 1 cell gap
        }
    });
}

/// 将文字渲染为独立 RGBA 精灵图（透明背景）。
///
/// 与直接合成到目标画布的 `draw_text` 不同，此函数生成紧凑的 `Img`，
/// 可重复执行 `alpha_composite`，无需每帧重新运行字形缓存和格式化逻辑。
/// 用于热点循环（例如每段重复 150 次的 mania GIF 时间标签）。
pub fn render_text_sprite(text: &str, size: u32, color: Rgba) -> Img {
    let scale = scale_for(size) as i64;
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
            cx += sprite.w as i64 + scale;
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
}
