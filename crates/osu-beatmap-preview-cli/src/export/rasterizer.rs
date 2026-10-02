//! 场景的 CPU 参考光栅器，复用现有 `Img` 绘图原语。

#![cfg(test)]

use crate::export::canvas::Img;
use crate::export::scene::{DrawCommand, FrameScene, SceneRect};
use osu_beatmap_preview_core::storyboard::{draw_transformed_sprite, SpriteTransform};
use osu_beatmap_preview_core::support::error::{PreviewError, Result};

pub(crate) trait FrameBackend {
    fn render_frame(&mut self, scene: &FrameScene) -> Result<Img>;
}

#[derive(Default)]
pub(crate) struct CpuRasterizer;

impl FrameBackend for CpuRasterizer {
    fn render_frame(&mut self, scene: &FrameScene) -> Result<Img> {
        let mut target = Img::new(scene.size.width, scene.size.height, [0, 0, 0, 0]);
        let mut clips = vec![SceneRect {
            x: 0.0,
            y: 0.0,
            width: scene.size.width as f32,
            height: scene.size.height as f32,
        }];
        for command in scene.commands.iter() {
            match command {
                DrawCommand::PushClip(rect) => {
                    let parent = clips.last().copied().expect("裁剪栈始终保留画布边界");
                    clips.push(intersection(parent, *rect));
                }
                DrawCommand::PopClip => {
                    if clips.len() == 1 {
                        return Err(PreviewError::render("frame scene clip stack underflow"));
                    }
                    clips.pop();
                }
                DrawCommand::Sprite {
                    resource,
                    destination,
                    alpha,
                } => {
                    let source = scene.resources.get(resource).ok_or_else(|| {
                        PreviewError::render(format!(
                            "frame scene resource {} is missing",
                            resource.0
                        ))
                    })?;
                    let width = destination.width.round().max(1.0) as u32;
                    let height = destination.height.round().max(1.0) as u32;
                    let image = if source.w == width && source.h == height {
                        source.as_ref().clone()
                    } else {
                        source.resize(width, height)
                    };
                    let image = if *alpha < 1.0 {
                        image.scale_alpha((*alpha as f64).clamp(0.0, 1.0))
                    } else {
                        image
                    };
                    composite_clipped(
                        &mut target,
                        &image,
                        destination.x.round() as i64,
                        destination.y.round() as i64,
                        *clips.last().expect("裁剪栈始终非空"),
                    );
                }
                // 外部纹理槽位（浏览器视频帧 GPU 直拷）只存在于 GPU 渲染器；
                // CPU 参考光栅器没有对应像素，跳过即可（实时路径不经过这里）。
                DrawCommand::ExternalSprite { .. } => {}
                DrawCommand::TransformedSprite {
                    resource,
                    position,
                    origin,
                    size,
                    rotation,
                    color,
                    additive,
                } => {
                    let source = scene.resources.get(resource).ok_or_else(|| {
                        PreviewError::render(format!(
                            "frame scene resource {} is missing",
                            resource.0
                        ))
                    })?;
                    // 与 GPU 光栅器同一套几何语义：直接复用 core 的故事板光栅。
                    let clip = *clips.last().expect("裁剪栈始终非空");
                    let sprite = SpriteTransform {
                        position: *position,
                        size: *size,
                        origin: *origin,
                        rotation: *rotation,
                        colour: [
                            f32::from(color[0]) / 255.0,
                            f32::from(color[1]) / 255.0,
                            f32::from(color[2]) / 255.0,
                        ],
                        alpha: f32::from(color[3]) / 255.0,
                        additive: *additive,
                    };
                    draw_transformed_sprite(
                        &mut target,
                        source,
                        &sprite,
                        [clip.x, clip.y, clip.x + clip.width, clip.y + clip.height],
                    );
                }
                DrawCommand::Rectangle { rect, color } => {
                    let rect = intersection(*clips.last().expect("裁剪栈始终非空"), *rect);
                    target.fill_rect_size(
                        rect.x.round() as i64,
                        rect.y.round() as i64,
                        rect.width.round() as i64,
                        rect.height.round() as i64,
                        *color,
                    );
                }
                DrawCommand::Circle {
                    center,
                    radius,
                    color,
                } => target.fill_circle_aa(
                    center[0] as f64,
                    center[1] as f64,
                    *radius as f64,
                    *color,
                ),
                DrawCommand::Ring {
                    center,
                    radius,
                    thickness,
                    color,
                } => target.stroke_circle_aa(
                    center[0] as f64,
                    center[1] as f64,
                    *radius as f64,
                    *thickness as f64,
                    *color,
                ),
                DrawCommand::Line {
                    from,
                    to,
                    thickness,
                    color,
                } => target.draw_line(
                    from[0] as f64,
                    from[1] as f64,
                    to[0] as f64,
                    to[1] as f64,
                    *thickness as f64,
                    *color,
                ),
                DrawCommand::Glyph {
                    resource,
                    destination,
                    color,
                } => {
                    let source = scene.resources.get(resource).ok_or_else(|| {
                        PreviewError::render(format!("frame scene glyph {} is missing", resource.0))
                    })?;
                    let mut glyph = source.as_ref().clone();
                    for pixel in glyph.data.chunks_exact_mut(4) {
                        pixel[0] = color[0];
                        pixel[1] = color[1];
                        pixel[2] = color[2];
                        pixel[3] = ((pixel[3] as u16 * color[3] as u16) / 255) as u8;
                    }
                    composite_clipped(
                        &mut target,
                        &glyph,
                        destination.x.round() as i64,
                        destination.y.round() as i64,
                        *clips.last().expect("裁剪栈始终非空"),
                    );
                }
                DrawCommand::SliderMesh {
                    vertices,
                    thickness,
                    border,
                    body,
                } => {
                    for points in vertices.windows(2) {
                        target.draw_line(
                            points[0][0] as f64,
                            points[0][1] as f64,
                            points[1][0] as f64,
                            points[1][1] as f64,
                            *thickness as f64,
                            *border,
                        );
                        target.draw_line(
                            points[0][0] as f64,
                            points[0][1] as f64,
                            points[1][0] as f64,
                            points[1][1] as f64,
                            (*thickness * 0.72) as f64,
                            *body,
                        );
                    }
                }
            }
        }
        if clips.len() != 1 {
            return Err(PreviewError::render("frame scene clip stack is unbalanced"));
        }
        Ok(target)
    }
}

