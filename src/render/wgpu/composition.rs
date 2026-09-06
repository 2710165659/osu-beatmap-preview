//! 将 playfield、背景和 HUD 组合为固定 WGPU 画布上的有序场景。

use std::sync::Arc;

use crate::infrastructure::media::VideoStyle;
use crate::realtime::RealtimeError;
use crate::render::canvas::Img;
use crate::render::geometry::GameMode;
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
    mode: GameMode,
) -> Result<FrameScene, RealtimeError> {
    let (scale, offset) =
        fit_playfield(mode, playfield.width(), playfield.height(), width, height)?;
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
            fallback_background(mode, style),
        );
    }
    builder.append_scaled(&playfield, offset, scale);

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
    Ok(builder.finish())
}

fn fit_playfield(
    mode: GameMode,
    playfield_width: u32,
    playfield_height: u32,
    canvas_width: u32,
    canvas_height: u32,
) -> Result<(f32, [f32; 2]), RealtimeError> {
    if playfield_width == 0 || playfield_height == 0 {
        return Err(RealtimeError::InvalidFrame(
            "playfield dimensions must be positive".to_string(),
        ));
    }
    let width_ratio = canvas_width as f32 / playfield_width as f32;
    let height_ratio = canvas_height as f32 / playfield_height as f32;
    let scale = match mode {
        GameMode::Standard | GameMode::Catch => width_ratio.min(height_ratio),
        GameMode::Taiko => width_ratio,
        GameMode::Mania => height_ratio,
    };
    let scaled_width = (playfield_width as f32 * scale).round();
    let scaled_height = (playfield_height as f32 * scale).round();
    if scaled_width > canvas_width as f32 + 0.5 || scaled_height > canvas_height as f32 + 0.5 {
        return Err(RealtimeError::CanvasTooSmall {
            width: canvas_width,
            height: canvas_height,
            required_width: scaled_width.ceil() as u32,
            required_height: scaled_height.ceil() as u32,
        });
    }
    Ok((
        scale,
        [
            (canvas_width as f32 - scaled_width) / 2.0,
            (canvas_height as f32 - scaled_height) / 2.0,
        ],
    ))
}

fn fallback_background(mode: GameMode, style: VideoStyle) -> [u8; 4] {
    let render = &crate::infrastructure::config::current().render;
    let color = match mode {
        GameMode::Standard => render.standard.mp4.style.IMAGE_BACKGROUND_COLOR,
        GameMode::Catch => render.catch.mp4.style.PLAYFIELD_BACKGROUND,
        GameMode::Taiko => render.taiko.mp4.style.IMAGE_BACKGROUND,
        GameMode::Mania => render.mania.mp4.style.IMAGE_BACKGROUND,
    };
    if color[3] == 0 {
        style.black_opaque
    } else {
        color
    }
}

#[cfg(test)]
mod tests {
    use super::fit_playfield;
    use crate::realtime::RealtimeError;
    use crate::render::geometry::GameMode;

    #[test]
    fn standard和catch按contain居中补边() {
        let (scale, offset) = fit_playfield(GameMode::Standard, 530, 384, 1280, 720).unwrap();
        assert!((scale - 1.875).abs() < 0.001);
        assert!((offset[0] - 142.8).abs() < 1.0);
        assert_eq!(offset[1], 0.0);
    }

    #[test]
    fn taiko铺满宽度且上下补边() {
        let (scale, offset) = fit_playfield(GameMode::Taiko, 683, 100, 1280, 720).unwrap();
        assert!((scale - 1280.0 / 683.0).abs() < 0.001);
        assert!(offset[0].abs() < 0.01);
        assert!(offset[1] > 250.0);
    }

    #[test]
    fn mania铺满高度且左右补边() {
        let (scale, offset) = fit_playfield(GameMode::Mania, 272, 384, 1280, 720).unwrap();
        assert!((scale - 1.875).abs() < 0.001);
        assert!(offset[0] > 380.0);
        assert!(offset[1].abs() < 0.01);
    }

    #[test]
    fn taiko和mania在补边方向超出画布时报告所需尺寸() {
        let taiko = fit_playfield(GameMode::Taiko, 100, 100, 1280, 720).unwrap_err();
        assert!(matches!(
            taiko,
            RealtimeError::CanvasTooSmall {
                width: 1280,
                height: 720,
                required_width: 1280,
                required_height: 1280,
            }
        ));
        let mania = fit_playfield(GameMode::Mania, 100, 100, 720, 1280).unwrap_err();
        assert!(matches!(
            mania,
            RealtimeError::CanvasTooSmall {
                width: 720,
                height: 1280,
                required_width: 1280,
                required_height: 1280,
            }
        ));
    }
}
