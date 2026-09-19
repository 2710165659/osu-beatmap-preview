//! Standard WGPU 实时帧源准备与原生场景生成。

use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::errors::Result;
use crate::domain::models::{Beatmap, BreakPeriod, StandardHitObject};
use crate::domain::mods::ModSettings;
use crate::domain::shared::time_selection::TimeAxis;
use crate::render::canvas::Img;
use crate::render::cpu::modes::standard::alpha::*;
use crate::render::cpu::modes::standard::constants::*;
use crate::render::cpu::modes::standard::context::{
    apply_standard_object_mods, build_render_context, build_visible_indexes_by_snapshot, py_round,
    stacked_position, standard_objects, to_frame_point, RenderCache, RenderContext,
};
use crate::render::cpu::modes::standard::follow_points::{
    build_sprite, follow_point_height, follow_point_position, follow_point_state,
};
use crate::render::cpu::modes::standard::slider::{
    alpha_to_byte, build_reverse_arrow, darken, get_slider_render_data, rotate_arrow_point,
    slider_ball_arrow_angle, slider_ball_arrow_geometry, slider_snaked_range, SliderRenderData,
};
use crate::render::geometry::{GameMode, OutputFormat};
use crate::render::scene::{FrameScene, FrameSceneBuilder, SceneRect};
use crate::render::text::{render_text_sprite, scaled_bitmap_font_height};
use crate::render::wgpu::RealtimeFrameSource;

struct PreparedSlider {
    data: Arc<SliderRenderData>,
    full_path: Arc<[[f32; 2]]>,
    // 与 CPU 路径一致：折返点只画 `»` 箭头，不带渐变边缘。
    reverse_arrows: Vec<Arc<Img>>,
}

struct PreparedBreak {
    period: BreakPeriod,
    counters: Vec<Arc<Img>>,
    info: Arc<Img>,
}

/// 预旋转好的跟随点图标：按连接方向的角度取整后共享，逐帧只做缩放与合成。
struct PreparedFollowPoints {
    /// 与 `context.follow_points` 一一对应的精灵。
    sprites: Vec<Arc<Img>>,
    /// 精灵的基准像素高度（淡入开始时的最大高度 1.5 倍物件缩放）。
    base_height: f64,
}

/// stacking、Mod、路径采样与文字资源都在会话加载时完成；逐帧只计算生命周期和
/// 可见路径区间，滑条主体不再执行 CPU 像素扫描。
pub fn prepare_realtime(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    time_axis: TimeAxis,
) -> Result<RealtimeFrameSource> {
    let objects = apply_standard_object_mods(standard_objects(beatmap)?, mods);
    let context = build_render_context(beatmap, objects, mods, time_axis, OutputFormat::Mp4);
    let mut cache = RenderCache::default();
    let sliders = context
        .hit_objects
        .iter()
        .enumerate()
        .map(|(index, object)| {
            (object.hit_type & 2 != 0).then(|| {
                let data = get_slider_render_data(&mut cache, &context, index);
                let reverse_arrows = data
                    .reverse_angles
                    .iter()
                    .map(|&angle| {
                        let angle_deg = -angle.to_degrees();
                        Arc::new(
                            build_reverse_arrow(
                                context.frame_circle_diameter,
                                context.combo_info[index].color,
                            )
                            .rotate_expand(angle_deg),
                        )
                    })
                    .collect();
                PreparedSlider {
                    full_path: path_vertices(&data.frame_path.points),
                    data,
                    reverse_arrows,
                }
            })
        })
        .collect::<Vec<_>>();
    let digit_masks = context
        .skin
        .digit_crops
        .iter()
        .map(|image| Arc::new((*image).clone()))
        .collect::<Vec<_>>();
    let breaks = prepare_breaks(&beatmap.break_periods, &context);
    let follow_points = prepare_follow_points(&context);
    Ok(RealtimeFrameSource::new(
        GameMode::Standard,
        move |absolute_time_ms| {
            let indexes = build_visible_indexes_by_snapshot(
                &context.hit_objects,
                &[absolute_time_ms],
                context.settings.preempt_ms,
            );
            Ok(render_scene(
                &context,
                &sliders,
                &digit_masks,
                &breaks,
                &follow_points,
                absolute_time_ms,
                &indexes[0],
            ))
        },
    ))
}

