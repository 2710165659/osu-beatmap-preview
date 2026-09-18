//! osu!standard 滑条渲染：路径数据、主体、滑条球和反向箭头。

use crate::domain::models::StandardHitObject;
use crate::domain::shared::slider_path::{
    build_path, build_standard_slider_path, path_position_at, SliderPath,
};
use crate::render::canvas::Img;
use std::collections::HashMap;
use std::sync::Arc;

use super::constants::*;
use super::context::{
    color_id, py_round, stack_offset, to_frame_point, CachedLayer, RenderCache, RenderContext,
};

// ——— 数据 ———

pub struct SliderRenderData {
    pub frame_path: SliderPath,
    pub head_center: (f64, f64),
    pub reverse_centers: Vec<(f64, f64)>,
    pub reverse_angles: Vec<f64>,
    pub ticks: Vec<SliderTickRenderData>,
}

pub struct SliderTickRenderData {
    pub center: (f64, f64),
    pub time: f64,
    pub time_preempt: f64,
}

// ——— 滑条主体 ———

pub fn draw_slider_body(
    frame: &mut Img,
    points: &[(f64, f64)],
    width: i64,
    color: [u8; 3],
    alpha: f64,
    traceable: bool,
) {
    if points.len() < 2 {
        return;
    }
    let layer = render_slider_body_layer(
        points,
        width,
        color,
        alpha_to_byte(alpha),
        traceable,
        frame.w as i64,
        frame.h as i64,
    );
    frame.alpha_composite(&layer.image, layer.offset.0, layer.offset.1);
}

#[allow(clippy::too_many_arguments)]
pub fn draw_cached_slider_body(
    frame: &mut Img,
    context: &RenderContext,
    cache: &mut RenderCache,
    index: usize,
    slider_data: &SliderRenderData,
    color: [u8; 3],
    alpha: f64,
    traceable: bool,
) {
    let alpha_key = alpha_to_byte(alpha);
    let alpha_cache_key = (index, alpha_key);

    // 先查本线程的 alpha 变体缓存：命中时完全不触碰共享槽，省掉同步开销。
    if let Some(layer) = cache.slider_body_alpha_layers.get(&alpha_cache_key) {
        frame.alpha_composite(&layer.image, layer.offset.0, layer.offset.1);
        return;
    }

    // 基础图层是「序号 + 路径几何 + 宽度 + 颜色 + traceable」的纯函数，且这些输入在
    // 同一个 `RenderContext` 内都不变，因此放进跨线程共享槽：多个 rayon 线程同时
    // 需要同一条滑条时只有一个真正构建，其余复用同一份结果。
    let shared = context.body_layers.get_or_init(index, || {
        render_slider_body_layer(
            &slider_data.frame_path.points,
            context.slider_body_width,
            color,
            255,
            traceable,
            context.frame_layout.frame_width,
            context.frame_layout.frame_height,
        )
    });

    // 共享槽只保存未调 alpha 的基础图层；按 alpha 派生的副本留在本线程缓存。
    // 不同线程会各自派生一次，但 `scale_alpha` 比重新构建图层便宜一个数量级。
    let scaled = CachedLayer {
        image: shared.image.scale_alpha(alpha_key as f64 / 255.0),
        offset: shared.offset,
    };
    frame.alpha_composite(&scaled.image, scaled.offset.0, scaled.offset.1);
    cache
        .slider_body_alpha_layers
        .insert(alpha_cache_key, scaled);
}

pub fn render_slider_body_layer(
    points: &[(f64, f64)],
    width: i64,
    color: [u8; 3],
    alpha_byte: u8,
    traceable: bool,
    frame_width: i64,
    frame_height: i64,
) -> CachedLayer {
    let scale = crate::config::current()
        .render
        .standard
        .png
        .style
        .SLIDER_BODY_SUPERSAMPLE;
    let pad = (width + 4) as f64;
    let min_x = points.iter().map(|p| p.0).fold(f64::MAX, f64::min);
    let min_y = points.iter().map(|p| p.1).fold(f64::MAX, f64::min);
    let max_x = points.iter().map(|p| p.0).fold(f64::MIN, f64::max);
    let max_y = points.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    let left = ((min_x - pad).floor() as i64).max(0);
    let top = ((min_y - pad).floor() as i64).max(0);
    let right = ((max_x + pad).ceil() as i64).min(frame_width);
    let bottom = ((max_y + pad).ceil() as i64).min(frame_height);

    let layer_w = ((right - left) * scale).max(1) as u32;
    let layer_h = ((bottom - top) * scale).max(1) as u32;
    let mut layer = Img::new(layer_w, layer_h, [0, 0, 0, 0]);
    let scaled_points: Vec<(f64, f64)> = points
        .iter()
        .map(|&(x, y)| {
            (
                (x - left as f64) * scale as f64,
                (y - top as f64) * scale as f64,
            )
        })
        .collect();

    let inner_width = py_round(
        width as f64
            * (1.0 - crate::render::cpu::modes::standard::constants::ARGON_SLIDER_BORDER_PORTION),
    )
    .max(1);
    let body_alpha = py_round(
        alpha_byte as f64 * crate::render::cpu::modes::standard::constants::ARGON_SLIDER_BODY_ALPHA,
    )
    .clamp(0, 255) as u8;
    // 边框颜色：使用 combo 颜色（与 C# Argon 的 AccentColour 一致）
    // 轨道内部颜色：使用 Darken(4)（与 C# Argon 的 AccentColour.Darken(4) 一致）
    // 注意：SliderBorder / SliderTrackOverride 不在当前 skin 配置中，以匹配 Argon 风格
    let border_color = color;
    let inner_color = darken(color, 4.0);

    if traceable {
        // TC：AccentColour=transparent，BorderColour=accent。
        // 先用 border_color 绘制完整主体，再用背景色擦除中心，
        // 保留形状和尺寸并得到仅边框的外观。
        layer.stroke_polyline(
            &scaled_points,
            (width * scale) as f64,
            [
                border_color[0],
                border_color[1],
                border_color[2],
                body_alpha,
            ],
            true,
        );
        // 用背景色擦除中心填充（游戏区域为黑色）。
        let bg = crate::config::current()
            .render
            .standard
            .png
            .style
            .IMAGE_BACKGROUND_COLOR;
        layer.stroke_polyline(
            &scaled_points,
            (inner_width * scale) as f64,
            [bg[0], bg[1], bg[2], 255],
            true,
        );
    } else {
        layer.stroke_polyline(
            &scaled_points,
            (width * scale) as f64,
            [
                border_color[0],
                border_color[1],
                border_color[2],
                body_alpha,
            ],
            true,
        );
        layer.stroke_polyline(
            &scaled_points,
            (inner_width * scale) as f64,
            [inner_color[0], inner_color[1], inner_color[2], body_alpha],
            true,
        );
    }

    let resized = layer.resize((right - left).max(1) as u32, (bottom - top).max(1) as u32);
    CachedLayer {
        image: resized,
        offset: (left, top),
    }
}

