//! osu!standard renderer: per-frame 512×384 gameplay snapshots composed into a
//! PNG grid (5×8) or animated GIF (2×2 segments). Port of the Python renderer
//! with identical constants, alpha curves and layout.

mod alpha;
mod constants;
pub(crate) mod context;
mod digits;
mod gif;
mod objects;
mod png;
pub(crate) mod slider;

pub(crate) use gif::render_standard_gif;
pub(crate) use png::render_standard_png;

use crate::canvas::Img;
use crate::text::{draw_text, text_size};
use constants::*;

/// Draw text centered horizontally within `IMAGE_WIDTH` at `(x, y)`.
pub(crate) fn draw_centered_text(
    canvas: &mut Img,
    text: &str,
    x: i64,
    y: i64,
    size: u32,
    color: [u8; 4],
) {
    let (text_w, _) = text_size(text, size);
    let text_x = x + (IMAGE_WIDTH - text_w as i64) / 2;
    draw_text(canvas, text_x, y, text, size, color);
}

/// Draw a time label (and optional note) centered below a frame at `(x, y)`.
pub(crate) fn draw_time_label(
    canvas: &mut Img,
    label: &str,
    x: i64,
    y: i64,
    note: Option<&str>,
    label_color: [u8; 4],
    note_color: [u8; 4],
) {
    draw_centered_text(canvas, label, x, y, TIME_LABEL_FONT_SIZE, label_color);
    if let Some(note_text) = note {
        let (_, label_h) = text_size(label, TIME_LABEL_FONT_SIZE);
        let note_y = y + label_h as i64 + TIME_LABEL_NOTE_TOP_GAP;
        draw_centered_text(
            canvas,
            note_text,
            x,
            note_y,
            TIME_LABEL_NOTE_FONT_SIZE,
            note_color,
        );
    }
}
