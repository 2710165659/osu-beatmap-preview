//! Hit object rendering: hit circles, sliders, spinners, approach circles.

use crate::canvas::Img;
use crate::models::{BreakPeriod, StandardHitObject};

use super::alpha::*;
use super::constants::*;
use super::context::{
    color_id, py_round, to_frame_point, RenderCache, RenderContext,
};
use super::slider::{
    darken, draw_cached_slider_body, draw_slider_body, draw_slider_ball, draw_slider_reverse_arrows,
    draw_ring_aa, fill_circle_gradient_aa, get_slider_render_data, is_full_slider_body,
    resized_with_alpha, slider_snaked_range, with_alpha,
};

// ——— frame rendering ———

pub(crate) fn render_frame(
    context: &RenderContext,
    cache: &mut RenderCache,
    snapshot_time: i64,
    break_periods: &[BreakPeriod],
    visible_indexes: &[usize],
) -> Img {
    let mut frame = Img::new(IMAGE_WIDTH as u32, IMAGE_HEIGHT as u32, IMAGE_BACKGROUND_COLOR);

    for &index in visible_indexes {
        let hit_object = &context.hit_objects[index];
        if hit_object.hit_type & 8 != 0 {
            draw_spinner(&mut frame, context, cache, hit_object, snapshot_time);
        } else if hit_object.hit_type & 2 != 0 {
            draw_slider(&mut frame, context, cache, index, snapshot_time);
        } else {
            draw_hit_circle(&mut frame, context, cache, index, snapshot_time);
        }
    }

    for &index in visible_indexes {
        let hit_object = &context.hit_objects[index];
        if hit_object.hit_type & 8 == 0 {
            draw_approach_circle(
                &mut frame,
                context,
                cache,
                hit_object,
                context.combo_info[index].color,
                snapshot_time,
            );
        }
    }

    if let Some(current_break) = current_break_period(break_periods, snapshot_time) {
        draw_break_overlay(&mut frame, current_break, snapshot_time);
    }

    frame
}

// ——— hit circle ———

fn draw_hit_circle(
    frame: &mut Img,
    context: &RenderContext,
    cache: &mut RenderCache,
    index: usize,
    snapshot_time: i64,
) {
    let hit_object = &context.hit_objects[index];
    let combo = context.combo_info[index];
    let alpha = object_alpha(
        hit_object.start_time,
        hit_object.start_time,
        snapshot_time,
        &context.settings,
    );
    let center = to_frame_point(
        hit_object.x as f64,
        hit_object.y as f64,
        &context.frame_layout,
    );
    draw_circle_piece(frame, context, cache, center, combo.color, alpha, &combo.number.to_string());
}

// ——— slider ———

fn draw_slider(
    frame: &mut Img,
    context: &RenderContext,
    cache: &mut RenderCache,
    index: usize,
    snapshot_time: i64,
) {
    let hit_object = &context.hit_objects[index];
    let combo = context.combo_info[index];
    let alpha = slider_body_alpha(hit_object, snapshot_time, &context.settings);
    let overlay_alpha = normal_object_alpha(
        hit_object.start_time,
        hit_object.end_time,
        snapshot_time,
        &context.settings,
    );
    let slider_data = get_slider_render_data(cache, context, index);
    let (snaked_start, snaked_end) =
        slider_snaked_range(hit_object, snapshot_time, &context.settings);
    if is_full_slider_body(snaked_start, snaked_end) {
        draw_cached_slider_body(frame, context, cache, index, &slider_data, combo.color, alpha);
    } else {
        let visible_path = crate::slider_path::slice_path(&slider_data.frame_path, snaked_start, snaked_end);
        draw_slider_body(frame, &visible_path, context.slider_body_width, combo.color, alpha);
    }

    draw_slider_reverse_arrows(
        frame, context, cache, &slider_data, hit_object, snapshot_time,
        snaked_start, snaked_end, combo.color, alpha,
    );
    draw_slider_ball(
        frame, context, cache, &slider_data, hit_object, snapshot_time,
        combo.color, overlay_alpha,
    );
    let head_alpha = slider_head_alpha(
        hit_object, snapshot_time, &context.settings, snaked_start, snaked_end,
    );
    if head_alpha > 0.0 {
        draw_circle_piece(
            frame, context, cache, slider_data.head_center,
            combo.color, head_alpha, &combo.number.to_string(),
        );
    }
}

// ——— spinner ———