// ——— 滑条数据 ———

pub fn get_slider_render_data(
    cache: &mut RenderCache,
    context: &RenderContext,
    index: usize,
) -> Arc<SliderRenderData> {
    if let Some(cached) = cache.slider_data.get(&index) {
        return Arc::clone(cached);
    }

    let hit_object = &context.hit_objects[index];
    let slider_type = hit_object.slider_type.as_deref().unwrap_or("B");
    let world_path = build_standard_slider_path(
        hit_object.x,
        hit_object.y,
        &hit_object.slider_points,
        slider_type,
        hit_object.slider_pixel_length,
    );
    let offset = stack_offset(hit_object, &context.settings);
    let frame_points: Vec<(f64, f64)> = world_path
        .points
        .iter()
        .map(|&(x, y)| to_frame_point(x + offset, y + offset, &context.frame_layout))
        .collect();
    let frame_path = build_path(&frame_points);

    let ticks = generate_slider_ticks(
        &frame_path,
        world_path.total_length,
        hit_object.start_time,
        hit_object.end_time,
        hit_object.slider_repeats,
        context
            .slider_timings
            .get(index)
            .copied()
            .unwrap_or((500.0, 1.0)),
        context.slider_tick_rate,
        context.slider_multiplier,
        context.settings.preempt_ms as f64,
    );

    let mut reverse_centers: Vec<(f64, f64)> = Vec::new();
    let mut reverse_angles: Vec<f64> = Vec::new();
    if frame_path.points.len() >= 2 {
        let n = frame_path.points.len();
        for repeat_index in 1..hit_object.slider_repeats.max(1) {
            let (center, dx, dy) = if repeat_index % 2 == 1 {
                let center = frame_path.points[n - 1];
                (
                    center,
                    frame_path.points[n - 2].0 - center.0,
                    frame_path.points[n - 2].1 - center.1,
                )
            } else {
                let center = frame_path.points[0];
                (
                    center,
                    frame_path.points[1].0 - center.0,
                    frame_path.points[1].1 - center.1,
                )
            };
            reverse_centers.push(center);
            reverse_angles.push(dy.atan2(dx));
        }
    }

    let head_center = frame_path.points.first().copied().unwrap_or((0.0, 0.0));
    let data = Arc::new(SliderRenderData {
        frame_path,
        head_center,
        reverse_centers,
        reverse_angles,
        ticks,
    });
    cache.slider_data.insert(index, Arc::clone(&data));
    data
}

/// 计算滑条 tick 的出现时间（毫秒，绝对谱面时间）。
///
/// 打击音只需要时间序列，不需要路径几何，因此用一条最短路径复用
/// [`generate_slider_ticks`]，保证画面 tick 与声音 tick 使用同一套 osu! 规则。
pub fn slider_tick_times(
    world_length: f64,
    start_time: i64,
    end_time: i64,
    repeats: i32,
    beat_length: f64,
    slider_velocity: f64,
    tick_rate: f64,
    slider_multiplier: f64,
) -> Vec<f64> {
    let dummy_path = build_path(&[(0.0, 0.0), (1.0, 0.0)]);
    generate_slider_ticks(
        &dummy_path,
        world_length,
        start_time,
        end_time,
        repeats,
        (beat_length, slider_velocity),
        tick_rate,
        slider_multiplier,
        0.0,
    )
    .into_iter()
    .map(|tick| tick.time)
    .collect()
}