fn intersection(left: SceneRect, right: SceneRect) -> SceneRect {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.width).min(right.x + right.width);
    let bottom_edge = (left.y + left.height).min(right.y + right.height);
    SceneRect {
        x,
        y,
        width: (right_edge - x).max(0.0),
        height: (bottom_edge - y).max(0.0),
    }
}

fn composite_clipped(target: &mut Img, source: &Img, x: i64, y: i64, clip: SceneRect) {
    let clip_x = clip.x.round().max(0.0) as i64;
    let clip_y = clip.y.round().max(0.0) as i64;
    let clip_right = (clip.x + clip.width).round().min(target.w as f32) as i64;
    let clip_bottom = (clip.y + clip.height).round().min(target.h as f32) as i64;
    let left = x.max(clip_x);
    let top = y.max(clip_y);
    let right = (x + source.w as i64).min(clip_right);
    let bottom = (y + source.h as i64).min(clip_bottom);
    if left >= right || top >= bottom {
        return;
    }
    let cropped = source.crop(
        (left - x) as u32,
        (top - y) as u32,
        (right - x) as u32,
        (bottom - y) as u32,
    );
    target.alpha_composite(&cropped, left, top);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::scene::FrameScene;

    #[test]
    fn realtime_visibility_mods_change_pixels_and_survive_seeking() {
        use osu_beatmap_preview_core::{
            domain::mods::parse_mods, domain::parser::parse_beatmap_bytes,
        };
        for mode in 0..=3 {
            let objects = if mode == 3 {
                "64,192,1000,1,0,0:0:0:0:\n192,192,1300,128,0,3000:0:0:0:0:\n320,192,1600,1,0,0:0:0:0:\n"
            } else {
                "80,96,1000,1,0,0:0:0:0:\n300,220,1300,1,8,0:0:0:0:\n180,300,1600,2,0,L|350:280,2,170\n"
            };
            let source = format!("osu file format v14\n\n[General]\nMode:{mode}\n\n[Difficulty]\nCircleSize:4\nApproachRate:6\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n{objects}");
            let beatmap = parse_beatmap_bytes(source.as_bytes()).unwrap();
            let normal = realtime_source(&beatmap, None);
            let normal = CpuRasterizer
                .render_frame(&normal.render(1200).unwrap())
                .unwrap();
            for code in ["HD", "FL"] {
                let mods = parse_mods(&[code.into()]).unwrap();
                let source = realtime_source(&beatmap, Some(&mods));
                let expected = CpuRasterizer
                    .render_frame(&source.render(1200).unwrap())
                    .unwrap();
                assert!(normal.data != expected.data, "mode={mode}, mod={code}");
                for time in [300, 1800, 1200] {
                    source.render(time).unwrap();
                }
                let repeated = CpuRasterizer
                    .render_frame(&source.render(1200).unwrap())
                    .unwrap();
                assert_eq!(expected.data, repeated.data);
            }
        }
    }

    /// FL 必须压暗整个输出画布：lazer 把遮罩加到 `drawableRuleset.Overlays`（整屏），
    /// MP4 导出同样在整张 16:9 画布上压暗。实时链路一度把物件层画在内容框里，
    /// 合成阶段补出的背景（playfield 之外）就不会被压暗——表现为「FL 覆盖不全」；
    /// 另一条同源问题：物件层按 contain 缩放时会在另一轴留下 1~4px 的缝，
    /// 那条缝同样压不到，看起来是画面边缘的一条亮边。
    #[test]
    fn flashlight_darkens_the_whole_canvas_outside_the_playfield() {
        use osu_beatmap_preview_core::{
            domain::mods::parse_mods,
            domain::parser::parse_beatmap_bytes,
            render::canvas::Img,
            render::scene::DrawCommand,
            render::wgpu::{composition::compose_video_scene, VideoStyle},
        };
        use std::sync::Arc;

        let cases = [
            (0, "80,96,1000,1,0,0:0:0:0:\n300,220,1300,1,0,0:0:0:0:\n"),
            (1, "80,96,1000,1,0,0:0:0:0:\n300,220,1300,1,0,0:0:0:0:\n"),
            (2, "80,96,1000,1,0,0:0:0:0:\n300,220,1300,1,0,0:0:0:0:\n"),
            (3, "64,192,1000,1,0,0:0:0:0:\n192,192,1300,1,0,0:0:0:0:\n"),
        ];
        // 纯白背景：补边处只要被压暗，像素就会明显变黑。
        let background = Arc::new(Img::new(2, 2, [255, 255, 255, 255]));
        for (mode, objects) in cases {
            let text = format!(
                "osu file format v14\n\n[General]\nMode:{mode}\n\n[Difficulty]\nCircleSize:4\nApproachRate:6\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n{objects}"
            );
            let beatmap = parse_beatmap_bytes(text.as_bytes()).unwrap();
            let scene = |mods| {
                let source = realtime_source(&beatmap, mods);
                let playfield = source.render(1200).unwrap();
                let size = (playfield.width(), playfield.height());
                (
                    compose_video_scene(
                        playfield,
                        1200,
                        1000,
                        1280,
                        720,
                        Some(&background),
                        None,
                        None,
                        VideoStyle::default(),
                    )
                    .unwrap(),
                    size,
                )
            };
            let flashlight = parse_mods(&["FL".into()]).unwrap();
            let (plain, _) = scene(None);
            let (masked, playfield_size) = scene(Some(&flashlight));
            // 遮罩精灵就是物件层本身（图像尺寸 = 物件层尺寸）：它必须铺满整张画布，
            // 否则没被盖住的那条缝在 FL 下会露出背景。
            let mask = masked
                .commands
                .iter()
                .filter_map(|command| match command {
                    DrawCommand::Sprite {
                        resource,
                        destination,
                        ..
                    } => Some((&masked.resources[resource], *destination)),
                    _ => None,
                })
                .find(|(image, _)| (image.w, image.h) == playfield_size)
                .map(|(_, destination)| destination)
                .expect("FL 必须产出物件层大小的遮罩精灵");
            assert!(
                mask.x <= 0.0
                    && mask.y <= 0.0
                    && mask.x + mask.width >= 1280.0
                    && mask.y + mask.height >= 720.0,
                "mode={mode}：FL 遮罩必须铺满画布，实际 {mask:?}"
            );

            let frame = |scene| CpuRasterizer.render_frame(&scene).unwrap();
            let plain = frame(plain);
            let masked = frame(masked);
            // 四角都在 playfield 之外（右上角避开合成阶段画上去的时间标签）。
            for (x, y) in [(4, 4), (1275, 4), (4, 715), (1275, 715)] {
                assert_eq!(
                    plain.get(x, y),
                    [255, 255, 255, 255],
                    "mode={mode}：基准帧的边缘应是背景原色"
                );
                assert_eq!(
                    masked.get(x, y),
                    [0, 0, 0, 255],
                    "mode={mode}：画布边缘必须被 FL 压暗"
                );
            }
        }
    }

    /// Mania 的 HD/FL 分层必须与 lazer 一致：HD 只改音符层的 alpha
    /// （lazer 把 `HitObjectContainer` 包进 `PlayfieldCoveringWrapper`），
    /// FL 是整帧遮罩（`ModFlashlight` 把遮罩加到 `drawableRuleset.Overlays`，
    /// 压在键道底色和判定线之上，GIF/MP4 导出同理）。
    /// 这条断言同时阻止两种误改：把 FL 改成"只压暗音符层"，或在仅开 FL 时
    /// 额外建立音符层并叠加 HD 遮罩。
    #[test]
    fn mania_realtime_hidden_and_flashlight_layers_match_lazer() {
        use osu_beatmap_preview_core::{
            domain::mods::{parse_mods, ModSettings},
            domain::parser::parse_beatmap_bytes,
            render::cpu::modes::mania::animation::{build_video_layout, segment_left},
            render::cpu::modes::mania::skin::load_mania_skin_config,
            render::geometry::OutputFormat,
            render::scene::DrawCommand,
        };
        let map = parse_beatmap_bytes(
            b"osu file format v14\n[General]\nMode:3\n[Difficulty]\nCircleSize:4\nApproachRate:5\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n64,192,1000,1,0,0:0:0:0:\n192,192,1500,1,0,0:0:0:0:\n",
        )
        .unwrap();
        // 实时物件层与 MP4 导出一样是 16:9 画布，FL 遮罩因此能盖住整帧。
        let layout = build_video_layout(
            &load_mania_skin_config(4, OutputFormat::Mp4),
            OutputFormat::Mp4,
        );
        let left = segment_left(0, &layout);
        // 键道左侧面板与判定线都在 FL 可视带（playfield 纵向中点 ± 半径）之外。
        let panel = (
            (left + layout.left_panel_width / 2) as u32,
            (layout.playfield_top + 5) as u32,
        );
        let judgement = (
            (left + 5) as u32,
            (layout.playfield_top + layout.hit_position_y) as u32,
        );
        let render = |mods: Option<&ModSettings>| {
            CpuRasterizer
                .render_frame(&realtime_source(&map, mods).render(1000).unwrap())
                .unwrap()
        };
        let baseline = render(None);
        let hd = render(Some(&parse_mods(&["HD".into()]).unwrap()));
        let fl = render(Some(&parse_mods(&["FL".into()]).unwrap()));

        // HD 只作用于音符层：键道底色与判定线逐像素不变。
        assert_eq!(hd.get(panel.0, panel.1), baseline.get(panel.0, panel.1));
        assert_eq!(
            hd.get(judgement.0, judgement.1),
            baseline.get(judgement.0, judgement.1)
        );
        assert_ne!(hd.data, baseline.data, "HD 必须改变覆盖带内的音符");

        // FL 是整帧遮罩：键道底色与判定线一起被压暗。
        assert_eq!(fl.get(panel.0, panel.1), [0, 0, 0, 255]);
        assert_ne!(
            fl.get(judgement.0, judgement.1),
            baseline.get(judgement.0, judgement.1)
        );
        assert_eq!(fl.get(judgement.0, judgement.1), [0, 0, 0, 255]);

        // 仅开 FL 时不得额外建立音符层：整帧精灵只有 FL 遮罩本身。
        let full_frame_sprites = realtime_source(&map, Some(&parse_mods(&["FL".into()]).unwrap()))
            .render(1000)
            .unwrap()
            .commands
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    DrawCommand::Sprite { destination, .. }
                        if destination.x == 0.0
                            && destination.y == 0.0
                            && destination.width == layout.image_width as f32
                            && destination.height == layout.image_height as f32
                )
            })
            .count();
        assert_eq!(
            full_frame_sprites, 1,
            "仅开 FL 时只应有一个整帧精灵（FL 遮罩），不应额外分层叠加 HD 遮罩"
        );
    }

    #[test]
    fn catch_hidden_kiai_is_visible_in_realtime_and_obeys_flashlight() {
        use osu_beatmap_preview_core::{
            domain::mods::parse_mods, domain::parser::parse_beatmap_bytes,
        };
        let map = parse_beatmap_bytes(
            b"osu file format v14\n[General]\nMode:2\n[Difficulty]\nCircleSize:5\nApproachRate:5\n[TimingPoints]\n0,500,4,1,0,100,1,0\n750,-100,4,1,0,100,0,1\n1300,-100,4,1,0,100,0,0\n[HitObjects]\n0,192,800,1,0,0:0:0:0:\n512,192,1500,1,0,0:0:0:0:\n512,192,2500,1,0,0:0:0:0:\n",
        ).unwrap();
        let mut plain = map.clone();
        for point in &mut plain.timing_points {
            point.kiai_mode = false;
        }
        let hd = parse_mods(&["HD".into()]).unwrap();
        let normal = realtime_source(&plain, Some(&hd));
        let kiai = realtime_source(&map, Some(&hd));
        let pixels = |source: &osu_beatmap_preview_core::render::wgpu::RealtimeFrameSource,
                      time| {
            CpuRasterizer
                .render_frame(&source.render(time).unwrap())
                .unwrap()
                .data
        };
        let expected = pixels(&kiai, 1000);
        assert_ne!(
            expected,
            pixels(&normal, 1000),
            "WGPU 场景必须保留 Kiai 中淡淡的水果轮廓"
        );
        assert_eq!(pixels(&kiai, 1300), pixels(&normal, 1300));
        kiai.render(2400).unwrap();
        assert_eq!(expected, pixels(&kiai, 1000));
        let hdfl = parse_mods(&["HD".into(), "FL".into()]).unwrap();
        let masked = realtime_source(&map, Some(&hdfl));
        let plain_masked = realtime_source(&plain, Some(&hdfl));
        assert_eq!(
            pixels(&masked, 1000),
            pixels(&plain_masked, 1000),
            "FL 外的 Kiai 轮廓仍必须隐藏"
        );
    }

    fn realtime_source(
        beatmap: &osu_beatmap_preview_core::domain::models::Beatmap,
        mods: Option<&osu_beatmap_preview_core::domain::mods::ModSettings>,
    ) -> osu_beatmap_preview_core::render::wgpu::RealtimeFrameSource {
        use osu_beatmap_preview_core::render::wgpu;
        match beatmap.mode() {
            0 => wgpu::prepare_standard(
                beatmap,
                mods,
                osu_beatmap_preview_core::domain::shared::time_selection::TimeAxis::new(0),
            ),
            1 => wgpu::prepare_taiko(beatmap, mods),
            2 => wgpu::prepare_catch(beatmap, mods),
            3 => wgpu::prepare_mania(beatmap, mods),
            _ => unreachable!(),
        }
        .unwrap()
    }

    /// 图像场景经参考光栅器后像素保持不变。
    #[test]
    fn image_scene_round_trips_through_reference_backend() {
        let source = Img::new(3, 2, [10, 20, 30, 128]);
        let scene = FrameScene::from_image(source.clone(), 0);
        let rendered = CpuRasterizer.render_frame(&scene).unwrap();
        assert_eq!(rendered.data, source.data);
    }
}
