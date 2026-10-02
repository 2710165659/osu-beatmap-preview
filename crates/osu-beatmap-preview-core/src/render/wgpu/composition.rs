//! 将 playfield、背景和 HUD 组合为固定 WGPU 画布上的有序场景。
//!
//! 层序与 osu! 一致：静态背景图在最底，背景视频叠在其上（`background_video`
//! 的 alpha 是淡入淡出可见度），故事板的 underlay（Background/Pass/Foreground）
//! 在视频之后、playfield 之下，playfield 与 HUD 在其上，故事板的 Overlay 层压在
//! playfield 之上（时间标签之前）。

use std::sync::Arc;

use super::VideoStyle;
use crate::domain::errors::PreviewError;
use crate::domain::parser::round_half_even;
use crate::render::canvas::Img;
use crate::render::scene::{FrameScene, FrameSceneBuilder, SceneRect};
use crate::render::text::{draw_text, text_size};
use crate::storyboard::draw::{append_scene_sprites, StoryboardViewport};
use crate::storyboard::{Storyboard, Textures};

const LABEL_REFERENCE_WIDTH: f64 = 1280.0;
const LABEL_REFERENCE_HEIGHT: f64 = 720.0;

/// osu! 的视频淡入淡出时长（`DrawableStoryboardVideo` 的 `FadeIn(500)` / `FadeOut(500)`）。
pub const VIDEO_FADE_MS: i64 = 500;

/// 视频自身时间轴 `video_ms` 处的可见度：开始处 [`VIDEO_FADE_MS`] 淡入、
/// 结束前 [`VIDEO_FADE_MS`] 淡出，窗口外为 0（短视频短于两段淡入淡出时取
/// 两者更低值）。CLI 导出的 `media::background_video::visibility_alpha` 同公式。
pub fn visibility_alpha(video_ms: i64, duration_ms: i64) -> f64 {
    if video_ms < 0 || video_ms >= duration_ms.max(1) {
        return 0.0;
    }
    let fade_in = video_ms as f64 / VIDEO_FADE_MS as f64;
    let fade_out = (duration_ms - video_ms) as f64 / VIDEO_FADE_MS as f64;
    fade_in.min(fade_out).clamp(0.0, 1.0)
}

/// 背景视频层的来源。
///
/// 逐帧视频像素不一定要进 CPU 内存：`External` 走渲染器的外部纹理槽位，
/// 由后端直接从浏览器视频帧 GPU→GPU 拷贝（renderer 的
/// `copy_external_frame`），只有不具备 GPU 直拷能力的宿主才用 `Pixels`。
pub enum VideoSource<'a> {
    /// 像素帧（RGBA）与淡入淡出可见度。
    Pixels(&'a Arc<Img>, f32),
    /// 外部纹理槽位与淡入淡出可见度。
    External { slot: u32, alpha: f32 },
}

/// 故事板合成输入：已解析故事板、贴图、求值时刻（谱面毫秒，与 `current_ms`
/// 同坐标系）与用户暗度亮度（1 − dim，预乘进精灵颜色）。
pub struct StoryboardLayers<'a> {
    pub storyboard: &'a Storyboard,
    pub textures: &'a Textures,
    pub chart_ms: f64,
    /// 用户暗度的亮度（`BACKGROUND_DIM=0.7` 时为 0.3）；1.0 表示不变暗。
    pub brightness: f32,
}

