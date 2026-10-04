//! Taiko WGPU 实时帧源准备。

use crate::domain::errors::{PreviewError, Result};
use crate::domain::models::Beatmap;
use crate::domain::mods::ModSettings;
use crate::render::canvas::Img;
use crate::render::cpu::modes::taiko::animation::{drum_roll_tick_transform, measure_line_alpha};
use crate::render::cpu::modes::taiko::animation_render::{
    build_multiplier_points, build_video_animation_layout, compute_time_range, gif_row_center_y,
    gif_row_top, hidden_alpha, judgement_line_x, object_x, prepare_hit_objects_with_mods,
    prepare_measure_lines, taiko_flashlight, taiko_visibility_timeline, AnimationLayout,
    MultiplierLookup, PreparedAnimationPoint, PreparedTaikoHitObject,
};
use crate::render::cpu::modes::taiko::constants::*;
use crate::render::cpu::modes::taiko::timing::{
    apply_taiko_object_mods, effective_slider_multiplier, effective_timing_points,
    taiko_hit_objects,
};
use crate::render::geometry::{GameMode, OutputFormat};
use crate::render::scene::{FrameSceneBuilder, SceneRect};
use crate::render::wgpu::RealtimeFrameSource;
use std::sync::Arc;

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
    let hidden_sprites: Vec<_> = prepared_hit_objects
        .iter()
        .map(|object| {
            (object.hidden && object.hit_object.hit_type & (DRUMROLL_FLAG | SWELL_FLAG) == 0).then(
                || {
                    let sound = object.hit_object.hitsound;
                    Arc::new(crate::render::cpu::modes::taiko::notes::build_note_disc(
                        if sound & HIT_SOUNDS_RIM != 0 {
                            RIM_NOTE_COLOR
                        } else {
                            CENTRE_NOTE_COLOR
                        },
                        if sound & HIT_SOUNDS_STRONG != 0 {
                            layout.big_note_diameter
                        } else {
                            layout.normal_note_diameter
                        },
                        crate::render::geometry::scale_stroke_px(1.0, layout.render_scale),
                        false,
                    ))
                },
            )
        })
        .collect();
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
                &hidden_sprites,
            );
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
    let judgement_width = crate::render::geometry::scale_stroke_px(3.0, layout.render_scale);
    scene.rectangle(
        rect(
            judgement_line_x(layout) - judgement_width / 2,
            top,
            judgement_width,
            layout.row_height,
        ),
        layout.judgement_line_color,
    );
}