/// 预先按连接方向旋转跟随点图标。
///
/// 图标在淡入期间从 1.5 倍缩到 1 倍，这里按最大高度建图，逐帧用四边形缩放，
/// 因此同一角度只需要一张精灵。角度取整到 1°，与滑条球箭头、折返箭头一致。
fn prepare_follow_points(context: &RenderContext) -> PreparedFollowPoints {
    let base_height =
        follow_point_height(&context.settings, context.frame_layout.scale, 0.0).max(1.0);
    let mut sprites_by_angle: HashMap<i64, Arc<Img>> = HashMap::new();
    let sprites = context
        .follow_points
        .iter()
        .map(|point| {
            let angle = py_round(point.rotation);
            Arc::clone(
                sprites_by_angle
                    .entry(angle)
                    .or_insert_with(|| Arc::new(build_sprite(base_height, angle as f64))),
            )
        })
        .collect();
    PreparedFollowPoints {
        sprites,
        base_height,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_scene(
    context: &RenderContext,
    sliders: &[Option<PreparedSlider>],
    digits: &[Arc<Img>],
    breaks: &[PreparedBreak],
    follow_points: &PreparedFollowPoints,
    time: i64,
    visible: &[usize],
) -> FrameScene {
    let mut scene = FrameSceneBuilder::new(
        context.frame_layout.frame_width as u32,
        context.frame_layout.frame_height as u32,
        time,
    );
    // 保留透明底层以维持空时间点的场景命令契约；实际背景由画布合成阶段提供。
    scene.rectangle(
        rect(
            0.0,
            0.0,
            context.frame_layout.frame_width as f64,
            context.frame_layout.frame_height as f64,
        ),
        [0, 0, 0, 0],
    );
    // 跟随点在游戏里位于物件层之下，先于物件发出命令。
    draw_follow_points(&mut scene, context, follow_points, time);
    for &index in visible {
        let object = &context.hit_objects[index];
        if object.hit_type & 8 != 0 {
            draw_spinner(&mut scene, context, object, time);
        } else if object.hit_type & 2 != 0 {
            draw_slider(
                &mut scene,
                context,
                sliders[index].as_ref().expect("滑条预计算数据必须存在"),
                digits,
                index,
                time,
            );
        } else {
            draw_hit_circle(&mut scene, context, digits, index, time);
        }
    }
    for &index in visible {
        if context.hit_objects[index].hit_type & 8 == 0 {
            draw_approach_circle(&mut scene, context, index, time);
        }
    }
    if let Some(current) = breaks
        .iter()
        .find(|value| break_alpha(&value.period, time) > 0.0)
    {
        draw_break(&mut scene, context, current, time);
    }
    scene.finish()
}

/// 绘制当前时刻存活区间内的跟随点，精灵按统一的基准高度等比缩放。
fn draw_follow_points(
    scene: &mut FrameSceneBuilder,
    context: &RenderContext,
    prepared: &PreparedFollowPoints,
    time: i64,
) {
    for (point, sprite) in context.follow_points.iter().zip(&prepared.sprites) {
        let (alpha, progress) = follow_point_state(point, time);
        if alpha <= 0.0 {
            continue;
        }
        let ratio = follow_point_height(&context.settings, context.frame_layout.scale, progress)
            / prepared.base_height;
        let side = sprite.w as f64 * ratio;
        let world = follow_point_position(point, progress);
        let center = to_frame_point(world.0, world.1, &context.frame_layout);
        scene.sprite(
            Arc::clone(sprite),
            rect(center.0 - side / 2.0, center.1 - side / 2.0, side, side),
            alpha as f32,
        );
    }
}

fn draw_hit_circle(
    scene: &mut FrameSceneBuilder,
    context: &RenderContext,
    digits: &[Arc<Img>],
    index: usize,
    time: i64,
) {
    if context.settings.traceable {
        return;
    }
    let object = &context.hit_objects[index];
    let alpha = object_alpha(
        object.start_time,
        object.start_time,
        time,
        &context.settings,
    );
    let world = stacked_position(object, &context.settings);
    draw_circle_piece(
        scene,
        context,
        digits,
        to_frame_point(world.0, world.1, &context.frame_layout),
        context.combo_info[index].color,
        alpha,
        context.combo_info[index].number,
    );
}

fn draw_slider(
    scene: &mut FrameSceneBuilder,
    context: &RenderContext,
    slider: &PreparedSlider,
    digits: &[Arc<Img>],
    index: usize,
    time: i64,
) {
    let object = &context.hit_objects[index];
    let color = context.combo_info[index].color;
    let body_alpha = slider_body_alpha(object, time, &context.settings);
    let overlay_alpha =
        normal_object_alpha(object.start_time, object.end_time, time, &context.settings);
    let (snake_start, snake_end) = slider_snaked_range(object, time, &context.settings);
    let path = if snake_start <= 0.001 && snake_end >= 0.999 {
        Arc::clone(&slider.full_path)
    } else {
        path_vertices(&crate::domain::shared::slider_path::slice_path(
            &slider.data.frame_path,
            snake_start,
            snake_end,
        ))
    };
    if path.len() >= 2 && body_alpha > 0.0 {
        let alpha = alpha_to_byte(body_alpha);
        let body = if context.settings.traceable {
            image_background_color()
        } else {
            let value = darken(color, 4.0);
            [
                value[0],
                value[1],
                value[2],
                (alpha as f64 * ARGON_SLIDER_BODY_ALPHA).round() as u8,
            ]
        };
        scene.slider_mesh(
            path,
            context.slider_body_width as f32,
            [color[0], color[1], color[2], alpha],
            body,
        );
    }
    draw_ticks(scene, context, &slider.data, color, overlay_alpha, time);
    draw_reverse_arrows(
        scene,
        slider,
        object,
        body_alpha,
        snake_start,
        snake_end,
        time,
    );
    draw_slider_ball(
        scene,
        context,
        &slider.data,
        object,
        color,
        overlay_alpha,
        time,
    );
    let head_alpha = slider_head_alpha(object, time, &context.settings, snake_start, snake_end);
    if head_alpha > 0.0 && !context.settings.traceable {
        draw_circle_piece(
            scene,
            context,
            digits,
            slider.data.head_center,
            color,
            head_alpha,
            context.combo_info[index].number,
        );
    }
}

fn draw_ticks(
    scene: &mut FrameSceneBuilder,
    context: &RenderContext,
    slider: &SliderRenderData,
    color: [u8; 3],
    parent_alpha: f64,
    time: i64,
) {
    let diameter = (context.frame_circle_diameter as f64 * ARGON_SLIDER_TICK_SIZE_RATIO)
        .round()
        .max(1.0) as f32;
    for tick in &slider.ticks {
        let alpha =
            slider_tick_alpha(tick.time, tick.time_preempt, time, &context.settings) * parent_alpha;
        if alpha > 0.0 {
            scene.ring(
                [tick.center.0 as f32, tick.center.1 as f32],
                diameter / 2.0,
                (diameter * ARGON_SLIDER_TICK_BORDER_RATIO as f32).max(1.0),
                [color[0], color[1], color[2], alpha_to_byte(alpha)],
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_reverse_arrows(
    scene: &mut FrameSceneBuilder,
    slider: &PreparedSlider,
    object: &StandardHitObject,
    alpha: f64,
    snake_start: f64,
    snake_end: f64,
    time: i64,
) {
    if object.slider_repeats <= 1 || alpha <= 0.0 {
        return;
    }
    let spans = object.slider_repeats.max(1) as f64;
    let duration = (object.end_time - object.start_time) as f64;
    let fade = 300.0_f64.min(duration / spans) / duration.max(1.0);
    for (index, &center) in slider.data.reverse_centers.iter().enumerate() {
        let repeat = (index + 1) as i64;
        let position = if repeat % 2 == 1 { 1.0 } else { 0.0 };
        if !(snake_start - 0.001 <= position && position <= snake_end + 0.001) {
            continue;
        }
        let repeat_alpha = if time < object.start_time {
            if repeat == 1 {
                1.0
            } else {
                0.0
            }
        } else {
            let traversal = (time - object.start_time) as f64
                / (object.end_time - object.start_time).max(1) as f64
                * spans;
            if traversal < (repeat - 1) as f64 || traversal >= repeat as f64 {
                0.0
            } else if traversal > repeat as f64 - fade {
                ((repeat as f64 - traversal) / fade).max(0.0)
            } else {
                1.0
            }
        };
        let opacity = alpha * repeat_alpha;
        if opacity <= 0.0 {
            continue;
        }
        let arrow = &slider.reverse_arrows[index];
        scene.sprite(
            Arc::clone(arrow),
            rect(
                py_round(center.0 - arrow.w as f64 / 2.0) as f64,
                py_round(center.1 - arrow.h as f64 / 2.0) as f64,
                arrow.w as f64,
                arrow.h as f64,
            ),
            opacity as f32,
        );
    }
}

fn draw_slider_ball(
    scene: &mut FrameSceneBuilder,
    context: &RenderContext,
    slider: &SliderRenderData,
    object: &StandardHitObject,
    color: [u8; 3],
    alpha: f64,
    time: i64,
) {
    if !(object.start_time..=object.end_time).contains(&time) || alpha <= 0.0 {
        return;
    }
    let completion =
        (time - object.start_time) as f64 / (object.end_time - object.start_time).max(1) as f64;
    let progress = slider_path_progress(object.slider_repeats.max(1) as i64, completion);
    let center = crate::domain::shared::slider_path::path_position_at(&slider.frame_path, progress);
    let point = [center.0 as f32, center.1 as f32];
    let follow_radius = context.slider_follow_size as f32 / 2.0;
    let follow_border = (4.0 * context.frame_circle_diameter as f64 / 128.0).max(1.0) as f32;
    scene.circle(
        point,
        (follow_radius - follow_border).max(0.0),
        [
            color[0],
            color[1],
            color[2],
            alpha_to_byte(alpha * 0.7 * 77.0 / 255.0),
        ],
    );
    scene.ring(
        point,
        follow_radius,
        follow_border,
        [color[0], color[1], color[2], alpha_to_byte(alpha * 0.7)],
    );
    let radius = context.slider_ball_size as f32 / 2.0;
    scene.circle(
        point,
        radius,
        [color[0], color[1], color[2], alpha_to_byte(alpha)],
    );
    scene.ring(
        point,
        radius,
        (2.5 * context.frame_circle_diameter as f64 * ARGON_BORDER_RATIO).max(1.0) as f32,
        [255, 255, 255, alpha_to_byte(alpha)],
    );
    // 方向箭头：与 CPU 路径共用几何参数，用两段线段加三个圆头拼出 `>` 形
    // （`Line` 命令没有旋转，圆头也要显式补上），朝向由共享的旋转函数解析计算。
    if let Some(angle) = slider_ball_arrow_angle(
        &slider.frame_path,
        object.slider_repeats.max(1) as i64,
        completion,
    ) {
        let arrow = slider_ball_arrow_geometry(context.frame_circle_diameter as f64);
        let white = [255, 255, 255, alpha_to_byte(alpha)];
        let thickness = arrow.thickness as f32;
        let to_scene = |local: (f64, f64)| {
            let point = rotate_arrow_point(center, local, angle);
            [point.0 as f32, point.1 as f32]
        };
        let tip = to_scene(arrow.tip);
        let top = to_scene(arrow.top);
        let bottom = to_scene(arrow.bottom);
        scene.line(top, tip, thickness, white);
        scene.line(tip, bottom, thickness, white);
        for vertex in [top, tip, bottom] {
            scene.circle(vertex, thickness / 2.0, white);
        }
    }
}

fn draw_spinner(
    scene: &mut FrameSceneBuilder,
    context: &RenderContext,
    object: &StandardHitObject,
    time: i64,
) {
    let alpha = spinner_alpha(object, time, &context.settings);
    if alpha <= 0.0 {
        return;
    }
    let center = to_frame_point(
        PLAYFIELD_WIDTH / 2.0,
        PLAYFIELD_HEIGHT / 2.0,
        &context.frame_layout,
    );
    let point = [center.0 as f32, center.1 as f32];
    let scale = context.spinner_size as f64 / 256.0;
    let base = 80.0 * scale;
    let progress = ((time - object.start_time) as f64
        / (object.end_time - object.start_time).max(1) as f64)
        .clamp(0.0, 1.0);
    scene.circle(
        point,
        (base * (0.8 + 0.6 * progress)) as f32,
        [
            ARGON_SPINNER_PINK[0],
            ARGON_SPINNER_PINK[1],
            ARGON_SPINNER_PINK[2],
            (30.0 * alpha) as u8,
        ],
    );
    scene.ring(
        point,
        (base * 0.8) as f32,
        (10.0 * scale).max(1.0) as f32,
        [255, 255, 255, alpha_to_byte(alpha)],
    );
    scene.ring(
        point,
        base as f32,
        (3.0 * scale).max(1.0) as f32,
        [255, 255, 255, alpha_to_byte(alpha)],
    );
}

fn draw_approach_circle(
    scene: &mut FrameSceneBuilder,
    context: &RenderContext,
    index: usize,
    time: i64,
) {
    let object = &context.hit_objects[index];
    if context.settings.hidden || time >= object.start_time {
        return;
    }
    let elapsed = (time - (object.start_time - context.settings.preempt_ms)) as f64;
    let progress = (elapsed / context.settings.preempt_ms as f64).clamp(0.0, 1.0);
    let alpha = 0.9 * (elapsed / (context.settings.fade_in_ms * 2.0).max(1.0)).min(1.0);
    if alpha <= 0.0 {
        return;
    }
    let world = stacked_position(object, &context.settings);
    let center = to_frame_point(world.0, world.1, &context.frame_layout);
    let diameter = context.frame_circle_diameter as f64 * (4.0 - 3.0 * progress);
    let color = context.combo_info[index].color;
    scene.ring(
        [center.0 as f32, center.1 as f32],
        (diameter / 2.0) as f32,
        (diameter * 0.03).max(1.0) as f32,
        [color[0], color[1], color[2], alpha_to_byte(alpha)],
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_circle_piece(
    scene: &mut FrameSceneBuilder,
    context: &RenderContext,
    digit_masks: &[Arc<Img>],
    center: (f64, f64),
    color: [u8; 3],
    alpha: f64,
    number: u32,
) {
    if alpha <= 0.0 {
        return;
    }
    let diameter = context.frame_circle_diameter as f64;
    let border = diameter * ARGON_BORDER_RATIO;
    let alpha = alpha_to_byte(alpha);
    let dark = darken(color, 4.0);
    let point = [center.0 as f32, center.1 as f32];
    scene.circle(
        point,
        ((diameter - 1.0) / 2.0) as f32,
        [dark[0], dark[1], dark[2], alpha],
    );
    scene.ring(
        point,
        (diameter / 2.0) as f32,
        border as f32,
        [255, 255, 255, alpha],
    );
    let outer = ((diameter - 4.0 * border).max(0.0) / 2.0) as f32;
    scene.circle(point, outer, [color[0], color[1], color[2], alpha]);
    let middle = (outer as f64 - 2.5 * border).max(0.0) as f32;
    let middle_color = darken(color, 0.5);
    scene.circle(
        point,
        middle,
        [middle_color[0], middle_color[1], middle_color[2], alpha],
    );
    scene.circle(
        point,
        (middle as f64 - 2.5 * border).max(0.0) as f32,
        [dark[0], dark[1], dark[2], alpha],
    );
    draw_number(scene, context, digit_masks, number, center, alpha);
}

fn draw_number(
    scene: &mut FrameSceneBuilder,
    context: &RenderContext,
    masks: &[Arc<Img>],
    number: u32,
    center: (f64, f64),
    alpha: u8,
) {
    let height = (context.frame_circle_diameter as f64 * 0.30)
        .round()
        .max(1.0) as i64;
    let digits = number
        .to_string()
        .bytes()
        .map(|value| (value - b'0') as usize)
        .collect::<Vec<_>>();
    let widths = digits
        .iter()
        .map(|&digit| {
            (masks[digit].w as f64 * height as f64 / masks[digit].h.max(1) as f64)
                .round()
                .max(1.0) as i64
        })
        .collect::<Vec<_>>();
    let overlap = (context.skin.hitcircle_overlap as f64 * height as f64 / 100.0).round() as i64;
    let total = widths.iter().sum::<i64>() - overlap * (digits.len() as i64 - 1);
    let mut x = (center.0 - total as f64 / 2.0).round() as i64;
    let y = (center.1 - height as f64 / 2.0).round() as i64;
    for (&digit, &width) in digits.iter().zip(&widths) {
        scene.glyph(
            Arc::clone(&masks[digit]),
            rect(x as f64, y as f64, width as f64, height as f64),
            [255, 255, 255, alpha],
        );
        x += width - overlap;
    }
}

fn prepare_breaks(periods: &[BreakPeriod], context: &RenderContext) -> Vec<PreparedBreak> {
    let scale = crate::render::geometry::output_scale(GameMode::Standard, context.output_format);
    let counter_size = scaled_bitmap_font_height(BREAK_OVERLAY_COUNTER_FONT_SIZE, scale);
    let info_size = scaled_bitmap_font_height(BREAK_OVERLAY_INFO_FONT_SIZE, scale);
    periods
        .iter()
        .map(|period| {
            let maximum = ((period.end_time - period.start_time + 999).div_euclid(1000)).max(0);
            let counters = (0..=maximum)
                .map(|value| {
                    Arc::new(render_text_sprite(
                        &value.to_string(),
                        counter_size,
                        [255, 255, 255, 255],
                    ))
                })
                .collect();
            let label = format!(
                "Break {} - {}",
                crate::render::text::format_mmssmmm(
                    context.time_axis.to_display(period.start_time)
                ),
                crate::render::text::format_mmssmmm(context.time_axis.to_display(period.end_time)),
            );
            PreparedBreak {
                period: *period,
                counters,
                info: Arc::new(render_text_sprite(&label, info_size, [255, 255, 255, 255])),
            }
        })
        .collect()
}

fn draw_break(
    scene: &mut FrameSceneBuilder,
    context: &RenderContext,
    prepared: &PreparedBreak,
    time: i64,
) {
    let period = &prepared.period;
    let alpha = break_alpha(period, time);
    let width = context.frame_layout.frame_width as f64;
    let height = context.frame_layout.frame_height as f64;
    let center_y = height / 2.0;
    let scale = crate::render::geometry::output_scale(GameMode::Standard, context.output_format);
    for (offset, direction) in [(-0.22, 1.0), (0.22, -1.0)] {
        let center_x = width * (0.5 + offset);
        for (size, thickness, opacity) in [(32.0, 9.0, 35.0), (20.0, 4.0, 80.0)] {
            let half = size * scale / 2.0;
            let tip = [(center_x + direction * half) as f32, center_y as f32];
            let rgba = [238, 238, 238, (opacity * alpha).round() as u8];
            scene.line(
                [
                    (center_x - direction * half) as f32,
                    (center_y - half) as f32,
                ],
                tip,
                (thickness * scale) as f32,
                rgba,
            );
            scene.line(
                tip,
                [
                    (center_x - direction * half) as f32,
                    (center_y + half) as f32,
                ],
                (thickness * scale) as f32,
                rgba,
            );
        }
    }
    let track_width = width * BREAK_OVERLAY_BAR_WIDTH_RATIO;
    let track_height = BREAK_OVERLAY_BAR_HEIGHT * scale;
    scene.line(
        [(width / 2.0 - track_width / 2.0) as f32, center_y as f32],
        [(width / 2.0 + track_width / 2.0) as f32, center_y as f32],
        track_height as f32,
        [48, 48, 48, (150.0 * alpha).round() as u8],
    );
    let fill = track_width * break_remaining_ratio(period, time);
    scene.line(
        [(width / 2.0 - fill / 2.0) as f32, center_y as f32],
        [(width / 2.0 + fill / 2.0) as f32, center_y as f32],
        track_height as f32,
        [238, 238, 238, (230.0 * alpha).round() as u8],
    );
    let seconds = ((period.end_time - time + 999).div_euclid(1000)).max(0) as usize;
    if let Some(mask) = prepared.counters.get(seconds) {
        scene.glyph(
            Arc::clone(mask),
            rect(
                (width - mask.w as f64) / 2.0,
                center_y - crate::render::geometry::scale_px(15.0, scale) as f64 - mask.h as f64,
                mask.w as f64,
                mask.h as f64,
            ),
            [238, 238, 238, alpha_to_byte(alpha)],
        );
    }
    scene.glyph(
        Arc::clone(&prepared.info),
        rect(
            (width - prepared.info.w as f64) / 2.0,
            center_y
                + crate::render::geometry::scale_px(BREAK_OVERLAY_INFO_TOP_GAP as f64, scale)
                    as f64,
            prepared.info.w as f64,
            prepared.info.h as f64,
        ),
        [185, 185, 185, alpha_to_byte(alpha)],
    );
}

fn break_alpha(period: &BreakPeriod, time: i64) -> f64 {
    if period.end_time - period.start_time < BREAK_MIN_DURATION_MS
        || !(period.start_time..=period.end_time).contains(&time)
    {
        return 0.0;
    }
    if time < period.start_time + BREAK_FADE_DURATION_MS {
        return (time - period.start_time) as f64 / BREAK_FADE_DURATION_MS as f64;
    }
    if time > period.end_time - BREAK_FADE_DURATION_MS {
        return (period.end_time - time) as f64 / BREAK_FADE_DURATION_MS as f64;
    }
    1.0
}

fn break_remaining_ratio(period: &BreakPeriod, time: i64) -> f64 {
    let duration = period.end_time - BREAK_FADE_DURATION_MS - period.start_time;
    if duration <= 0 {
        return 0.0;
    }
    ((period.end_time - BREAK_FADE_DURATION_MS - time) as f64 / duration as f64).clamp(0.0, 1.0)
}

fn path_vertices(points: &[(f64, f64)]) -> Arc<[[f32; 2]]> {
    points
        .iter()
        .map(|&(x, y)| [x as f32, y as f32])
        .collect::<Vec<_>>()
        .into()
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> SceneRect {
    SceneRect {
        x: x as f32,
        y: y as f32,
        width: width.max(0.0) as f32,
        height: height.max(0.0) as f32,
    }
}

fn image_background_color() -> [u8; 4] {
    crate::config::current()
        .render
        .standard
        .mp4
        .style
        .IMAGE_BACKGROUND_COLOR
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HitObjects, KvSection, TimingPoint};
    use crate::render::scene::DrawCommand;

    /// 构造一条从 (100, 192) 到 (400, 192) 的水平滑条，起点 1000ms。
    /// SliderMultiplier 1.4 与 500ms 拍长下每段滑行 1071ms。
    fn slider_beatmap(repeats: i32) -> Beatmap {
        let mut general = KvSection::default();
        general.insert("Mode", "0".to_string());
        let mut difficulty = KvSection::default();
        difficulty.insert("CircleSize", "4".to_string());
        difficulty.insert("ApproachRate", "5".to_string());
        difficulty.insert("SliderMultiplier", "1.4".to_string());
        difficulty.insert("SliderTickRate", "1".to_string());
        Beatmap {
            metadata: KvSection::default(),
            difficulty,
            general,
            timing_points: vec![TimingPoint {
                time: 0.0,
                beat_length: 500.0,
                meter: 4,
                uninherited: true,
                kiai_mode: false,
                omit_first_bar_line: false,
                sample_set: 0,
                sample_index: 0,
                sample_volume: 100,
            }],
            hit_objects: HitObjects::Standard(vec![StandardHitObject {
                x: 100,
                y: 192,
                start_time: 1000,
                end_time: 1000 + 1071 * repeats as i64,
                hit_type: 6,
                hitsound: 0,
                new_combo: true,
                combo_offset: 0,
                slider_type: Some("L".to_string()),
                slider_points: vec![(400, 192)],
                slider_repeats: repeats,
                slider_pixel_length: 300.0,
                slider_edge_hitsounds: vec![0; repeats as usize + 1],
                stack_height: 0,
                samples: Vec::new(),
                slider_edge_samples: Vec::new(),
            }]),
            break_periods: Vec::new(),
            background_filename: None,
            combo_colors: Vec::new(),
            beat_divisor: 0,
        }
    }

    /// 场景里白色方向箭头由两条线段和三个圆头组成，这里取出线段的起终点。
    fn arrow_segments(scene: &FrameScene) -> Vec<([f32; 2], [f32; 2])> {
        scene
            .commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::Line {
                    from,
                    to,
                    thickness,
                    color,
                } if *color == [255, 255, 255, 255] => {
                    assert!(*thickness > 0.0);
                    Some((*from, *to))
                }
                _ => None,
            })
            .collect()
    }

    /// 两个水平相距 300 个 playfield 单位的圆圈。
    fn two_circle_beatmap() -> Beatmap {
        let mut general = KvSection::default();
        general.insert("Mode", "0".to_string());
        let mut difficulty = KvSection::default();
        difficulty.insert("CircleSize", "4".to_string());
        difficulty.insert("ApproachRate", "5".to_string());
        let circle = |x: i32, start_time: i64| StandardHitObject {
            x,
            y: 192,
            start_time,
            end_time: start_time,
            hit_type: 1,
            ..Default::default()
        };
        Beatmap {
            metadata: KvSection::default(),
            difficulty,
            general,
            timing_points: Vec::new(),
            hit_objects: HitObjects::Standard(vec![circle(100, 1000), circle(400, 2000)]),
            break_periods: Vec::new(),
            background_filename: None,
            combo_colors: Vec::new(),
            beat_divisor: 0,
        }
    }

    /// 跟随点精灵出现在连接上的正确位置，并在生命周期结束后消失。
    #[test]
    fn realtime_follow_points_appear_between_objects() {
        let source = prepare_realtime(&two_circle_beatmap(), None, TimeAxis::new(0))
            .expect("实时场景必须可以准备");

        let sprites = |scene: &FrameScene| {
            scene
                .commands
                .iter()
                .filter_map(|command| match command {
                    DrawCommand::Sprite { destination, .. } => Some(*destination),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        // 1200ms 时全部 7 个跟随点都已淡入：第一个（连接上 48/300 处）停在
        // 世界坐标 (148, 192)，位于左侧圆圈判定圈之外。
        let scene = source.render(1200).expect("任意时间都必须能出帧");
        let visible = sprites(&scene);
        // 距离 300 共 7 个跟随点（48 起每 32 一个），各自的存活区间都覆盖 1200ms。
        assert_eq!(visible.len(), 7);
        let layout =
            crate::render::cpu::modes::standard::context::build_frame_layout(OutputFormat::Mp4);
        let center = to_frame_point(148.0, 192.0, &layout);
        assert!(
            visible.iter().any(|quad| {
                (quad.x + quad.width / 2.0 - center.0 as f32).abs() < 0.01
                    && (quad.y + quad.height / 2.0 - center.1 as f32).abs() < 0.01
            }),
            "第一个跟随点应停在连接上的 48/300 处：{visible:?}"
        );

        // 淡入之前（fade_in = 360ms）与最后一个跟随点淡出之后都没有跟随点。
        assert!(sprites(&source.render(300).expect("出帧失败")).is_empty());
        assert!(
            sprites(&source.render(2300).expect("出帧失败")).is_empty(),
            "最后一个跟随点在 2200ms 之后应已消失"
        );
    }

    /// 实时场景的滑条球箭头跟随折返方向。
    #[test]
    fn realtime_slider_ball_arrow_follows_repeat_direction() {
        let source = prepare_realtime(&slider_beatmap(2), None, TimeAxis::new(0))
            .expect("实时场景必须可以准备");
        // 第一段滑行球向右，折返后向左；两段都必须出现在箭头线段上。
        for (time, tip_on_right) in [(1400_i64, true), (2600, false)] {
            let scene = source.render(time).expect("任意时间都必须能出帧");
            let segments = arrow_segments(&scene);
            assert_eq!(segments.len(), 2, "时间 {time} 应只有方向箭头的两条线段");
            let [first, second] = [segments[0], segments[1]];
            // 两条线段共享的端点就是箭头的尖端，另外两端在尖端后方。
            let tip = if first.0 == second.0 || first.0 == second.1 {
                first.0
            } else {
                first.1
            };
            let tails = [first.0, first.1, second.0, second.1]
                .into_iter()
                .filter(|point| *point != tip)
                .collect::<Vec<_>>();
            let tail_x = tails.iter().map(|point| point[0]).sum::<f32>() / tails.len() as f32;
            assert_eq!(
                tip[0] > tail_x,
                tip_on_right,
                "时间 {time} 的箭头方向应与滑行方向一致：tip={tip:?} tails={tails:?}"
            );
            // 水平滑条上箭头的两端应关于尖端上下对称。
            assert_eq!(tails.len(), 2);
            let tail_y = (tails[0][1] + tails[1][1]) / 2.0;
            assert!((tip[1] - tail_y).abs() < 0.5, "箭头应上下对称：tip={tip:?}");
        }
    }
}