#[allow(clippy::too_many_arguments)]
pub fn compose_video_scene(
    playfield: FrameScene,
    current_ms: i64,
    total_ms: i64,
    width: u32,
    height: u32,
    background: Option<&Arc<Img>>,
    background_video: Option<VideoSource<'_>>,
    storyboard: Option<StoryboardLayers<'_>>,
    style: VideoStyle,
) -> crate::domain::errors::Result<FrameScene> {
    let (scale, offset) = fit_playfield(playfield.width(), playfield.height(), width, height)?;
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
    // 背景视频叠在背景图上；alpha = 0 等价于不显示，直接不入场景省一次上传。
    let destination = SceneRect {
        x: 0.0,
        y: 0.0,
        width: width as f32,
        height: height as f32,
    };
    match background_video {
        Some(VideoSource::Pixels(frame, alpha)) if alpha > 0.0 => {
            builder.sprite(Arc::clone(frame), destination, alpha);
        }
        Some(VideoSource::External { slot, alpha }) if alpha > 0.0 => {
            builder.external_sprite(slot, destination, alpha);
        }
        _ => {}
    }
    // 故事板层序与 osu! 一致：Background/Pass/Foreground（underlay）压在背景之上、
    // playfield 之下；只有 Overlay 层压在 playfield 之上、时间标签之下。全部层都
    // 吃用户暗度（亮度预乘进精灵颜色，见 `StoryboardLayers::brightness`）。
    match storyboard.as_ref() {
        Some(layers) => {
            let view =
                StoryboardViewport::new(width as f32, height as f32, layers.storyboard.widescreen);
            let (behind, front) = layers.storyboard.sprites_at(layers.chart_ms);
            append_scene_sprites(
                &mut builder,
                &behind,
                layers.textures,
                &view,
                layers.brightness,
            );
            builder.append_scaled(&playfield, offset, scale);
            append_scene_sprites(
                &mut builder,
                &front,
                layers.textures,
                &view,
                layers.brightness,
            );
        }
        None => builder.append_scaled(&playfield, offset, scale),
    }

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

/// 把物件层等比缩放到铺满画布并居中（cover），多出来的部分由画布裁掉。
///
/// 四个模式的实时物件层都是按 [`crate::render::geometry::video_canvas`] 补齐的
/// 16:9 画布，与输出画布的宽高比只差「补齐到偶数像素」带来的零点几个百分点。
/// 这里必须用 cover 而不是 contain：contain 会在另一轴留下 1~4px 的缝，
/// 那条缝属于画布背景而不是物件层，FL 遮罩压不到它——看起来就是画面顶部/底部
/// （或左右）的一条亮边。裁掉的几个像素在补边区域，不影响物件。
fn fit_playfield(
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
    let scale = width_ratio.max(height_ratio);
    let scaled_width = playfield_width as f32 * scale;
    let scaled_height = playfield_height as f32 * scale;
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
    use super::{compose_video_scene, fit_playfield, visibility_alpha, VideoSource, VideoStyle};
    use crate::render::scene::{DrawCommand, FrameScene, SceneSize};

    /// 物件层铺满画布（cover）：宽高比一致时是 1:1，略有偏差时裁掉多出来的几个像素。
    #[test]
    fn playfield_covers_the_canvas_without_letterbox() {
        // 16:9 物件层铺到 16:9 画布：基本 1:1，位移只有补齐误差带来的零点几像素。
        let (scale, offset) = fit_playfield(684, 384, 1920, 1080).unwrap();
        assert!((scale - 2.8125).abs() < 0.01);
        let scaled = (684.0 * scale, 384.0 * scale);
        assert!(
            scaled.0 >= 1920.0 - 0.5 && scaled.1 >= 1080.0 - 0.5,
            "{scaled:?}"
        );
        assert!(offset[0].abs() < 5.0 && offset[1].abs() < 5.0, "{offset:?}");

        // taiko 补齐到偶数像素后略偏窄的 684×386 同样铺满，不会留下左右缝。
        let (scale, offset) = fit_playfield(684, 386, 1920, 1080).unwrap();
        assert!(684.0 * scale >= 1920.0 - 0.5, "{}", 684.0 * scale);
        assert!(offset[1] >= -3.0 && offset[1] <= 0.0, "{offset:?}");

        // 非 16:9 物件层：铺满后裁掉多出来的一侧（cover，而不是补边）。
        let (scale, offset) = fit_playfield(530, 384, 1280, 720).unwrap();
        assert!((scale - 1280.0 / 530.0).abs() < 0.001);
        assert!(offset[0].abs() < 0.01);
        assert!(offset[1] < 0.0, "高度方向应被裁掉而不是补边：{offset:?}");

        // 非正尺寸仍然报错。
        assert!(fit_playfield(0, 100, 1280, 720).is_err());
        assert!(fit_playfield(100, 0, 1280, 720).is_err());
    }

    /// 时间标签随输出分辨率缩放。
    #[test]
    fn time_label_scales_with_output_resolution() {
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
                None,
                None,
                VideoStyle::default(),
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

    /// 视频淡入淡出可见度：窗口外为 0，窗口内线性，短视频取两者更低值。
    #[test]
    fn visibility_alpha_matches_osu_fade_windows() {
        assert_eq!(visibility_alpha(-1, 10_000), 0.0);
        assert_eq!(visibility_alpha(0, 10_000), 0.0);
        assert_eq!(visibility_alpha(250, 10_000), 0.5);
        assert_eq!(visibility_alpha(500, 10_000), 1.0);
        assert_eq!(visibility_alpha(5_000, 10_000), 1.0);
        assert_eq!(visibility_alpha(9_750, 10_000), 0.5);
        assert_eq!(visibility_alpha(10_000, 10_000), 0.0);
        assert_eq!(visibility_alpha(250, 400), 0.3);
        assert_eq!(visibility_alpha(200, 400), 0.4);
    }

    /// 背景视频叠在背景图上、playfield 之下，携带宿主给定的淡入淡出 alpha。
    #[test]
    fn background_video_layers_over_the_background_image() {
        use crate::render::canvas::Img;
        use std::sync::Arc;

        let playfield = FrameScene::clear(
            SceneSize {
                width: 530,
                height: 384,
            },
            0,
            [0, 0, 0, 255],
        );
        let background = Arc::new(Img::new(2, 2, [10, 10, 10, 255]));
        let video_frame = Arc::new(Img::new(2, 2, [200, 200, 200, 255]));
        let scene = compose_video_scene(
            playfield,
            0,
            60_000,
            1280,
            720,
            Some(&background),
            Some(VideoSource::Pixels(&video_frame, 0.5)),
            None,
            VideoStyle::default(),
        )
        .unwrap();
        // 命令顺序：背景图 → 背景视频 → playfield → 时间标签。
        assert!(scene.commands.len() >= 4);
        let DrawCommand::Sprite { alpha, .. } = &scene.commands[1] else {
            panic!("第二条命令必须是背景视频精灵");
        };
        assert_eq!(*alpha, 0.5);
    }

    /// 外部纹理槽位的视频层发出 `ExternalSprite`（像素不进场景资源表）。
    #[test]
    fn external_video_layer_emits_slot_sprite() {
        let playfield = FrameScene::clear(
            SceneSize {
                width: 530,
                height: 384,
            },
            0,
            [0, 0, 0, 255],
        );
        let scene = compose_video_scene(
            playfield,
            0,
            60_000,
            1280,
            720,
            None,
            Some(VideoSource::External {
                slot: 7,
                alpha: 0.25,
            }),
            None,
            VideoStyle::default(),
        )
        .unwrap();
        let DrawCommand::ExternalSprite { slot, alpha, .. } = &scene.commands[1] else {
            panic!("第二条命令必须是外部纹理精灵");
        };
        assert_eq!(*slot, 7);
        assert_eq!(*alpha, 0.25);
    }

    /// 故事板层序：underlay（Background/Pass/Foreground）在 playfield 之下，
    /// 只有 Overlay 在其上。
    #[test]
    fn storyboard_layers_straddle_the_playfield() {
        use super::StoryboardLayers;
        use crate::storyboard::{parse_storyboard, Textures};
        use std::sync::Arc;

        let playfield = FrameScene::clear(
            SceneSize {
                width: 530,
                height: 384,
            },
            0,
            [0, 0, 0, 255],
        );
        let storyboard = parse_storyboard(
            "[Events]\n\
             Sprite,Foreground,Centre,\"behind.png\",320,240\n\
             _F,0,0,,1\n\
             Sprite,Overlay,Centre,\"front.png\",320,240\n\
             _F,0,0,,1",
            None,
        );
        let texture = Arc::new(crate::render::canvas::Img::new(2, 2, [255, 255, 255, 255]));
        let mut textures = Textures::new();
        textures.insert("behind.png".to_string(), Arc::clone(&texture));
        textures.insert("front.png".to_string(), texture);
        let layers = StoryboardLayers {
            storyboard: &storyboard,
            textures: &textures,
            chart_ms: 0.0,
            brightness: 1.0,
        };
        let scene = compose_video_scene(
            playfield,
            0,
            60_000,
            1280,
            720,
            None,
            None,
            Some(layers),
            VideoStyle::default(),
        )
        .unwrap();
        // 命令顺序：兜底底色 → 背景层故事板 → playfield → 前景层故事板 → 时间标签。
        let is_transformed =
            |command: &DrawCommand| matches!(command, DrawCommand::TransformedSprite { .. });
        assert!(
            is_transformed(&scene.commands[1]),
            "第二条必须是背景层故事板"
        );
        assert!(
            !is_transformed(&scene.commands[2]),
            "第三条是 playfield 矩形"
        );
        assert!(
            is_transformed(&scene.commands[3]),
            "第四条必须是前景层故事板"
        );
    }
}