fn draw_spinner(
    frame: &mut Img,
    context: &RenderContext,
    _cache: &mut RenderCache,
    hit_object: &StandardHitObject,
    snapshot_time: i64,
) {
    let alpha = spinner_alpha(hit_object, snapshot_time, &context.settings);
    if alpha <= 0.0 {
        return;
    }
    let center = to_frame_point(
        PLAYFIELD_WIDTH / 2.0,
        PLAYFIELD_HEIGHT / 2.0,
        &context.frame_layout,
    );
    let scale = context.spinner_size as f64 / 256.0;
    let base_r = 80.0 * scale;
    let alpha_byte = super::slider::alpha_to_byte(alpha);

    let progress = ((snapshot_time - hit_object.start_time) as f64
        / (hit_object.end_time - hit_object.start_time).max(1) as f64)
        .clamp(0.0, 1.0);
    let disc_r = base_r * (0.8 + 0.6 * progress);
    let pink = ARGON_SPINNER_PINK;
    frame.fill_circle_aa(
        center.0,
        center.1,
        disc_r,
        [pink[0], pink[1], pink[2], (30.0 * alpha) as u8],
    );

    draw_ring_aa(
        frame, center.0, center.1, base_r * 0.8,
        (10.0 * scale).max(1.0), [255, 255, 255, alpha_byte],
    );
    draw_ring_aa(
        frame, center.0, center.1, base_r,
        (3.0 * scale).max(1.0), [255, 255, 255, alpha_byte],
    );
}

// ——— approach circle ———

fn draw_approach_circle(
    frame: &mut Img,
    context: &RenderContext,
    _cache: &mut RenderCache,
    hit_object: &StandardHitObject,
    color: [u8; 3],
    snapshot_time: i64,
) {
    if context.settings.hidden {
        return;
    }
    if snapshot_time >= hit_object.start_time {
        return;
    }

    let elapsed = (snapshot_time - (hit_object.start_time - context.settings.preempt_ms)) as f64;
    let progress = (elapsed / context.settings.preempt_ms as f64).clamp(0.0, 1.0);
    let alpha = 0.9 * (elapsed / (context.settings.fade_in_ms * 2.0).max(1.0)).min(1.0);
    if alpha <= 0.0 {
        return;
    }
    let approach_scale = 4.0 - 3.0 * progress;
    let d = context.frame_circle_diameter as f64 * approach_scale;
    let center = to_frame_point(
        hit_object.x as f64,
        hit_object.y as f64,
        &context.frame_layout,
    );
    let thickness = (d * 0.03).max(1.0);
    draw_ring_aa(
        frame,
        center.0,
        center.1,
        d / 2.0,
        thickness,
        [color[0], color[1], color[2], super::slider::alpha_to_byte(alpha)],
    );
}

// ——— circle piece (hitcircle + overlay + number) ———

fn draw_circle_piece(
    frame: &mut Img,
    context: &RenderContext,
    cache: &mut RenderCache,
    center: (f64, f64),
    color: [u8; 3],
    alpha: f64,
    number: &str,
) {
    if alpha <= 0.0 {
        return;
    }
    let d = context.frame_circle_diameter;
    let pos_x = py_round(center.0 - d as f64 / 2.0);
    let pos_y = py_round(center.1 - d as f64 / 2.0);
    {
        let piece = cache
            .procedural
            .entry((ID_CIRCLE_PIECE, color))
            .or_insert_with(|| build_circle_piece(d, color));
        let img = with_alpha(
            &mut cache.resized_alpha,
            piece,
            color_id(ID_CIRCLE_PIECE, color),
            alpha,
        );
        frame.alpha_composite(img, pos_x, pos_y);
    }
    draw_number(frame, context, cache, number, center, d, alpha);
}