/// 按 osu! SliderEventGenerator 规则生成可视化 tick。
#[allow(clippy::too_many_arguments)]
fn generate_slider_ticks(
    frame_path: &SliderPath,
    world_length: f64,
    start_time: i64,
    end_time: i64,
    repeats: i32,
    timing: (f64, f64),
    tick_rate: f64,
    slider_multiplier: f64,
    object_preempt: f64,
) -> Vec<SliderTickRenderData> {
    let span_count = repeats.max(1) as usize;
    let (beat_length, slider_velocity) = timing;
    if !world_length.is_finite()
        || world_length <= 0.0
        || !beat_length.is_finite()
        || beat_length <= 0.0
        || !slider_velocity.is_finite()
        || slider_velocity <= 0.0
        || !tick_rate.is_finite()
        || tick_rate <= 0.0
        || !slider_multiplier.is_finite()
        || slider_multiplier <= 0.0
    {
        return Vec::new();
    }

    // osu! 对异常长路径限制 tick 生成长度，避免损坏谱面造成过大的事件数量。
    let length = world_length.min(100_000.0);
    let scoring_distance = 100.0 * slider_multiplier * slider_velocity;
    let velocity = scoring_distance / beat_length;
    let tick_distance = (scoring_distance / tick_rate).clamp(0.0, length);
    let min_distance_from_end = velocity * 10.0;
    if !velocity.is_finite() || !tick_distance.is_finite() || tick_distance <= 0.0 {
        return Vec::new();
    }

    let span_duration = (end_time - start_time) as f64 / span_count as f64;
    let mut ticks = Vec::new();
    for span in 0..span_count {
        let span_start = start_time as f64 + span as f64 * span_duration;
        let reversed = span % 2 == 1;
        let mut distance = tick_distance;
        while distance < length && distance < length - min_distance_from_end {
            let path_progress = distance / length;
            let time_progress = if reversed {
                1.0 - path_progress
            } else {
                path_progress
            };
            let time = span_start + time_progress * span_duration;
            let time_preempt = if span > 0 {
                (time - span_start) / 2.0 + 200.0
            } else {
                (time - span_start) / 2.0 + object_preempt * 0.66
            };
            ticks.push(SliderTickRenderData {
                center: path_position_at(frame_path, path_progress),
                time,
                time_preempt: time_preempt.max(0.0),
            });
            distance += tick_distance;
        }
    }
    ticks
}

pub fn draw_slider_ticks(
    frame: &mut Img,
    context: &RenderContext,
    cache: &mut RenderCache,
    slider_data: &SliderRenderData,
    snapshot_time: i64,
    color: [u8; 3],
    parent_alpha: f64,
) {
    if parent_alpha <= 0.0 {
        return;
    }
    let tick_size =
        py_round(context.frame_circle_diameter as f64 * ARGON_SLIDER_TICK_SIZE_RATIO).max(1);
    let sprite_key = (tick_size, color);
    cached_slider_tick_sprite(cache, tick_size, color);

    for tick in &slider_data.ticks {
        let tick_alpha = super::alpha::slider_tick_alpha(
            tick.time,
            tick.time_preempt,
            snapshot_time,
            &context.settings,
        ) * parent_alpha;
        if tick_alpha <= 0.0 {
            continue;
        }
        let sprite = &cache.slider_tick_sprites[&sprite_key];
        let sprite_id = color_id(ID_SLIDER_TICK + tick_size as u64, color);
        let image = with_alpha(&mut cache.resized_alpha, sprite, sprite_id, tick_alpha);
        let x = py_round(tick.center.0 - image.w as f64 / 2.0);
        let y = py_round(tick.center.1 - image.h as f64 / 2.0);
        frame.alpha_composite(image, x, y);
    }
}

fn cached_slider_tick_sprite(cache: &mut RenderCache, size: i64, color: [u8; 3]) -> &Img {
    cache
        .slider_tick_sprites
        .entry((size, color))
        .or_insert_with(|| build_slider_tick(size, color))
}

fn build_slider_tick(size: i64, color: [u8; 3]) -> Img {
    let d = size.max(1);
    let mut image = Img::new(d as u32, d as u32, [0, 0, 0, 0]);
    let center = d as f64 / 2.0;
    let border = (d as f64 * ARGON_SLIDER_TICK_BORDER_RATIO).max(1.0);
    draw_ring_aa(
        &mut image,
        center,
        center,
        d as f64 / 2.0,
        border,
        [color[0], color[1], color[2], 255],
    );
    image
}

pub fn is_full_slider_body(snaked_start: f64, snaked_end: f64) -> bool {
    snaked_start <= 0.001 && snaked_end >= 0.999
}

pub fn slider_snaked_range(
    hit_object: &StandardHitObject,
    snapshot_time: i64,
    settings: &super::context::RenderSettings,
) -> (f64, f64) {
    let span_count = hit_object.slider_repeats.max(1) as i64;
    let mut start = 0.0;
    let mut end = 1.0;

    if snapshot_time < hit_object.start_time {
        if crate::render::cpu::modes::standard::constants::SNAKING_IN_SLIDERS {
            let snake_start = hit_object.start_time - settings.preempt_ms;
            end = ((snapshot_time - snake_start) as f64 / (settings.preempt_ms as f64 / 3.0))
                .clamp(0.0, 1.0);
        }
        return (start, end);
    }

    let effective_time = snapshot_time.min(hit_object.end_time);
    let completion = ((effective_time - hit_object.start_time) as f64
        / (hit_object.end_time - hit_object.start_time).max(1) as f64)
        .clamp(0.0, 1.0);
    let span = ((completion * span_count as f64) as i64).min(span_count - 1);
    let span_progress = super::alpha::slider_path_progress(span_count, completion);

    if span >= span_count - 1 && crate::render::cpu::modes::standard::constants::SNAKING_OUT_SLIDERS
    {
        if span % 2 == 1 {
            end = span_progress;
        } else {
            start = span_progress;
        }
    }
    (start, end)
}

