//! 将 playfield、背景和 HUD 组合为固定 WGPU 画布上的有序场景。

use std::sync::Arc;

use super::VideoStyle;
use crate::domain::errors::PreviewError;
use crate::domain::parser::round_half_even;
use crate::render::canvas::Img;
use crate::render::geometry::GameMode;
use crate::render::scene::{FrameScene, FrameSceneBuilder, SceneRect};
use crate::render::text::{draw_text, text_size};

const LABEL_REFERENCE_WIDTH: f64 = 1280.0;
const LABEL_REFERENCE_HEIGHT: f64 = 720.0;

#[allow(clippy::too_many_arguments)]
pub fn compose_video_scene(
    playfield: FrameScene,
    current_ms: i64,
    total_ms: i64,
    width: u32,
    height: u32,
    background: Option<&Arc<Img>>,
    style: VideoStyle,
    mode: GameMode,
) -> crate::domain::errors::Result<FrameScene> {
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
            style.fallback_background,
        );
    }
    builder.append_scaled(&playfield, offset, scale);

    let label = format!(
        "{}/{}",
        crate::render::text::format_mmss_floor(current_ms),
        crate::render::text::format_mmss_floor(total_ms)
    );
    let label_scale = (width as f64 / LABEL_REFERENCE_WIDTH)
        .min(height as f64 / LABEL_REFERENCE_HEIGHT)
        .max(f64::MIN_POSITIVE);
    let label_font_size = round_half_even(style.label_font_size as f64 * label_scale).max(1) as u32;
    let label_pad = round_half_even(style.label_pad as f64 * label_scale).max(0);
    let (label_width, label_height) = text_size(&label, label_font_size);
    let mut image = Img::new(label_width.max(1), label_height.max(1), [0, 0, 0, 0]);
    draw_text(&mut image, 0, 0, &label, label_font_size, style.label_color);
    builder.sprite(
        Arc::new(image),
        SceneRect {
            x: (width as i64 - label_width as i64 - label_pad) as f32,
            y: label_pad as f32,
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
) -> crate::domain::errors::Result<(f32, [f32; 2])> {
    if playfield_width == 0 || playfield_height == 0 {
        return Err(PreviewError::render(
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
        return Err(PreviewError::render(format!(
            "画布尺寸不足，需要 {}x{}",
            scaled_width.ceil() as u32,
            scaled_height.ceil() as u32
        )));
    }
    Ok((
        scale,
        [
            (canvas_width as f32 - scaled_width) / 2.0,
            (canvas_height as f32 - scaled_height) / 2.0,
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::{compose_video_scene, fit_playfield, VideoStyle};
    use crate::render::geometry::GameMode;
    use crate::render::scene::{DrawCommand, FrameScene, SceneSize};

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
        assert!(fit_playfield(GameMode::Taiko, 100, 100, 1280, 720).is_err());
        assert!(fit_playfield(GameMode::Mania, 100, 100, 720, 1280).is_err());
    }

    #[test]
    fn 时间标签随输出分辨率缩放() {
        let playfield = FrameScene::clear(
            SceneSize {
                width: 530,
                height: 384,
            },
            0,
            [0, 0, 0, 255],
        );
        let label_height = |width, height| {
            let scene = compose_video_scene(
                playfield.clone(),
                0,
                60_000,
                width,
                height,
                None,
                VideoStyle::default(),
                GameMode::Standard,
            )
            .unwrap();
            let DrawCommand::Sprite { destination, .. } = scene.commands.last().unwrap() else {
                panic!("最后一条命令必须是时间标签精灵");
            };
            destination.height
        };

        assert_eq!(label_height(1280, 720), 18.0);
        assert_eq!(label_height(1920, 1080), 27.0);
        assert_eq!(label_height(854, 480), 12.0);
    }
}
