//! Taiko WGPU 实时帧源准备。

use crate::domain::errors::{PreviewError, Result};
use crate::domain::models::Beatmap;
use crate::domain::mods::ModSettings;
use crate::render::cpu::modes::taiko::animation::measure_line_alpha;
use crate::render::cpu::modes::taiko::animation_render::{
    build_multiplier_points, build_video_animation_layout, compute_time_range,
    draw_hit_object_state, gif_row_center_y, gif_row_top, judgement_line_x, object_x,
    prepare_hit_objects_with_mods, prepare_measure_lines, taiko_flashlight,
    taiko_visibility_timeline, AnimationLayout, MultiplierLookup, PreparedAnimationPoint,
    PreparedTaikoHitObject,
};
use crate::render::cpu::modes::taiko::constants::*;
use crate::render::cpu::modes::taiko::timing::{
    apply_taiko_object_mods, effective_overall_difficulty, effective_slider_multiplier,
    effective_timing_points, taiko_hit_objects,
};
use crate::render::geometry::{GameMode, OutputFormat};
use crate::render::scene::{FrameSceneBuilder, SceneRect};
use crate::render::wgpu::RealtimeFrameSource;
use std::cell::RefCell;
#[cfg(any(test, feature = "replay"))]
use std::sync::Arc;

thread_local! {
    // 实时转盘的直径随动画变化，按线程复用圆环缓存，避免每帧重复光栅化。
    static TAIKO_REALTIME_CACHE: RefCell<crate::render::cpu::modes::taiko::notes::RenderCache> =
        RefCell::new(crate::render::cpu::modes::taiko::notes::RenderCache::default());
}

#[cfg(feature = "replay")]
pub fn prepare_replay(
    prepared: Arc<crate::replay::preparation::taiko::PreparedTaiko>,
    gameplay: Arc<crate::gameplay::timeline::GameplayTimeline>,
    font: Option<crate::render::shared::replay::font::ArgonFont>,
    dimensions: (u32, u32),
) -> Result<RealtimeFrameSource> {
    let plan = crate::render::shared::taiko::replay::ReplayRenderPlan::new(
        prepared,
        gameplay,
        font,
        Some(dimensions),
    )?;
    Ok(RealtimeFrameSource::new(GameMode::Taiko, move |time| {
        let mut scene = FrameSceneBuilder::new(dimensions.0, dimensions.1, time);
        plan.draw_scene(&mut scene, time);
        plan.draw_scene_overlay(&mut scene, time);
        Ok(scene.finish())
    }))
}

/// 预计算 Taiko 滚动倍率、物件和小节线，供任意时间点重复取景。
pub fn prepare_realtime(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
) -> Result<RealtimeFrameSource> {
    let hit_objects = apply_taiko_object_mods(taiko_hit_objects(beatmap), mods);
    if hit_objects.is_empty() {
        return Err(PreviewError::render("taiko beatmap has no hit objects"));
    }
    let slider_multiplier = effective_slider_multiplier(beatmap, mods)?;
    let timing_points = effective_timing_points(beatmap, mods);
    let multiplier_lookup = MultiplierLookup {
        points: build_multiplier_points(&timing_points, slider_multiplier),
    };
    let prepared_hit_objects = prepare_hit_objects_with_mods(
        &hit_objects,
        &multiplier_lookup,
        &timing_points,
        beatmap.difficulty.get_f64_or("SliderTickRate", 1.0),
        effective_overall_difficulty(beatmap, mods),
        mods,
    );
    let prepared_measure_lines = prepare_measure_lines(
        &hit_objects,
        &timing_points,
        &multiplier_lookup,
        crate::config::current()
            .render
            .taiko
            .mp4
            .style
            .SHOW_MEASURE_LINES,
    );
    // 与 MP4 导出用同一套画布布局：物件层就是 16:9 画布本身，
    // 否则 FL 遮罩只盖住 playfield，合成阶段补出的背景不会被压暗。
    let layout = build_video_animation_layout(compute_time_range(), OutputFormat::Mp4);
    let flashlight = mods
        .is_some_and(|mods| mods.flashlight)
        .then(|| taiko_visibility_timeline(&hit_objects, &beatmap.break_periods));
    let autoplay =
        crate::render::cpu::modes::taiko::autoplay::Autoplay::new(&prepared_hit_objects, &layout);
    Ok(RealtimeFrameSource::new(
        GameMode::Taiko,
        move |absolute_time_ms| {
            let mut scene = FrameSceneBuilder::new(
                layout.image_width as u32,
                layout.image_height as u32,
                absolute_time_ms,
            );
            draw_background_scene(&mut scene, &layout);
            draw_objects_scene(
                &mut scene,
                &prepared_hit_objects,
                &prepared_measure_lines,
                &layout,
                absolute_time_ms,
            );
            autoplay.draw_scene(&mut scene, &layout, absolute_time_ms);
            if let Some(timeline) = &flashlight {
                taiko_flashlight(timeline, absolute_time_ms, &layout, 0).draw_scene(
                    &mut scene,
                    crate::render::geometry::PixelRect {
                        x: 0,
                        y: 0,
                        width: layout.image_width,
                        height: layout.image_height,
                    },
                );
            }
            Ok(scene.finish())
        },
    ))
}