fn draw_objects_scene(
    scene: &mut FrameSceneBuilder,
    hit_objects: &[PreparedTaikoHitObject],
    measure_lines: &[PreparedAnimationPoint],
    layout: &AnimationLayout,
    snapshot_time: i64,
    hidden_sprites: &[Option<Arc<Img>>],
) {
    let clip_left = judgement_line_x(layout);
    let clip_right = layout.playfield_left + layout.left_panel_width + layout.right_panel_width;
    let top = gif_row_top(0, layout);
    draw_measure_lines_scene(
        scene,
        measure_lines,
        layout,
        snapshot_time,
        clip_left,
        clip_right,
    );
    for (index, hit_object) in hit_objects.iter().enumerate().rev() {
        if crate::render::cpu::modes::taiko::animation_render::can_skip(
            hit_object,
            snapshot_time,
            layout,
            clip_left,
            clip_right,
        ) {
            continue;
        }

        // CPU 路径的普通 note 不裁剪判定线左侧：圆心到达判定线前始终绘制整颗
        // note，越线后一次性隐藏。长条/滚奏仍需裁剪，避免尾部穿过判定线。
        let clipped = hit_object.hit_object.hit_type & (SWELL_FLAG | DRUMROLL_FLAG) != 0;
        if clipped {
            scene.push_clip(rect(
                clip_left,
                top,
                clip_right - clip_left,
                layout.row_height,
            ));
        }
        draw_hit_object_scene(
            scene,
            hit_object,
            layout,
            snapshot_time,
            hidden_sprites[index].as_ref(),
        );
        if clipped {
            scene.pop_clip();
        }
    }
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

fn draw_hit_object_scene(
    scene: &mut FrameSceneBuilder,
    object: &PreparedTaikoHitObject,
    layout: &AnimationLayout,
    snapshot_time: i64,
    hidden_sprite: Option<&Arc<Img>>,
) {
    let base = &object.hit_object;
    if object.hidden && base.hit_type & (SWELL_FLAG | DRUMROLL_FLAG) != 0 {
        if hidden_alpha(
            base.start_time as f64,
            snapshot_time,
            object.start_multiplier,
            layout.time_range,
        ) <= 0.0
        {
            return;
        }
        // HD 长条使用同一物件层，确保 GPU 与 CPU 的整体淡出及重叠边缘一致。
        let mut layer = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            [0, 0, 0, 0],
        );
        crate::render::cpu::modes::taiko::animation_render::draw_hit_object(
            &mut layer,
            object,
            layout,
            0,
            snapshot_time,
            &mut crate::render::cpu::modes::taiko::notes::RenderCache::default(),
        );
        scene.sprite(
            Arc::new(layer),
            rect(0, 0, layout.image_width, layout.image_height),
            1.0,
        );
        return;
    }
    if base.hit_type & SWELL_FLAG != 0 {
        draw_span_scene(
            scene,
            object,
            layout,
            snapshot_time,
            true,
            SWELL_COLOR,
            true,
        );
    } else if base.hit_type & DRUMROLL_FLAG != 0 {
        draw_span_scene(
            scene,
            object,
            layout,
            snapshot_time,
            base.hitsound & HIT_SOUNDS_STRONG != 0,
            ROLL_COLOR,
            false,
        );
    } else {
        let center_x = object_x(
            base.start_time as f64,
            snapshot_time as f64,
            object.start_multiplier,
            layout,
        );
        // 与 CPU 路径一致：普通 note 的中心越过判定线后整颗消失，
        // 不显示被 scissor 从中间切开的半颗 note。
        if center_x < judgement_line_x(layout) {
            return;
        }
        let diameter = if base.hitsound & HIT_SOUNDS_STRONG != 0 {
            layout.big_note_diameter
        } else {
            layout.normal_note_diameter
        };
        let color = if base.hitsound & HIT_SOUNDS_RIM != 0 {
            RIM_NOTE_COLOR
        } else {
            CENTRE_NOTE_COLOR
        };
        if let Some(sprite) = hidden_sprite {
            let alpha = hidden_alpha(
                base.start_time as f64,
                snapshot_time,
                object.start_multiplier,
                layout.time_range,
            );
            if alpha > 0.0 {
                scene.sprite(
                    Arc::clone(sprite),
                    rect(
                        crate::domain::parser::round_half_even(
                            center_x as f64 - diameter as f64 / 2.0,
                        ),
                        crate::domain::parser::round_half_even(
                            gif_row_center_y(0, layout) as f64 - diameter as f64 / 2.0,
                        ),
                        diameter,
                        diameter,
                    ),
                    alpha as f32,
                );
            }
            return;
        }
        draw_note_disc_scene(
            scene,
            center_x,
            gif_row_center_y(0, layout),
            diameter,
            color,
            layout,
            false,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_span_scene(
    scene: &mut FrameSceneBuilder,
    object: &PreparedTaikoHitObject,
    layout: &AnimationLayout,
    snapshot_time: i64,
    large: bool,
    color: [u8; 3],
    swell_marker: bool,
) {
    let start_x = object_x(
        object.hit_object.start_time as f64,
        snapshot_time as f64,
        object.start_multiplier,
        layout,
    );
    let end_x = object_x(
        object.hit_object.end_time as f64,
        snapshot_time as f64,
        object.end_multiplier,
        layout,
    );
    let center_y = gif_row_center_y(0, layout);
    let head_diameter = if large {
        layout.big_note_diameter
    } else {
        layout.normal_note_diameter
    };
    let body_ratio = if large {
        SWELL_BODY_HEIGHT_RATIO
    } else {
        SPAN_BODY_HEIGHT_RATIO
    };
    let body_height = (head_diameter as f64 * body_ratio).round().max(1.0) as i64;
    if end_x > start_x {
        scene.rectangle(
            rect(
                start_x,
                (center_y as f64 - body_height as f64 / 2.0).round() as i64,
                end_x - start_x,
                body_height,
            ),
            [color[0], color[1], color[2], 255],
        );
        scene.circle(
            [end_x as f32, center_y as f32],
            body_height as f32 / 2.0,
            [color[0], color[1], color[2], 255],
        );
    }
    if object.hit_object.hit_type & DRUMROLL_FLAG != 0 {
        let base_diameter = (layout.normal_note_diameter as f64 * DRUM_ROLL_TICK_DIAMETER_RATIO)
            .round()
            .max(1.0) as i64;
        for tick in &object.drum_roll_ticks {
            let (mut alpha, scale) = drum_roll_tick_transform(tick.time, snapshot_time);
            if object.hidden {
                alpha *= hidden_alpha(tick.time, snapshot_time, tick.multiplier, layout.time_range);
            }
            if alpha <= 0.0 {
                continue;
            }
            let diameter = (base_diameter as f64 * scale).round().max(1.0) as f32;
            let center_x = object_x(tick.time, snapshot_time as f64, tick.multiplier, layout);
            let mut tick_color = DRUM_ROLL_TICK_COLOR;
            tick_color[3] = (tick_color[3] as f64 * alpha).round().clamp(0.0, 255.0) as u8;
            scene.circle(
                [center_x as f32, center_y as f32],
                diameter / 2.0,
                tick_color,
            );
        }
    }
    draw_note_disc_scene(
        scene,
        start_x,
        center_y,
        head_diameter,
        color,
        layout,
        swell_marker,
    );
}

fn draw_note_disc_scene(
    scene: &mut FrameSceneBuilder,
    center_x: i64,
    center_y: i64,
    diameter: i64,
    color: [u8; 3],
    layout: &AnimationLayout,
    swell_marker: bool,
) {
    let radius = diameter as f32 / 2.0;
    let edge = crate::render::geometry::scale_stroke_px(1.0, layout.render_scale).max(1) as f32;
    let ring = (diameter as f64 * NOTE_RING_THICKNESS_RATIO).max(1.0) as f32;
    let center = [center_x as f32, center_y as f32];
    scene.circle(center, radius, NOTE_EDGE_COLOR);
    scene.circle(center, (radius - edge).max(0.0), NOTE_RING_COLOR);
    let fill_radius = (radius - edge - ring).max(0.0);
    let fill = [color[0], color[1], color[2], 255];
    scene.circle(center, fill_radius, fill);
    if swell_marker {
        let inner = fill_radius * 0.55;
        scene.circle(center, inner, NOTE_RING_COLOR);
        scene.circle(center, (inner - ring).max(0.0), fill);
    }
}