// ——— 滑条球 ———

/// 滑条球方向箭头的几何参数（局部坐标，箭头指向 +X，单边尺寸对称于球心）。
///
/// 对照 lazer `ArgonSliderBall`：图标是 FontAwesome Solid `AngleRight`，
/// 其等比缩放后的墨迹高度为物件直径的 0.3 倍，两端与尖端为圆角笔画，
/// 这里按同一比例还原，端帽的圆角会让墨迹比字形略宽约 6%，视觉上可忽略。
pub struct SliderBallArrowGeometry {
    pub tip: (f64, f64),
    pub top: (f64, f64),
    pub bottom: (f64, f64),
    pub thickness: f64,
    /// 含圆头端帽的墨迹外接尺寸，用于生成精灵。
    pub size: (u32, u32),
}

pub fn slider_ball_arrow_geometry(circle_diameter: f64) -> SliderBallArrowGeometry {
    let height = (circle_diameter * ARGON_SLIDER_BALL_ARROW_HEIGHT_RATIO).max(1.0);
    let thickness = (height * ARGON_SLIDER_BALL_ARROW_THICKNESS_RATIO).max(1.0);
    let half_height = height * ARGON_SLIDER_BALL_ARROW_HALF_HEIGHT_RATIO;
    let tip_x = height * ARGON_SLIDER_BALL_ARROW_TIP_OFFSET_RATIO;
    // 折角外侧顶点比中心线尖端多出 (厚度/2)/sin45°，左右两侧对称，球心即墨迹中心。
    let half_width = tip_x + thickness / 2.0 * std::f64::consts::SQRT_2;
    let pad = 2.0_f64.max(height * 0.02);
    SliderBallArrowGeometry {
        tip: (tip_x, 0.0),
        top: (tip_x - half_height, -half_height),
        bottom: (tip_x - half_height, half_height),
        thickness,
        size: (
            (half_width * 2.0 + pad * 2.0).ceil().max(1.0) as u32,
            (height + pad * 2.0).ceil().max(1.0) as u32,
        ),
    }
}

/// 按 lazer `DrawableSliderBall.UpdateProgress` 的算法取滑条球当前的朝向。
///
/// `completion` 是滑条整体进度（0..1，从物件开始时间算起），路径进度必须经
/// [`super::alpha::slider_path_progress`] 折算，折返段才会自动反向。
/// 返回箭头应指向的屏幕角度（度，y 轴向下、顺时针为正）；方向向量长度小于
/// 0.01 时无法可靠求角（急折返或极短滑条），返回 `None` 由调用方跳过绘制。
pub fn slider_ball_arrow_angle(path: &SliderPath, span_count: i64, completion: f64) -> Option<f64> {
    if path.points.len() < 2 || !path.total_length.is_finite() || path.total_length <= 0.0 {
        return None;
    }
    let spans = span_count.max(1);
    // 游戏中取 0.1 个世界像素的采样距离；这里按路径长度归一化，坐标系缩放会约掉。
    let check = (0.1 / path.total_length).clamp(1e-6, 0.5);
    let position = |completion: f64| {
        path_position_at(
            path,
            super::alpha::slider_path_progress(spans, completion.clamp(0.0, 1.0)),
        )
    };
    let before = position((1.0 - check).min(completion));
    let after = position((completion + check).min(1.0));
    let (dx, dy) = (after.0 - before.0, after.1 - before.1);
    (dx.hypot(dy) >= 0.01).then(|| dy.atan2(dx).to_degrees())
}

/// 把滑条球箭头的局部坐标（指向 +X）旋转到帧坐标。
///
/// 屏幕坐标系 y 轴向下，`angle_deg` 为正表示视觉上的顺时针；与 CPU 路径使用的
/// `Img::rotate_expand(-angle_deg)` 等价，两条渲染路径因此得到同一朝向。
pub fn rotate_arrow_point(center: (f64, f64), local: (f64, f64), angle_deg: f64) -> (f64, f64) {
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    (
        center.0 + local.0 * cos - local.1 * sin,
        center.1 + local.0 * sin + local.1 * cos,
    )
}

/// 程序化绘制滑条球方向箭头（白色 `>` 字形），未旋转时指向 +X。
pub fn build_slider_ball_arrow(circle_diameter: f64) -> Img {
    let arrow = slider_ball_arrow_geometry(circle_diameter);
    let mut img = Img::new(arrow.size.0, arrow.size.1, [0, 0, 0, 0]);
    let cx = img.w as f64 / 2.0;
    let cy = img.h as f64 / 2.0;
    img.stroke_polyline(
        &[
            (cx + arrow.top.0, cy + arrow.top.1),
            (cx + arrow.tip.0, cy + arrow.tip.1),
            (cx + arrow.bottom.0, cy + arrow.bottom.1),
        ],
        arrow.thickness,
        [255, 255, 255, 255],
        true,
    );
    img
}