fn build_circle_piece(diameter: i64, color: [u8; 3]) -> Img {
    let d = diameter.max(1);
    let mut img = Img::new(d as u32, d as u32, [0, 0, 0, 0]);
    let c = d as f64 / 2.0;
    let border = d as f64 * ARGON_BORDER_RATIO;
    // C# Argon: outerFill = accentColour.Darken(4)
    let dark = darken(color, 4.0);

    // 1. outerFill: 深色填充圆
    img.fill_circle_aa(c, c, (d as f64 - 1.0) / 2.0, [dark[0], dark[1], dark[2], 255]);
    // 2. border: 白色外环
    draw_ring_aa(&mut img, c, c, d as f64 / 2.0, border, [255, 255, 255, 255]);

    // 3. outerGradient: 外层亮渐变 (accentColour -> accentColour.Darken(0.1))
    let outer_d = (d as f64 - 4.0 * border).max(0.0);
    fill_circle_gradient_aa(&mut img, c, c, outer_d / 2.0, color, darken(color, 0.1));

    // 4. innerGradient: 内层暗渐变 (accentColour.Darken(0.5) -> accentColour.Darken(0.6))
    let inner_d = (outer_d - 2.0 * 2.5 * border).max(0.0);
    fill_circle_gradient_aa(
        &mut img, c, c, inner_d / 2.0,
        darken(color, 0.5), darken(color, 0.6),
    );

    // 5. innerFill: 最内层深色填充 (同 outerFill 颜色)
    let fill_d = (inner_d - 2.0 * 2.5 * border).max(0.0);
    img.fill_circle_aa(c, c, fill_d / 2.0, [dark[0], dark[1], dark[2], 255]);
    img
}

fn draw_number(
    frame: &mut Img,
    context: &RenderContext,
    cache: &mut RenderCache,
    number: &str,
    center: (f64, f64),
    circle_diameter: i64,
    alpha: f64,
) {
    let digit_height = py_round(circle_diameter as f64 * 0.30).max(1);
    let digits: Vec<usize> = number
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| d as usize))
        .collect();
    if digits.is_empty() {
        return;
    }

    let widths: Vec<i64> = digits
        .iter()
        .map(|&d| {
            let crop = context.skin.digit_crops[d];
            py_round(crop.w as f64 * digit_height as f64 / crop.h.max(1) as f64).max(1)
        })
        .collect();
    let overlap = py_round(
        context.skin.hitcircle_overlap as f64 * digit_height as f64 / 100.0,
    );
    let total_width: i64 = widths.iter().sum::<i64>() - overlap * (digits.len() as i64 - 1);
    let mut x = py_round(center.0 - total_width as f64 / 2.0);
    let y = py_round(center.1 - digit_height as f64 / 2.0);

    for (&d, &w) in digits.iter().zip(widths.iter()) {
        let digit_img = resized_with_alpha(
            &mut cache.resized_alpha,
            context.skin.digit_crops[d],
            d as u64,
            (w as u32, digit_height as u32),
            alpha,
        );
        let dw = digit_img.w as i64;
        frame.alpha_composite(digit_img, x, y);
        x += dw - overlap;
    }
}

// ——— break overlay ———

fn current_break_period<'a>(
    break_periods: &'a [BreakPeriod],
    snapshot_time: i64,
) -> Option<&'a BreakPeriod> {
    break_periods
        .iter()
        .find(|p| break_overlay_alpha(p, snapshot_time) > 0.0)
}

fn draw_break_overlay(frame: &mut Img, break_period: &BreakPeriod, snapshot_time: i64) {
    let alpha = break_overlay_alpha(break_period, snapshot_time);
    if alpha <= 0.0 {
        return;
    }

    let mut layer = Img::new(frame.w, frame.h, [0, 0, 0, 0]);
    let center_x = IMAGE_WIDTH as f64 / 2.0;
    let center_y = IMAGE_HEIGHT as f64 / 2.0;

    draw_break_arrows(&mut layer, alpha);
    draw_break_remaining_bar(&mut layer, break_period, snapshot_time, center_x, center_y, alpha);

    let remaining_seconds =
        ((break_period.end_time - snapshot_time + 999).div_euclid(1000)).max(0);
    let counter_label = remaining_seconds.to_string();
    let (_, counter_h) =
        crate::text::text_size(&counter_label, BREAK_OVERLAY_COUNTER_FONT_SIZE);
    let counter_y = py_round(center_y - 15.0) - counter_h as i64;
    let counter_color = [
        BREAK_OVERLAY_COLOR[0],
        BREAK_OVERLAY_COLOR[1],
        BREAK_OVERLAY_COLOR[2],
        py_round(BREAK_OVERLAY_COLOR[3] as f64 * alpha).clamp(0, 255) as u8,
    ];
    draw_centered_text(
        &mut layer, &counter_label, 0, counter_y,
        BREAK_OVERLAY_COUNTER_FONT_SIZE, counter_color,
    );

    let break_label = format!(
        "Break {} - {}",
        crate::text::format_mmssmmm(break_period.start_time),
        crate::text::format_mmssmmm(break_period.end_time)
    );
    let info_y = py_round(center_y) + BREAK_OVERLAY_INFO_TOP_GAP;
    let info_color = [
        BREAK_OVERLAY_INFO_COLOR[0],
        BREAK_OVERLAY_INFO_COLOR[1],
        BREAK_OVERLAY_INFO_COLOR[2],
        py_round(BREAK_OVERLAY_INFO_COLOR[3] as f64 * alpha).clamp(0, 255) as u8,
    ];
    draw_centered_text(
        &mut layer, &break_label, 0, info_y,
        BREAK_OVERLAY_INFO_FONT_SIZE, info_color,
    );

    frame.alpha_composite(&layer, 0, 0);
}

