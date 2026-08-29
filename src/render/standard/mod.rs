//! osu!standard 渲染器：将每帧 512×384 游戏画面合成为 PNG 网格（5×8）
//! 或 GIF 动画（2×2 分段）。移植自 Python 渲染器，常量、alpha 曲线与布局保持一致。

mod alpha;
mod constants;
pub(crate) mod context;
mod digits;
mod gif;
mod objects;
mod png;
pub(crate) mod slider;
mod video;

pub(crate) use gif::render_standard_gif;
pub(crate) use png::render_standard_png;
pub(crate) use video::render_standard_video;

use crate::render::canvas::Img;
use crate::render::text::{draw_text, text_size};
/// 在 `IMAGE_WIDTH` 范围内以 `(x, y)` 为基准水平居中绘制文字。
pub(crate) fn draw_centered_text(
    canvas: &mut Img,
    text: &str,
    x: i64,
    y: i64,
    size: u32,
    color: [u8; 4],
) {
    let (text_w, _) = text_size(text, size);
    let text_x = x + (crate::render::standard::constants::IMAGE_WIDTH - text_w as i64) / 2;
    draw_text(canvas, text_x, y, text, size, color);
}

/// 在 `(x, y)` 下方居中绘制时间标签及可选提示文字。
pub(crate) fn draw_time_label(
    canvas: &mut Img,
    label: &str,
    x: i64,
    y: i64,
    note: Option<&str>,
    label_color: [u8; 4],
    note_color: [u8; 4],
) {
    draw_centered_text(
        canvas,
        label,
        x,
        y,
        crate::config::current()
            .layout
            .standard
            .png
            .TIME_LABEL_FONT_SIZE,
        label_color,
    );
    if let Some(note_text) = note {
        let (_, label_h) = text_size(
            label,
            crate::config::current()
                .layout
                .standard
                .png
                .TIME_LABEL_FONT_SIZE,
        );
        let note_y = y
            + label_h as i64
            + crate::config::current()
                .layout
                .standard
                .png
                .TIME_LABEL_NOTE_TOP_GAP;
        draw_centered_text(
            canvas,
            note_text,
            x,
            note_y,
            crate::config::current()
                .layout
                .standard
                .png
                .TIME_LABEL_NOTE_FONT_SIZE,
            note_color,
        );
    }
}