#[allow(clippy::too_many_arguments)]
pub fn draw_slider_ball(
    frame: &mut Img,
    context: &RenderContext,
    cache: &mut RenderCache,
    slider_data: &SliderRenderData,
    hit_object: &StandardHitObject,
    snapshot_time: i64,
    color: [u8; 3],
    alpha: f64,
) {
    if !(hit_object.start_time <= snapshot_time && snapshot_time <= hit_object.end_time) {
        return;
    }
    if alpha <= 0.0 {
        return;
    }

    let completion = (snapshot_time - hit_object.start_time) as f64
        / (hit_object.end_time - hit_object.start_time).max(1) as f64;
    let progress =
        super::alpha::slider_path_progress(hit_object.slider_repeats.max(1) as i64, completion);
    let center = path_position_at(&slider_data.frame_path, progress);

    {
        let follow = cache
            .procedural
            .entry((ID_FOLLOW, color))
            .or_insert_with(|| {
                build_follow_circle(
                    context.slider_follow_size,
                    context.frame_circle_diameter,
                    color,
                )
            });
        let img = with_alpha(
            &mut cache.resized_alpha,
            follow,
            color_id(ID_FOLLOW, color),
            alpha * 0.7,
        );
        let fx = py_round(center.0 - img.w as f64 / 2.0);
        let fy = py_round(center.1 - img.h as f64 / 2.0);
        frame.alpha_composite(img, fx, fy);
    }
    {
        let ball = cache
            .procedural
            .entry((ID_SLIDER_BALL, color))
            .or_insert_with(|| {
                build_slider_ball(
                    context.slider_ball_size,
                    context.frame_circle_diameter,
                    color,
                )
            });
        let img = with_alpha(
            &mut cache.resized_alpha,
            ball,
            color_id(ID_SLIDER_BALL, color),
            alpha,
        );
        let bx = py_round(center.0 - img.w as f64 / 2.0);
        let by = py_round(center.1 - img.h as f64 / 2.0);
        frame.alpha_composite(img, bx, by);
    }
    {
        // 方向箭头为白色，与游戏一致：不随 combo 颜色变化，只按角度缓存旋转结果。
        // 角度取整到 1°，与折返箭头一致地复用精灵。
        let Some(angle) = slider_ball_arrow_angle(
            &slider_data.frame_path,
            hit_object.slider_repeats.max(1) as i64,
            completion,
        ) else {
            return;
        };
        let angle_deg = -angle;
        let angle_key = py_round(angle_deg);
        let rotated = cache.ball_arrows.entry(angle_key).or_insert_with(|| {
            build_slider_ball_arrow(context.frame_circle_diameter as f64).rotate_expand(angle_deg)
        });
        let arrow = with_alpha(
            &mut cache.resized_alpha,
            rotated,
            ID_BALL_ARROW + (angle_key + 720) as u64,
            alpha,
        );
        let ox = py_round(center.0 - arrow.w as f64 / 2.0);
        let oy = py_round(center.1 - arrow.h as f64 / 2.0);
        frame.alpha_composite(arrow, ox, oy);
    }
}

fn build_slider_ball(diameter: i64, circle_diameter: i64, color: [u8; 3]) -> Img {
    let d = diameter.max(1);
    let mut img = Img::new(d as u32, d as u32, [0, 0, 0, 0]);
    let c = d as f64 / 2.0;
    let border = 2.5
        * circle_diameter as f64
        * crate::render::cpu::modes::standard::constants::ARGON_BORDER_RATIO;
    // C# Argon: fill = accentColour -> accentColour.Darken(0.5) 垂直渐变
    fill_circle_gradient_aa(&mut img, c, c, d as f64 / 2.0, color, darken(color, 0.5));
    draw_ring_aa(&mut img, c, c, d as f64 / 2.0, border, [255, 255, 255, 255]);
    img
}

fn build_follow_circle(diameter: i64, circle_diameter: i64, color: [u8; 3]) -> Img {
    let d = diameter.max(1);
    let mut img = Img::new(d as u32, d as u32, [0, 0, 0, 0]);
    let c = d as f64 / 2.0;
    let border = (4.0 * circle_diameter as f64 / 128.0).max(1.0);
    img.fill_circle_aa(
        c,
        c,
        d as f64 / 2.0 - border,
        [color[0], color[1], color[2], 77],
    );
    draw_ring_aa(
        &mut img,
        c,
        c,
        d as f64 / 2.0,
        border,
        [color[0], color[1], color[2], 255],
    );
    img
}

// ——— 反向箭头 ———

