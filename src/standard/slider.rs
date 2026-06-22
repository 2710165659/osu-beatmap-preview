//! Slider rendering: path data, body, ball, reverse arrows for osu!standard.

use crate::canvas::Img;
use crate::models::StandardHitObject;
use crate::common::slider_path::{
    build_path, build_standard_slider_path, path_position_at, SliderPath,
};
use std::collections::HashMap;
use std::sync::Arc;

use super::constants::*;
use super::context::{
    color_id, py_round, to_frame_point, CachedLayer, RenderCache, RenderContext,
};

// ——— data ———

pub(crate) struct SliderRenderData {
    pub(crate) frame_path: SliderPath,
    pub(crate) head_center: (f64, f64),
    pub(crate) reverse_centers: Vec<(f64, f64)>,
    pub(crate) reverse_angles: Vec<f64>,
}

// ——— slider body ———

pub(crate) fn draw_slider_body(
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
    let layer = render_slider_body_layer(points, width, color, alpha_to_byte(alpha), traceable);
    frame.alpha_composite(&layer.image, layer.offset.0, layer.offset.1);
}

pub(crate) fn draw_cached_slider_body(
    frame: &mut Img,
    context: &RenderContext,
    cache: &mut RenderCache,
    index: usize,
    slider_data: &SliderRenderData,
    color: [u8; 3],
    alpha: f64,
    traceable: bool,
) {
    let cache_key = (index, traceable);
    if !cache.slider_body_layers.contains_key(&cache_key) {
        let layer = render_slider_body_layer(
            &slider_data.frame_path.points,
            context.slider_body_width,
            color,
            255,
            traceable,
        );
        cache.slider_body_layers.insert(cache_key, layer);
    }

    let alpha_key = alpha_to_byte(alpha);
    let (offset_x, offset_y) = {
        let layer = &cache.slider_body_layers[&cache_key];
        layer.offset
    };
    if alpha_key >= 255 {
        let layer = &cache.slider_body_layers[&cache_key];
        frame.alpha_composite(&layer.image, offset_x, offset_y);
        return;
    }

    let key = (index, alpha_key);
    if !cache.slider_body_alpha_layers.contains_key(&key) {
        let scaled = cache.slider_body_layers[&cache_key]
            .image
            .scale_alpha(alpha_key as f64 / 255.0);
        cache.slider_body_alpha_layers.insert(key, scaled);
    }
    frame.alpha_composite(&cache.slider_body_alpha_layers[&key], offset_x, offset_y);
}

pub(crate) fn render_slider_body_layer(
    points: &[(f64, f64)],
    width: i64,
    color: [u8; 3],
    alpha_byte: u8,
    traceable: bool,
) -> CachedLayer {
    let scale = SLIDER_BODY_SUPERSAMPLE;
    let pad = (width + 4) as f64;
    let min_x = points.iter().map(|p| p.0).fold(f64::MAX, f64::min);
    let min_y = points.iter().map(|p| p.1).fold(f64::MAX, f64::min);
    let max_x = points.iter().map(|p| p.0).fold(f64::MIN, f64::max);
    let max_y = points.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    let left = ((min_x - pad).floor() as i64).max(0);
    let top = ((min_y - pad).floor() as i64).max(0);
    let right = ((max_x + pad).ceil() as i64).min(IMAGE_WIDTH);
    let bottom = ((max_y + pad).ceil() as i64).min(IMAGE_HEIGHT);

    let layer_w = ((right - left) * scale).max(1) as u32;
    let layer_h = ((bottom - top) * scale).max(1) as u32;
    let mut layer = Img::new(layer_w, layer_h, [0, 0, 0, 0]);
    let scaled_points: Vec<(f64, f64)> = points
        .iter()
        .map(|&(x, y)| ((x - left as f64) * scale as f64, (y - top as f64) * scale as f64))
        .collect();

    let inner_width = py_round(width as f64 * (1.0 - ARGON_SLIDER_BORDER_PORTION)).max(1);
    let body_alpha = py_round(alpha_byte as f64 * ARGON_SLIDER_BODY_ALPHA).clamp(0, 255) as u8;
    // 边框颜色：使用 combo 颜色（与 C# Argon 的 AccentColour 一致）
    // 轨道内部颜色：使用 Darken(4)（与 C# Argon 的 AccentColour.Darken(4) 一致）
    // 注意：skin.ini 的 SliderBorder / SliderTrackOverride 会被忽略，以匹配 Argon 风格
    let border_color = color;
    let inner_color = darken(color, 4.0);

    if traceable {
        // TC: AccentColour=transparent, BorderColour=accent
        // Draw full body shape with border_color, then erase center with background color
        // to create a hollow border-only look while preserving shape/size.
        layer.stroke_polyline(
            &scaled_points,
            (width * scale) as f64,
            [border_color[0], border_color[1], border_color[2], body_alpha],
            true,
        );
        // Erase the center fill with background color (playfield is black)
        let bg = IMAGE_BACKGROUND_COLOR;
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
            [border_color[0], border_color[1], border_color[2], body_alpha],
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

// ——— slider data ———

pub(crate) fn get_slider_render_data(
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
    let frame_points: Vec<(f64, f64)> = world_path
        .points
        .iter()
        .map(|&(x, y)| to_frame_point(x, y, &context.frame_layout))
        .collect();
    let frame_path = build_path(&frame_points);

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
    });
    cache.slider_data.insert(index, Arc::clone(&data));
    data
}

