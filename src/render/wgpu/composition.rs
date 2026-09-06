//! 将 playfield、背景和 HUD 组合为固定 WGPU 画布上的有序场景。

use std::sync::Arc;

use crate::infrastructure::media::VideoStyle;
use crate::render::canvas::Img;
use crate::render::scene::{FrameScene, FrameSceneBuilder, SceneRect};
use crate::render::text::{draw_text, text_size};

pub(crate) fn compose_video_scene(
    playfield: FrameScene,
    current_ms: i64,
    total_ms: i64,
    width: u32,
    height: u32,
    background: Option<&Arc<Img>>,
    style: VideoStyle,
) -> FrameScene {
    let mut builder = FrameSceneBuilder::new(width, height, playfield.absolute_time_ms());
    if let Some(background) = background {
        builder.sprite(
            Arc::clone(background),
            SceneRect {
                x: 0.0,
                y: 0.0,
                width: width as f32,
                height: height as f32,
            },
            1.0,
        );
    } else {
        builder.rectangle(
            SceneRect {
                x: 0.0,
                y: 0.0,
                width: width as f32,
                height: height as f32,
            },
            style.black_opaque,
        );
    }
    let offset = [
        width.saturating_sub(playfield.width()) as f32 / 2.0,
        height.saturating_sub(playfield.height()) as f32 / 2.0,
    ];
    builder.append(&playfield, offset);

    let label = format!(
        "{}/{}",
        crate::render::text::format_mmss_floor(current_ms),
        crate::render::text::format_mmss_floor(total_ms)
    );
    let (label_width, label_height) = text_size(&label, style.label_font_size);
    let mut image = Img::new(label_width.max(1), label_height.max(1), [0, 0, 0, 0]);
    draw_text(
        &mut image,
        0,
        0,
        &label,
        style.label_font_size,
        style.label_color,
    );
    builder.sprite(
        Arc::new(image),
        SceneRect {
            x: (width as i64 - label_width as i64 - style.label_pad) as f32,
            y: style.label_pad as f32,
            width: label_width as f32,
            height: label_height as f32,
        },
        1.0,
    );
    builder.finish()
}