fn rect(x: i64, y: i64, width: i64, height: i64) -> SceneRect {
    SceneRect {
        x: x as f32,
        y: y as f32,
        width: width.max(0) as f32,
        height: height.max(0) as f32,
    }
}

fn draw_background_scene(scene: &mut FrameSceneBuilder, layout: &AnimationLayout) {
    let top = gif_row_top(0, layout);
    let left = layout.playfield_left;
    let panel_width = layout.left_panel_width;
    scene.rectangle(
        rect(left, top, panel_width, layout.row_height),
        [30, 30, 30, 255],
    );
    let stripe = ((panel_width as f64 * 0.12) as i64).max(2);
    scene.rectangle(
        rect(left, top, stripe, layout.row_height),
        layout.track_accent_color,
    );
    scene.rectangle(
        rect(left + panel_width - stripe, top, stripe, layout.row_height),
        layout.track_accent_color,
    );
    let panel_center = [
        (left as f64 + panel_width as f64 / 2.0) as f32,
        (top as f64 + layout.row_height as f64 / 2.0) as f32,
    ];
    let drum_radius = layout.row_height.min(panel_width) as f64 * 0.36;
    scene.circle(
        panel_center,
        (drum_radius + (1.5 * layout.render_scale).max(0.5)) as f32,
        [20, 20, 20, 255],
    );
    scene.circle(panel_center, drum_radius as f32, [248, 238, 220, 255]);
    let center_line_width = crate::render::geometry::scale_stroke_px(2.0, layout.render_scale);
    scene.rectangle(
        rect(
            panel_center[0].round() as i64 - center_line_width / 2,
            (panel_center[1] as f64 - drum_radius).round() as i64,
            center_line_width,
            (2.0 * drum_radius).round() as i64,
        ),
        [180, 165, 135, 255],
    );

    let track_left = left + panel_width;
    scene.rectangle(
        rect(track_left, top, layout.right_panel_width, layout.row_height),
        layout.track_background_color,
    );
    let edge = crate::render::geometry::scale_stroke_px(1.0, layout.render_scale)
        .clamp(1, layout.row_height);
    scene.rectangle(
        rect(track_left, top, layout.right_panel_width, edge),
        layout.track_edge_color,
    );
    scene.rectangle(
        rect(
            track_left,
            top + layout.row_height - edge,
            layout.right_panel_width,
            edge,
        ),
        layout.track_edge_color,
    );
    let diameter = layout.normal_note_diameter as f32;
    scene.ring(
        [
            judgement_line_x(layout) as f32,
            gif_row_center_y(0, layout) as f32,
        ],
        diameter / 2.0,
        diameter * NOTE_RING_THICKNESS_RATIO as f32,
        layout.judgement_line_color,
    );
}