#[allow(clippy::too_many_arguments)]
pub fn draw_slider_reverse_arrows(
    frame: &mut Img,
    context: &RenderContext,
    cache: &mut RenderCache,
    slider_data: &SliderRenderData,
    hit_object: &StandardHitObject,
    snapshot_time: i64,
    snaked_start: f64,
    snaked_end: f64,
    color: [u8; 3],
    alpha: f64,
) {
    if hit_object.slider_repeats <= 1 {
        return;
    }

    let span_count = hit_object.slider_repeats.max(1) as f64;
    let duration = (hit_object.end_time - hit_object.start_time) as f64;
    let fade_out_ratio = (300.0_f64).min(duration / span_count) / duration.max(1.0);

    for (i, &center) in slider_data.reverse_centers.iter().enumerate() {
        let repeat_index = (i + 1) as i64;
        let position = if repeat_index % 2 == 1 { 1.0 } else { 0.0 };

        if !(snaked_start - 0.001 <= position && position <= snaked_end + 0.001) {
            continue;
        }

        let repeat_alpha;
        if snapshot_time < hit_object.start_time {
            if repeat_index > 1 {
                continue;
            }
            repeat_alpha = 1.0;
        } else {
            let completion = (snapshot_time - hit_object.start_time) as f64
                / (hit_object.end_time - hit_object.start_time).max(1) as f64;
            let traversal = completion * span_count;
            if traversal < (repeat_index - 1) as f64 {
                continue;
            }
            if traversal >= repeat_index as f64 {
                continue;
            }
            if traversal > repeat_index as f64 - fade_out_ratio {
                repeat_alpha = ((repeat_index as f64 - traversal) / fade_out_ratio).max(0.0);
            } else {
                repeat_alpha = 1.0;
            }
        }

        let effective_alpha = alpha * repeat_alpha;
        if effective_alpha <= 0.0 {
            continue;
        }

        let angle_deg = -slider_data.reverse_angles[i].to_degrees();
        let angle_key = py_round(angle_deg);

        // 只画 `»` 折返箭头：不叠加半透明弧光/渐变边缘，避免在深色背景与
        // 滑条尾部叠出一圈发灰的脏晕。
        let rotated_key = (angle_key, color);
        cache.reverse_arrows.entry(rotated_key).or_insert_with(|| {
            let base = build_reverse_arrow(context.frame_circle_diameter, color);
            base.rotate_expand(angle_deg)
        });
        let rotated = &cache.reverse_arrows[&rotated_key];
        let arrow_id = color_id(ID_ARROW_BASE + (angle_key + 720) as u64, color);
        let arrow = with_alpha(&mut cache.resized_alpha, rotated, arrow_id, effective_alpha);
        let ox = py_round(center.0 - arrow.w as f64 / 2.0);
        let oy = py_round(center.1 - arrow.h as f64 / 2.0);
        frame.alpha_composite(arrow, ox, oy);
    }
}

/// 程序化 Argon 折返图标（对照 lazer ArgonReverseArrow）：
/// 白色胶囊（lazer 为 40×20 / 128 物件）+ 深色 `»` 双 V 形图标
/// （lazer 的 FontAwesome AngleDoubleRight，icon 高约为胶囊高的 80%）。
/// 游戏内有 1.0→1.3 的脉冲缩放，静态图按 1.0 绘制，避免过大。
pub fn build_reverse_arrow(circle_diameter: i64, color: [u8; 3]) -> Img {
    let s = circle_diameter as f64 / 128.0; // 按 1.0 绘制，不放大
    let cap_w = 40.0 * s;
    let cap_h = 20.0 * s;
    let pad = 2.0_f64.max(2.0 * s);
    let w = (cap_w + pad * 2.0).ceil().max(1.0) as u32;
    let h = (cap_h + pad * 2.0).ceil().max(1.0) as u32;
    let mut img = Img::new(w, h, [0, 0, 0, 0]);
    let cx = w as f64 / 2.0;
    let cy = h as f64 / 2.0;

    // 白色胶囊主体
    let half_h = cap_h / 2.0;
    img.stroke_polyline(
        &[
            (cx - cap_w / 2.0 + half_h, cy),
            (cx + cap_w / 2.0 - half_h, cy),
        ],
        cap_h,
        [255, 255, 255, 255],
        true,
    );

    // 深色 `»` 图标：C# Argon = accent.Darken(4)
    // 图标高度约为胶囊高的 60%（原 72%，缩小一点）
    let dark = darken(color, 4.0);
    let dark_rgba = [dark[0], dark[1], dark[2], 255];
    let chev_h = cap_h * 0.60;
    let chev_w = chev_h * 0.50;
    let thickness = (cap_h * 0.15).max(1.5);
    let spacing = chev_w + thickness * 0.8;
    for k in [-0.5, 0.5] {
        let tip_x = cx + k * spacing + chev_w / 2.0;
        let back_x = tip_x - chev_w;
        img.stroke_polyline(
            &[
                (back_x, cy - chev_h / 2.0),
                (tip_x, cy),
                (back_x, cy + chev_h / 2.0),
            ],
            thickness,
            dark_rgba,
            true,
        );
    }
    img
}

// ——— 辅助函数 ———

/// 模拟 C# osu-framework 的 Color4.Darken(amount) 函数。
/// 将 RGB 通道各减去 amount * 255（加法变暗）。
/// 例如：Darken(0.1) = 每通道减 25.5，Darken(4) = 每通道减 1020（钳制到 0）。
pub fn darken(color: [u8; 3], amount: f64) -> [u8; 3] {
    let delta = py_round(amount * 255.0) as i32;
    [
        (color[0] as i32 - delta).clamp(0, 255) as u8,
        (color[1] as i32 - delta).clamp(0, 255) as u8,
        (color[2] as i32 - delta).clamp(0, 255) as u8,
    ]
}