fn draw_break_remaining_bar(
    layer: &mut Img,
    break_period: &BreakPeriod,
    snapshot_time: i64,
    center_x: f64,
    center_y: f64,
    alpha: f64,
) {
    let track_width = py_round(IMAGE_WIDTH as f64 * BREAK_OVERLAY_BAR_WIDTH_RATIO) as f64;
    let track_height = BREAK_OVERLAY_BAR_HEIGHT;
    let track_left = center_x - track_width / 2.0;
    let track_top = center_y - track_height / 2.0;
    layer.fill_rounded_rect(
        track_left,
        track_top,
        track_left + track_width,
        track_top + track_height,
        track_height / 2.0,
        [48, 48, 48, py_round(150.0 * alpha).clamp(0, 255) as u8],
    );

    let remaining_ratio = break_remaining_bar_ratio(break_period, snapshot_time);
    let fill_width = track_width * remaining_ratio;
    let fill_left = center_x - fill_width / 2.0;
    layer.fill_rounded_rect(
        fill_left,
        track_top,
        fill_left + fill_width,
        track_top + track_height,
        track_height / 2.0,
        [238, 238, 238, py_round(230.0 * alpha).clamp(0, 255) as u8],
    );
}

fn draw_break_arrows(layer: &mut Img, alpha: f64) {
    let color = [238, 238, 238, py_round(80.0 * alpha).clamp(0, 255) as u8];
    let glow_color = [238, 238, 238, py_round(35.0 * alpha).clamp(0, 255) as u8];
    let center_y = IMAGE_HEIGHT as f64 / 2.0;
    for (offset, direction) in [(-0.22, 1.0), (0.22, -1.0)] {
        let center_x = IMAGE_WIDTH as f64 / 2.0 + IMAGE_WIDTH as f64 * offset;
        draw_chevron(layer, center_x, center_y, 32.0, direction, glow_color, 9.0);
        draw_chevron(layer, center_x, center_y, 20.0, direction, color, 4.0);
    }
}

fn draw_chevron(
    layer: &mut Img,
    center_x: f64,
    center_y: f64,
    size: f64,
    direction: f64,
    color: [u8; 4],
    width: f64,
) {
    let half = size / 2.0;
    let point = (center_x + direction * half, center_y);
    let top = (center_x - direction * half, center_y - half);
    let bottom = (center_x - direction * half, center_y + half);
    layer.stroke_polyline(&[top, point, bottom], width, color, false);
}

fn break_overlay_alpha(break_period: &BreakPeriod, snapshot_time: i64) -> f64 {
    if break_period.end_time - break_period.start_time < BREAK_MIN_DURATION_MS {
        return 0.0;
    }
    if snapshot_time < break_period.start_time || snapshot_time > break_period.end_time {
        return 0.0;
    }
    if snapshot_time < break_period.start_time + BREAK_FADE_DURATION_MS {
        return (snapshot_time - break_period.start_time) as f64 / BREAK_FADE_DURATION_MS as f64;
    }
    if snapshot_time > break_period.end_time - BREAK_FADE_DURATION_MS {
        return (break_period.end_time - snapshot_time) as f64 / BREAK_FADE_DURATION_MS as f64;
    }
    1.0
}

fn break_remaining_bar_ratio(break_period: &BreakPeriod, snapshot_time: i64) -> f64 {
    let effective_duration =
        break_period.end_time - BREAK_FADE_DURATION_MS - break_period.start_time;
    if effective_duration <= 0 {
        return 0.0;
    }
    let remaining = break_period.end_time - BREAK_FADE_DURATION_MS - snapshot_time;
    (remaining as f64 / effective_duration as f64).clamp(0.0, 1.0)
}

fn draw_centered_text(canvas: &mut Img, text: &str, x: i64, y: i64, size: u32, color: [u8; 4]) {
    let (text_w, _) = crate::text::text_size(text, size);
    let text_x = x + (IMAGE_WIDTH - text_w as i64) / 2;
    crate::text::draw_text(canvas, text_x, y, text, size, color);
}
