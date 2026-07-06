//! Minimal bitmap text rendering (8x8 base font, nearest-neighbour scaled).
//! Glyphs are trimmed to their real width so digit spacing stays tight,
//! mirroring the role of PIL's default proportional font.
//!
//! Uses a thread-local lazy cache keyed on (char, size, color) so repeated
//! glyphs (digits, punctuation) are rendered once then alpha-composited.

use crate::canvas::{Img, Rgba};
use font8x8::legacy::BASIC_LEGACY;
use std::cell::RefCell;
use std::collections::HashMap;

// ─── glyph lookup ───

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

// ─── lazy glyph cache ───

type CacheKey = (char, u32, [u8; 4]);

thread_local! {
    static GLYPH_CACHE: RefCell<HashMap<CacheKey, Img>> = RefCell::new(HashMap::new());
}

/// Render a single glyph at `size` in `color` into a standalone RGBA sprite.
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

/// Draw text using a thread-local cache of pre-rendered glyph sprites.
/// Repeated characters (digits, ':', '.', etc.) are rendered once then
/// alpha-composited, avoiding the inner scale×scale blend_px loop.
pub fn draw_text(img: &mut Img, x: i64, y: i64, text: &str, size: u32, color: Rgba) {
    let scale = scale_for(size) as i64;

    GLYPH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();

        // Bound cache entries — typical use is < 50 entries across all modes.
        if cache.len() > 512 {
            cache.clear();
        }

        let mut cx = x;
        for ch in text.chars() {
            let key: CacheKey = (ch, size, color);
            let sprite = cache.entry(key).or_insert_with(|| build_glyph_sprite(ch, size, color));

            img.alpha_composite(sprite, cx, y);
            cx += sprite.w as i64 + scale; // advance = glyph width + 1 cell gap
        }
    });
}

/// Render text into a standalone RGBA sprite (transparent background).
///
/// Unlike `draw_text` which composites glyphs directly onto a target canvas,
/// this produces a compact `Img` that can be `alpha_composite`-d repeatedly
/// without re-running the glyph cache + format logic each frame.  Used by
/// hot loops (e.g. mania GIF time labels that repeat 150× per segment).
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
            let sprite = cache.entry(key).or_insert_with(|| build_glyph_sprite(ch, size, color));
            img.alpha_composite(sprite, cx, 0);
            cx += sprite.w as i64 + scale;
        }
    });
    img
}

pub fn format_mmssmmm(ms: i64) -> String {
    let ms = ms.max(0);
    let minutes = ms / 60000;
    let seconds = (ms % 60000) / 1000;
    let millis = ms % 1000;
    format!("{minutes:02}:{seconds:02}:{millis:03}")
}