pub fn alpha_to_byte(alpha: f64) -> u8 {
    py_round(alpha * 255.0).clamp(0, 255) as u8
}

pub fn apply_alpha_byte(img: &mut Img, alpha_byte: u8) {
    let factor = alpha_byte as f64 / 255.0;
    let mut lut = [0u8; 256];
    for (v, e) in lut.iter_mut().enumerate() {
        *e = py_round(v as f64 * factor).clamp(0, 255) as u8;
    }
    for px in img.data.chunks_exact_mut(4) {
        px[3] = lut[px[3] as usize];
    }
}

pub fn resized_with_alpha<'a>(
    cache: &'a mut HashMap<(u64, (u32, u32), u8), Img>,
    sprite_img: &Img,
    sprite_id: u64,
    size: (u32, u32),
    alpha: f64,
) -> &'a Img {
    let alpha_key = alpha_to_byte(alpha);
    let key = (sprite_id, size, alpha_key);
    cache.entry(key).or_insert_with(|| {
        let mut resized = sprite_img.resize(size.0, size.1);
        if alpha_key < 255 {
            apply_alpha_byte(&mut resized, alpha_key);
        }
        resized
    })
}

pub fn with_alpha<'a>(
    cache: &'a mut HashMap<(u64, (u32, u32), u8), Img>,
    img: &Img,
    id: u64,
    alpha: f64,
) -> &'a Img {
    let size = (img.w, img.h);
    resized_with_alpha(cache, img, id, size, alpha)
}

// ——— 抗锯齿绘制辅助函数 ———

pub fn fill_circle_gradient_aa(
    img: &mut Img,
    cx: f64,
    cy: f64,
    r: f64,
    top: [u8; 3],
    bottom: [u8; 3],
) {
    if r <= 0.0 {
        return;
    }
    let ya = (cy - r - 1.0).floor().max(0.0) as i64;
    let yb = (cy + r + 1.0).ceil().min(img.h as f64 - 1.0) as i64;
    let xa = (cx - r - 1.0).floor().max(0.0) as i64;
    let xb = (cx + r + 1.0).ceil().min(img.w as f64 - 1.0) as i64;
    // 每次步进 2 像素，在半分辨率采样并写入 2×2 区块。
    // 在抗锯齿圆上视觉差异可忽略，速度约提升 4 倍。
    let mut y = ya;
    while y <= yb {
        let t = ((y as f64 + 0.5 - (cy - r)) / (2.0 * r)).clamp(0.0, 1.0);
        let row = [
            py_round(top[0] as f64 + (bottom[0] as f64 - top[0] as f64) * t).clamp(0, 255) as u8,
            py_round(top[1] as f64 + (bottom[1] as f64 - top[1] as f64) * t).clamp(0, 255) as u8,
            py_round(top[2] as f64 + (bottom[2] as f64 - top[2] as f64) * t).clamp(0, 255) as u8,
        ];
        let mut x = xa;
        while x <= xb {
            let dx = x as f64 + 0.5 - cx;
            let dy = y as f64 + 0.5 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let cov = (r - dist + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let a = (255.0 * cov) as u8;
                let c = [row[0], row[1], row[2], a];
                img.blend_px(x, y, c);
                img.blend_px(x + 1, y, c);
                img.blend_px(x, y + 1, c);
                img.blend_px(x + 1, y + 1, c);
            }
            x += 2;
        }
        y += 2;
    }
}