fn draw_objects_scene(
    scene: &mut FrameSceneBuilder,
    hit_objects: &[PreparedTaikoHitObject],
    measure_lines: &[PreparedAnimationPoint],
    layout: &AnimationLayout,
    snapshot_time: i64,
) {
    let clip_left = judgement_line_x(layout);
    let clip_right = layout.playfield_left + layout.left_panel_width + layout.right_panel_width;
    draw_measure_lines_scene(
        scene,
        measure_lines,
        layout,
        snapshot_time,
        clip_left,
        clip_right,
    );
    TAIKO_REALTIME_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        for hit_object in hit_objects.iter().rev() {
            if crate::render::cpu::modes::taiko::animation_render::can_skip(
                hit_object,
                snapshot_time,
                layout,
                clip_left,
                clip_right,
            ) {
                continue;
            }

            draw_hit_object_state::<bool>(
                scene,
                hit_object,
                layout,
                0,
                snapshot_time,
                &mut cache,
                None,
            );
        }
    });
}

fn draw_measure_lines_scene(
    scene: &mut FrameSceneBuilder,
    measure_lines: &[PreparedAnimationPoint],
    layout: &AnimationLayout,
    snapshot_time: i64,
    left_bound: i64,
    right_bound: i64,
) {
    let height = (layout.row_height as f64 * MEASURE_LINE_HEIGHT_RATIO)
        .round()
        .max(1.0) as i64;
    let top = gif_row_center_y(0, layout) - height / 2;
    let width =
        crate::render::geometry::scale_stroke_px(MEASURE_LINE_WIDTH as f64, layout.render_scale);
    for line in measure_lines {
        let alpha = measure_line_alpha(line.time, snapshot_time);
        if alpha <= 0.0 {
            continue;
        }
        let center = object_x(line.time, snapshot_time as f64, line.multiplier, layout);
        let left = (center - width / 2).max(left_bound);
        let right = (left + width).min(right_bound);
        if right <= left {
            continue;
        }
        let mut color = ANIMATION_MEASURE_LINE_COLOR;
        color[3] = (color[3] as f64 * alpha).round().clamp(0.0, 255.0) as u8;
        scene.rectangle(rect(left, top, right - left, height), color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{canvas::Img, scene::DrawCommand};

    #[test]
    fn realtime_uses_a_judgement_ring_and_does_not_clip_swell_to_the_track() {
        let layout = build_video_animation_layout(1000.0, OutputFormat::Mp4);
        let mut builder =
            FrameSceneBuilder::new(layout.image_width as u32, layout.image_height as u32, 1200);
        draw_background_scene(&mut builder, &layout);
        let scene = builder.finish();
        assert!(scene.commands.iter().any(|command| matches!(command, DrawCommand::Ring { center, .. } if center[0] == judgement_line_x(&layout) as f32 && center[1] == gif_row_center_y(0, &layout) as f32)));

        let objects = crate::render::cpu::modes::taiko::animation_render::prepare_hit_objects(
            &[crate::domain::models::TaikoHitObject {
                start_time: 1000,
                end_time: 4000,
                hit_type: SWELL_FLAG,
                hitsound: 0,
                samples: Vec::new(),
            }],
            &MultiplierLookup {
                points: build_multiplier_points(&[], 1.0),
            },
            &[],
            1.0,
        );
        let mut builder =
            FrameSceneBuilder::new(layout.image_width as u32, layout.image_height as u32, 1200);
        draw_objects_scene(&mut builder, &objects, &[], &layout, 1200);
        let scene = builder.finish();
        assert!(!scene
            .commands
            .iter()
            .any(|command| matches!(command, DrawCommand::PushClip(_))));
        assert!(scene
            .commands
            .iter()
            .any(|command| matches!(command, DrawCommand::Sprite { .. })));
    }
    #[test]
    fn swell_local_layer_matches_full_canvas_with_hidden_and_scales() {
        for scale in [0.5, 1.0, 1.5, 2.0] {
            let mut config = crate::config::CoreConfig::default();
            config.render.taiko.mp4.SCALE = scale;
            crate::config::with_config(Arc::new(config), || {
                let layout = build_video_animation_layout(1000.0, OutputFormat::Mp4);
                let objects =
                    crate::render::cpu::modes::taiko::animation_render::prepare_hit_objects(
                        &[crate::domain::models::TaikoHitObject {
                            start_time: 1000,
                            end_time: 4000,
                            hit_type: SWELL_FLAG,
                            hitsound: 0,
                            samples: Vec::new(),
                        }],
                        &MultiplierLookup {
                            points: build_multiplier_points(&[], 1.0),
                        },
                        &[],
                        1.0,
                    );
                for hidden in [false, true] {
                    let mut object = objects[0].clone();
                    object.hidden = hidden;
                    for time in [400, 999, 1000, 1100, 1199, 1200, 1500, 2250, 2500] {
                        let mut expected = Img::new(
                            layout.image_width as u32,
                            layout.image_height as u32,
                            [0, 0, 0, 0],
                        );
                        crate::render::cpu::modes::taiko::animation_render::draw_hit_object(
                            &mut expected,
                            &object,
                            &layout,
                            0,
                            time,
                            &mut crate::render::cpu::modes::taiko::notes::RenderCache::default(),
                        );
                        let mut builder = FrameSceneBuilder::new(expected.w, expected.h, time);
                        TAIKO_REALTIME_CACHE.with(|cache| {
                            draw_hit_object_state::<bool>(
                                &mut builder,
                                &object,
                                &layout,
                                0,
                                time,
                                &mut cache.borrow_mut(),
                                None,
                            )
                        });
                        let scene = builder.finish();
                        let mut actual = Img::new(expected.w, expected.h, [0, 0, 0, 0]);
                        for command in scene.commands.iter() {
                            if let DrawCommand::Sprite {
                                resource,
                                destination,
                                ..
                            } = command
                            {
                                let sprite = &scene.resources[resource];
                                assert!(
                                    sprite.h < layout.image_height as u32,
                                    "转盘图层只应覆盖圆环范围"
                                );
                                actual.alpha_composite(
                                    sprite,
                                    destination.x as i64,
                                    destination.y as i64,
                                );
                            }
                        }
                        let difference = actual
                            .data
                            .iter()
                            .zip(&expected.data)
                            .position(|(a, b)| a != b);
                        assert!(difference.is_none(), "局部图层不能改变转盘像素：{scale}/{time}/{hidden}，首个差异 {difference:?}");
                    }
                }
            });
        }
    }
    #[test]
    fn realtime_roll_head_is_outside_tail_clip_and_expires_at_end() {
        for strong in [false, true] {
            let layout = build_video_animation_layout(1000.0, OutputFormat::Mp4);
            let objects = crate::render::cpu::modes::taiko::animation_render::prepare_hit_objects(
                &[crate::domain::models::TaikoHitObject {
                    start_time: 1000,
                    end_time: 3000,
                    hit_type: DRUMROLL_FLAG,
                    hitsound: if strong { HIT_SOUNDS_STRONG } else { 0 },
                    samples: Vec::new(),
                }],
                &MultiplierLookup {
                    points: build_multiplier_points(&[], 1.0),
                },
                &[],
                1.0,
            );
            for time in [1000, 2200, 2999] {
                let mut builder = FrameSceneBuilder::new(
                    layout.image_width as u32,
                    layout.image_height as u32,
                    time,
                );
                draw_objects_scene(&mut builder, &objects, &[], &layout, time);
                let scene = builder.finish();
                let mut depth = 0;
                let mut found_head = false;
                let diameter = if strong {
                    layout.big_note_diameter
                } else {
                    layout.normal_note_diameter
                };
                for command in scene.commands.iter() {
                    match command {
                        DrawCommand::PushClip(_) => depth += 1,
                        DrawCommand::PopClip => depth -= 1,
                        DrawCommand::Sprite {
                            resource,
                            destination,
                            ..
                        } if scene.resources[resource].w == diameter as u32
                            && scene.resources[resource].h == diameter as u32
                            && destination.x + destination.width / 2.0
                                == judgement_line_x(&layout) as f32
                            && destination.y + destination.height / 2.0
                                == gif_row_center_y(0, &layout) as f32 =>
                        {
                            assert_eq!(depth, 0, "圆头不能被轨道裁掉左半边");
                            found_head = true;
                        }
                        _ => {}
                    }
                }
                assert!(found_head, "活动滚奏必须保留圆头");
            }
            let mut builder =
                FrameSceneBuilder::new(layout.image_width as u32, layout.image_height as u32, 3000);
            draw_objects_scene(&mut builder, &objects, &[], &layout, 3000);
            assert!(builder.finish().commands.is_empty());
        }
    }
}