pub(crate) fn is_full_slider_body(snaked_start: f64, snaked_end: f64) -> bool {
    snaked_start <= 0.001 && snaked_end >= 0.999
}

pub(crate) fn slider_snaked_range(
    hit_object: &StandardHitObject,
    snapshot_time: i64,
    settings: &super::context::RenderSettings,
) -> (f64, f64) {
    let span_count = hit_object.slider_repeats.max(1) as i64;
    let mut start = 0.0;
    let mut end = 1.0;

    if snapshot_time < hit_object.start_time {
        if SNAKING_IN_SLIDERS {
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

    if span >= span_count - 1 && SNAKING_OUT_SLIDERS {
        if span % 2 == 1 {
            end = span_progress;
        } else {
            start = span_progress;
        }
    }
    (start, end)
}

// ——— slider ball ———

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_slider_ball(
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
}

fn build_slider_ball(diameter: i64, circle_diameter: i64, color: [u8; 3]) -> Img {
    let d = diameter.max(1);
    let mut img = Img::new(d as u32, d as u32, [0, 0, 0, 0]);
    let c = d as f64 / 2.0;
    let border = 2.5 * circle_diameter as f64 * ARGON_BORDER_RATIO;
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

// ——— reverse arrows ———

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_slider_reverse_arrows(
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

        // 绘制 repeat-edge-piece（白色半圆 + 水平 alpha 渐变）
        let edge_key = (angle_key,);  // 不依赖颜色，纯白色
        if !cache.reverse_edges.contains_key(&edge_key) {
            let edge_base = build_reverse_edge_piece(context.frame_circle_diameter);
            cache
                .reverse_edges
                .insert(edge_key, edge_base.rotate_expand(angle_deg));
        }
        let edge_rotated = &cache.reverse_edges[&edge_key];
        let edge_id = color_id(ID_REVERSE_EDGE + (angle_key + 720) as u64, [255, 255, 255]);
        let edge = with_alpha(&mut cache.resized_alpha, edge_rotated, edge_id, effective_alpha);
        let ex = py_round(center.0 - edge.w as f64 / 2.0);
        let ey = py_round(center.1 - edge.h as f64 / 2.0);
        frame.alpha_composite(edge, ex, ey);

        // 绘制 << 箭头（覆盖在 edge piece 之上）
        let rotated_key = (angle_key, color);
        if !cache.reverse_arrows.contains_key(&rotated_key) {
            let base = build_reverse_arrow(context.frame_circle_diameter, color);
            cache
                .reverse_arrows
                .insert(rotated_key, base.rotate_expand(angle_deg));
        }
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
fn build_reverse_arrow(circle_diameter: i64, color: [u8; 3]) -> Img {
    let s = circle_diameter as f64 / 128.0;  // 按 1.0 绘制，不放大
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
        &[(cx - cap_w / 2.0 + half_h, cy), (cx + cap_w / 2.0 - half_h, cy)],
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

/// 程序化 Argon 折返边缘纹理（对照 repeat-edge-piece.png）。
/// 白色左半圆 + 水平 alpha 渐变：从左边缘 A=127 线性衰减到右边缘 A=0。
/// 200×200 原始纹理的像素分析确认：alpha 只取决于 x 位置，半圆边界由弧形自然裁剪。
fn build_reverse_edge_piece(diameter: i64) -> Img {
    let d = diameter.max(1);
    let mut img = Img::new(d as u32, d as u32, [0, 0, 0, 0]);
    let cx = d as f64 / 2.0;
    let cy = d as f64 / 2.0;
    let r = d as f64 / 2.0;

    for y in 0..d {
        for x in 0..d {
            let fx = x as f64 + 0.5;
            let fy = y as f64 + 0.5;
            // 左半圆边界：点在圆内 且 x <= cx
            let dx = fx - cx;
            let dy = fy - cy;
            if dx * dx + dy * dy > r * r {
                continue;
            }
            if fx > cx {
                continue;
            }
            // 水平 alpha 渐变：从左边缘 A=127 线性衰减到右边缘 A=0
            let t = fx / r; // 0.0 (左) -> 1.0 (圆心/右边缘)
            let a = (127.0 * (1.0 - t)).clamp(0.0, 255.0) as u8;
            if a > 0 {
                img.blend_px(x, y, [255, 255, 255, a]);
            }
        }
    }
    img
}

// ——— helpers ———

/// 模拟 C# osu-framework 的 Color4.Darken(amount) 函数。
/// 将 RGB 通道各减去 amount * 255（加法变暗）。
/// 例如：Darken(0.1) = 每通道减 25.5，Darken(4) = 每通道减 1020（钳制到 0）。
pub(crate) fn darken(color: [u8; 3], amount: f64) -> [u8; 3] {
    let delta = py_round(amount * 255.0) as i32;
    [
        (color[0] as i32 - delta).clamp(0, 255) as u8,
        (color[1] as i32 - delta).clamp(0, 255) as u8,
        (color[2] as i32 - delta).clamp(0, 255) as u8,
    ]
}

pub(crate) fn alpha_to_byte(alpha: f64) -> u8 {
    py_round(alpha * 255.0).clamp(0, 255) as u8
}

pub(crate) fn apply_alpha_byte(img: &mut Img, alpha_byte: u8) {
    let factor = alpha_byte as f64 / 255.0;
    let mut lut = [0u8; 256];
    for (v, e) in lut.iter_mut().enumerate() {
        *e = py_round(v as f64 * factor).clamp(0, 255) as u8;
    }
    for px in img.data.chunks_exact_mut(4) {
        px[3] = lut[px[3] as usize];
    }
}

pub(crate) fn resized_with_alpha<'a>(
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

pub(crate) fn with_alpha<'a>(
    cache: &'a mut HashMap<(u64, (u32, u32), u8), Img>,
    img: &Img,
    id: u64,
    alpha: f64,
) -> &'a Img {
    let size = (img.w, img.h);
    resized_with_alpha(cache, img, id, size, alpha)
}

// ——— AA drawing helpers ———

pub(crate) fn fill_circle_gradient_aa(
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
    // Step by 2 px — sample at half resolution, write 2×2 blocks.
    // ~4× speed-up with negligible visual difference on AA circles.
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

pub(crate) fn draw_ring_aa(
    img: &mut Img,
    cx: f64,
    cy: f64,
    outer_r: f64,
    thickness: f64,
    color: [u8; 4],
) {
    if outer_r <= 0.0 || thickness <= 0.0 || color[3] == 0 {
        return;
    }
    let inner_r = (outer_r - thickness).max(0.0);
    let ya = (cy - outer_r - 1.0).floor().max(0.0) as i64;
    let yb = (cy + outer_r + 1.0).ceil().min(img.h as f64 - 1.0) as i64;
    let xa = (cx - outer_r - 1.0).floor().max(0.0) as i64;
    let xb = (cx + outer_r + 1.0).ceil().min(img.w as f64 - 1.0) as i64;
    for y in ya..=yb {
        for x in xa..=xb {
            let dx = x as f64 + 0.5 - cx;
            let dy = y as f64 + 0.5 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let cov =
                (outer_r - dist + 0.5).clamp(0.0, 1.0) * (dist - inner_r + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                img.blend_px(
                    x,
                    y,
                    [color[0], color[1], color[2], (color[3] as f64 * cov) as u8],
                );
            }
        }
    }
}