pub fn draw_ring_aa(img: &mut Img, cx: f64, cy: f64, outer_r: f64, thickness: f64, color: [u8; 4]) {
    img.stroke_circle_aa(cx, cy, outer_r, thickness, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(length: f64) -> SliderPath {
        build_path(&[(0.0, 0.0), (length, 0.0)])
    }

    #[test]
    fn slider_ticks_follow_tick_distance_and_time() {
        let ticks = generate_slider_ticks(
            &path(100.0),
            100.0,
            0,
            1000,
            1,
            (500.0, 1.0),
            2.0,
            1.4,
            800.0,
        );
        assert_eq!(ticks.len(), 1);
        assert!((ticks[0].center.0 - 70.0).abs() < 1e-9);
        assert!((ticks[0].time - 700.0).abs() < 1e-9);
        assert!((ticks[0].time_preempt - 878.0).abs() < 1e-9);
    }

    #[test]
    fn repeated_slider_ticks_reverse_time_progress() {
        let ticks = generate_slider_ticks(
            &path(100.0),
            100.0,
            0,
            1000,
            2,
            (500.0, 1.0),
            2.0,
            1.4,
            800.0,
        );
        assert_eq!(ticks.len(), 2);
        assert!((ticks[0].time - 350.0).abs() < 1e-9);
        assert!((ticks[1].time - 650.0).abs() < 1e-9);
        assert!((ticks[0].center.0 - ticks[1].center.0).abs() < 1e-9);
    }

    #[test]
    fn slider_ticks_skip_points_near_span_end() {
        let ticks = generate_slider_ticks(
            &path(142.0),
            142.0,
            0,
            1000,
            1,
            (500.0, 1.0),
            2.0,
            1.4,
            800.0,
        );
        assert_eq!(ticks.len(), 1);
    }

    #[test]
    fn invalid_tick_inputs_generate_no_ticks() {
        assert!(generate_slider_ticks(
            &path(100.0),
            100.0,
            0,
            1000,
            1,
            (500.0, 1.0),
            0.0,
            1.4,
            800.0,
        )
        .is_empty());
    }

    #[test]
    fn slider_tick_sprite_cache_reuses_same_size_and_color() {
        let mut cache = RenderCache::default();
        let first = cached_slider_tick_sprite(&mut cache, 12, [255, 192, 0]) as *const Img;
        let second = cached_slider_tick_sprite(&mut cache, 12, [255, 192, 0]) as *const Img;
        assert_eq!(cache.slider_tick_sprites.len(), 1);
        assert_eq!(first, second);
    }

    #[test]
    fn slider_ball_arrow_matches_glyph_proportions() {
        // 器件直径 128 时，图标墨迹高度应为 0.3 × 128 = 38.4，上下对称、尖端在右。
        let arrow = slider_ball_arrow_geometry(128.0);
        let ink_height = arrow.top.1.abs() * 2.0 + arrow.thickness;
        assert!((ink_height - 38.4).abs() < 1e-9, "ink_height={ink_height}");
        assert!((arrow.top.1 + arrow.bottom.1).abs() < 1e-9);
        assert!(arrow.tip.0 > arrow.top.0);

        let sprite = build_slider_ball_arrow(128.0);
        let (left, top, right, bottom) = sprite.alpha_bbox().expect("箭头必须有可见像素");
        assert!(
            (right - left) as f64 >= 23.0 && (right - left) as f64 <= 27.0,
            "墨迹宽度应接近字形比例：{}",
            right - left
        );
        assert!(
            (bottom - top) as f64 >= 36.0 && (bottom - top) as f64 <= 40.0,
            "墨迹高度应接近字形高度：{}",
            bottom - top
        );
        // 球心即墨迹中心：精灵的墨迹包围盒应大致居中。
        assert!(((left + right) as f64 / 2.0 - sprite.w as f64 / 2.0).abs() <= 1.0);
        assert!(((top + bottom) as f64 / 2.0 - sprite.h as f64 / 2.0).abs() <= 1.0);
        let white = sprite
            .data
            .chunks_exact(4)
            .filter(|pixel| pixel == &[255, 255, 255, 255])
            .count();
        assert!(white > 0, "箭头应为不透明白色");
    }

    #[test]
    fn slider_ball_arrow_angle_follows_path_direction() {
        let right = build_path(&[(0.0, 0.0), (100.0, 0.0)]);
        assert!(slider_ball_arrow_angle(&right, 1, 0.5).unwrap().abs() < 1e-9);
        let down = build_path(&[(0.0, 0.0), (0.0, 100.0)]);
        assert!((slider_ball_arrow_angle(&down, 1, 0.5).unwrap() - 90.0).abs() < 1e-9);
        let up = build_path(&[(0.0, 100.0), (0.0, 0.0)]);
        assert!((slider_ball_arrow_angle(&up, 1, 0.5).unwrap() + 90.0).abs() < 1e-9);
        // 折线拐角处的方向取局部切线，不取整条路径的首尾连线。
        let corner = build_path(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)]);
        assert!(slider_ball_arrow_angle(&corner, 1, 0.25).unwrap().abs() < 1e-9);
        assert!((slider_ball_arrow_angle(&corner, 1, 0.75).unwrap() - 90.0).abs() < 1e-9);
        // 单点路径没有方向。
        assert!(slider_ball_arrow_angle(&build_path(&[(5.0, 5.0)]), 1, 0.5).is_none());
    }

    #[test]
    fn slider_ball_arrow_reverses_on_return_span() {
        // 两次滑行（1 个折返）：返程时球向左移动，箭头必须跟着反向。
        let right = build_path(&[(0.0, 0.0), (100.0, 0.0)]);
        assert!(slider_ball_arrow_angle(&right, 2, 0.25).unwrap().abs() < 1e-9);
        assert!((slider_ball_arrow_angle(&right, 2, 0.75).unwrap() - 180.0).abs() < 1e-9);
    }

    #[test]
    fn slider_ball_arrow_rotation_is_consistent_across_paths() {
        // GPU 路径按角度旋转坐标：90°（y 轴向下即“向下”）时局部 +X 的尖端落在球心下方。
        let (x, y) = rotate_arrow_point((10.0, 10.0), (4.0, 0.0), 90.0);
        assert!((x - 10.0).abs() < 1e-9 && (y - 14.0).abs() < 1e-9);

        // CPU 路径用 rotate_expand 旋转精灵：同一角度必须把右侧探针转到下方。
        let mut probe = Img::new(9, 9, [0, 0, 0, 0]);
        probe.put(8, 4, [255, 255, 255, 255]);
        let rotated = probe.rotate_expand(-90.0);
        let (left, top, right, bottom) = rotated.alpha_bbox().expect("探针像素必须保留");
        let (ink_x, ink_y) = ((left + right) as f64 / 2.0, (top + bottom) as f64 / 2.0);
        assert!(
            (ink_x - rotated.w as f64 / 2.0).abs() <= 1.0 && ink_y > rotated.h as f64 / 2.0,
            "rotate_expand(-90) 应把右侧探针转到下方：({ink_x}, {ink_y})"
        );
    }
}
