//! Minimal bitmap text rendering (8x8 base font, nearest-neighbour scaled).
//! Glyphs are trimmed to their real width so digit spacing stays tight,
//! mirroring the role of PIL's default proportional font.

use crate::canvas::{Img, Rgba};
use font8x8::legacy::BASIC_LEGACY;

fn glyph(c: char) -> [u8; 8] {
    let idx = c as usize;
    if idx < BASIC_LEGACY.len() {
        BASIC_LEGACY[idx]
    } else {
        BASIC_LEGACY[b'?' as usize]
    }
}

/// Leftmost set column and width of the glyph's used columns.
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

/// Approximate PIL load_default(size=N): glyph cell height ~= size.
pub fn text_size(text: &str, size: u32) -> (u32, u32) {
    let scale = scale_for(size);
    let mut w = 0u32;
    for ch in text.chars() {
        let (_, gw) = glyph_extent(&glyph(ch));
        w += (gw + 1) * scale;
    }
    (w.saturating_sub(scale), 8 * scale)
}

pub fn draw_text(img: &mut Img, x: i64, y: i64, text: &str, size: u32, color: Rgba) {
    let scale = scale_for(size) as i64;
    let mut cx = x;
    for ch in text.chars() {
        let g = glyph(ch);
        let (min_col, gw) = glyph_extent(&g);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..8u32 {
                if bits >> col & 1 != 0 {
                    let px = cx + (col - min_col) as i64 * scale;
                    let py = y + row as i64 * scale;
                    for dy in 0..scale {
                        for dx in 0..scale {
                            img.blend_px(px + dx, py + dy, color);
                        }
                    }
                }
            }
        }
        cx += (gw as i64 + 1) * scale;
    }
}

pub fn format_mmssmmm(ms: i64) -> String {
    let ms = ms.max(0);
    let minutes = ms / 60000;
    let seconds = (ms % 60000) / 1000;
    let millis = ms % 1000;
    format!("{minutes:02}:{seconds:02}:{millis:03}")
}
