//! 音符对象渲染：打击圈、滑条、转盘和接近圈。

use crate::model::{BreakPeriod, StandardHitObject};
use crate::render::canvas::Img;

use super::alpha::*;
use super::constants::*;
use super::context::{
    color_id, py_round, stacked_position, to_frame_point, RenderCache, RenderContext,
};
use super::follow_points::{
    build_sprite, follow_point_position, follow_point_state, sprite_height_key,
};
use super::slider::{
    darken, draw_cached_slider_body, draw_ring_aa, draw_slider_ball, draw_slider_body,
    draw_slider_reverse_arrows, draw_slider_ticks, fill_circle_gradient_aa, get_slider_render_data,
    is_full_slider_body, resized_with_alpha, slider_snaked_range, with_alpha,
};
use crate::render::text::{draw_text, text_size};

// ——— 帧渲染 ———

/// 在指定内容宽度内以 `(x, y)` 为基准水平居中绘制文字。
fn draw_centered_text(
    canvas: &mut Img,
    text: &str,
    x: i64,
    y: i64,
    size: u32,
    color: [u8; 4],
    content_width: i64,
) {
    let (text_w, _) = text_size(text, size);
    let text_x = x + (content_width - text_w as i64) / 2;
    draw_text(canvas, text_x, y, text, size, color);
}

pub fn render_frame(
    context: &RenderContext,
    cache: &mut RenderCache,
    snapshot_time: i64,
    break_periods: &[BreakPeriod],
    visible_indexes: &[usize],
    background: Option<&Img>,
) -> Img {
    let mut frame = match background {
        Some(background) => background.clone(),
        None => {
            let color = match context.output_format {
                crate::render::geometry::OutputFormat::Png => {
                    crate::config::current()
                        .render
                        .standard
                        .png
                        .style
                        .IMAGE_BACKGROUND_COLOR
                }
                crate::render::geometry::OutputFormat::Gif => {
                    crate::config::current()
                        .render
                        .standard
                        .gif
                        .style
                        .IMAGE_BACKGROUND_COLOR
                }
                crate::render::geometry::OutputFormat::Mp4 => {
                    crate::config::current()
                        .render
                        .standard
                        .mp4
                        .style
                        .IMAGE_BACKGROUND_COLOR
                }
            };
            // 视频物件层的帧是最终画布，底色只填内容框：内容框之外的补边仍由
            // 合成阶段用画布底色处理，物件则可以溢出到画布边缘而不被切掉。
            let content = context.frame_layout.content;
            let mut frame = Img::new(
                context.frame_layout.frame_width as u32,
                context.frame_layout.frame_height as u32,
                [0, 0, 0, 0],
            );
            frame.fill_rect_size(content.x, content.y, content.width, content.height, color);
            frame
        }
    };

    // 跟随点在游戏里位于物件层之下（`OsuPlayfield` 的 FollowPoints 早于
    // HitObjectContainer），因此最先绘制，会被随后画出的物件覆盖。
    draw_follow_points(&mut frame, context, cache, snapshot_time);

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
                index,
                context.combo_info[index].color,
                snapshot_time,
            );
        }
    }

    if let Some(current_break) = current_break_period(break_periods, snapshot_time) {
        draw_break_overlay(&mut frame, current_break, snapshot_time, context);
    }

    frame
}

// ——— 跟随点 ———

/// 绘制当前时刻所有存活区间的跟随点。
///
/// 每个跟随点的存活区间只有 preempt + TimeFadeIn（AR5 下 1.2 秒），比整条连接
/// 短得多，因此逐帧扫描全部跟随点并跳过区间外的即可，不需要额外的索引结构。
fn draw_follow_points(
    frame: &mut Img,
    context: &RenderContext,
    cache: &mut RenderCache,
    snapshot_time: i64,
) {
    for point in &context.follow_points {
        let (alpha, progress) = follow_point_state(point, snapshot_time);
        if alpha <= 0.0 {
            continue;
        }
        let height = sprite_height_key(&context.settings, context.frame_layout.scale, progress);
        // 旋转角度取整到 1°，与滑条球箭头、折返箭头一致地复用同一张精灵。
        let angle = py_round(point.rotation);
        let sprite = cache
            .follow_point_sprites
            .entry((height, angle))
            .or_insert_with(|| build_sprite(height as f64, angle as f64));
        let world = follow_point_position(point, progress);
        let center = to_frame_point(world.0, world.1, &context.frame_layout);
        let x = py_round(center.0 - sprite.w as f64 / 2.0);
        let y = py_round(center.1 - sprite.h as f64 / 2.0);
        frame.alpha_composite_scaled(sprite, x, y, alpha);
    }
}

// ——— 打击圈 ———

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
    if context.settings.traceable {
        // TC：只显示接近圈，跳过打击圈主体。
        return;
    }
    let position = stacked_position(hit_object, &context.settings);
    let center = to_frame_point(position.0, position.1, &context.frame_layout);
    draw_circle_piece(
        frame,
        context,
        cache,
        center,
        combo.color,
        alpha,
        &combo.number.to_string(),
    );
}

// ——— 滑条 ———

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
        draw_cached_slider_body(
            frame,
            context,
            cache,
            index,
            &slider_data,
            combo.color,
            alpha,
            context.settings.traceable,
        );
    } else {
        let visible_path =
            crate::processing::path::slice_path(&slider_data.frame_path, snaked_start, snaked_end);
        draw_slider_body(
            frame,
            &visible_path,
            context.slider_body_width,
            combo.color,
            alpha,
            context.settings.traceable,
        );
    }

    draw_slider_ticks(
        frame,
        context,
        cache,
        &slider_data,
        snapshot_time,
        combo.color,
        overlay_alpha,
    );

    draw_slider_reverse_arrows(
        frame,
        context,
        cache,
        &slider_data,
        hit_object,
        snapshot_time,
        snaked_start,
        snaked_end,
        combo.color,
        alpha,
    );
    draw_slider_ball(
        frame,
        context,
        cache,
        &slider_data,
        hit_object,
        snapshot_time,
        combo.color,
        overlay_alpha,
    );
    let head_alpha = slider_head_alpha(
        hit_object,
        snapshot_time,
        &context.settings,
        snaked_start,
        snaked_end,
    );
    if head_alpha > 0.0 && !context.settings.traceable {
        draw_circle_piece(
            frame,
            context,
            cache,
            slider_data.head_center,
            combo.color,
            head_alpha,
            &combo.number.to_string(),
        );
    }
}

// ——— 转盘 ———

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
        super::constants::PLAYFIELD_WIDTH / 2.0,
        super::constants::PLAYFIELD_HEIGHT / 2.0,
        &context.frame_layout,
    );
    let scale = context.spinner_size as f64 / 256.0;
    let base_r = 80.0 * scale;
    let alpha_byte = super::slider::alpha_to_byte(alpha);

    let progress = ((snapshot_time - hit_object.start_time) as f64
        / (hit_object.end_time - hit_object.start_time).max(1) as f64)
        .clamp(0.0, 1.0);
    let disc_r = base_r * (0.8 + 0.6 * progress);
    let pink = super::constants::ARGON_SPINNER_PINK;
    frame.fill_circle_aa(
        center.0,
        center.1,
        disc_r,
        [pink[0], pink[1], pink[2], (30.0 * alpha) as u8],
    );

    draw_ring_aa(
        frame,
        center.0,
        center.1,
        base_r * 0.8,
        (10.0 * scale).max(1.0),
        [255, 255, 255, alpha_byte],
    );
    draw_ring_aa(
        frame,
        center.0,
        center.1,
        base_r,
        (3.0 * scale).max(1.0),
        [255, 255, 255, alpha_byte],
    );
}

// ——— 接近圈 ———

fn draw_approach_circle(
    frame: &mut Img,
    context: &RenderContext,
    _cache: &mut RenderCache,
    index: usize,
    color: [u8; 3],
    snapshot_time: i64,
) {
    let hit_object = &context.hit_objects[index];
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
    let position = stacked_position(hit_object, &context.settings);
    let center = to_frame_point(position.0, position.1, &context.frame_layout);
    let thickness = (d * 0.03).max(1.0);
    draw_ring_aa(
        frame,
        center.0,
        center.1,
        d / 2.0,
        thickness,
        [
            color[0],
            color[1],
            color[2],
            super::slider::alpha_to_byte(alpha),
        ],
    );
}

// ——— 打击圈主体（打击圈 + 覆盖层 + 数字） ———

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
    let border = d as f64 * super::constants::ARGON_BORDER_RATIO;
    // C# Argon：outerFill = accentColour.Darken(4)。
    let dark = darken(color, 4.0);

    // 1. outerFill: 深色填充圆
    img.fill_circle_aa(
        c,
        c,
        (d as f64 - 1.0) / 2.0,
        [dark[0], dark[1], dark[2], 255],
    );
    // 2. border: 白色外环
    draw_ring_aa(&mut img, c, c, d as f64 / 2.0, border, [255, 255, 255, 255]);

    // 3. outerGradient: 外层亮渐变 (accentColour -> accentColour.Darken(0.1))
    let outer_d = (d as f64 - 4.0 * border).max(0.0);
    fill_circle_gradient_aa(&mut img, c, c, outer_d / 2.0, color, darken(color, 0.1));

    // 4. innerGradient: 内层暗渐变 (accentColour.Darken(0.5) -> accentColour.Darken(0.6))
    let inner_d = (outer_d - 2.0 * 2.5 * border).max(0.0);
    fill_circle_gradient_aa(
        &mut img,
        c,
        c,
        inner_d / 2.0,
        darken(color, 0.5),
        darken(color, 0.6),
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
    let overlap = py_round(context.skin.hitcircle_overlap as f64 * digit_height as f64 / 100.0);
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

// ——— Break 覆盖层 ———

fn current_break_period(break_periods: &[BreakPeriod], snapshot_time: i64) -> Option<&BreakPeriod> {
    break_periods
        .iter()
        .find(|p| break_overlay_alpha(p, snapshot_time) > 0.0)
}

fn draw_break_overlay(
    frame: &mut Img,
    break_period: &BreakPeriod,
    snapshot_time: i64,
    context: &RenderContext,
) {
    let alpha = break_overlay_alpha(break_period, snapshot_time);
    if alpha <= 0.0 {
        return;
    }

    let mut layer = Img::new(frame.w, frame.h, [0, 0, 0, 0]);
    let center_x = context.frame_layout.frame_width as f64 / 2.0;
    let center_y = context.frame_layout.frame_height as f64 / 2.0;
    let render_scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Standard,
        context.output_format,
    );

    draw_break_arrows(&mut layer, alpha, render_scale);
    draw_break_remaining_bar(
        &mut layer,
        break_period,
        snapshot_time,
        center_x,
        center_y,
        alpha,
        render_scale,
    );

    let remaining_seconds = ((break_period.end_time - snapshot_time + 999).div_euclid(1000)).max(0);
    let counter_label = remaining_seconds.to_string();
    let (_, counter_h) = crate::render::text::text_size(
        &counter_label,
        crate::render::text::scaled_bitmap_font_height(
            super::constants::BREAK_OVERLAY_COUNTER_FONT_SIZE,
            render_scale,
        ),
    );
    let counter_y =
        py_round(center_y - crate::render::geometry::scale_px(15.0, render_scale) as f64)
            - counter_h as i64;
    let counter_color = [
        super::constants::BREAK_OVERLAY_COLOR[0],
        super::constants::BREAK_OVERLAY_COLOR[1],
        super::constants::BREAK_OVERLAY_COLOR[2],
        py_round(super::constants::BREAK_OVERLAY_COLOR[3] as f64 * alpha).clamp(0, 255) as u8,
    ];
    draw_centered_text(
        &mut layer,
        &counter_label,
        0,
        counter_y,
        crate::render::text::scaled_bitmap_font_height(
            super::constants::BREAK_OVERLAY_COUNTER_FONT_SIZE,
            render_scale,
        ),
        counter_color,
        context.frame_layout.frame_width,
    );

    let break_label = format!(
        "Break {} - {}",
        crate::render::text::format_mmssmmm(context.time_axis.to_display(break_period.start_time)),
        crate::render::text::format_mmssmmm(context.time_axis.to_display(break_period.end_time))
    );
    let info_y = py_round(center_y)
        + crate::render::geometry::scale_px(
            super::constants::BREAK_OVERLAY_INFO_TOP_GAP as f64,
            render_scale,
        );
    let info_color = [
        super::constants::BREAK_OVERLAY_INFO_COLOR[0],
        super::constants::BREAK_OVERLAY_INFO_COLOR[1],
        super::constants::BREAK_OVERLAY_INFO_COLOR[2],
        py_round(super::constants::BREAK_OVERLAY_INFO_COLOR[3] as f64 * alpha).clamp(0, 255) as u8,
    ];
    draw_centered_text(
        &mut layer,
        &break_label,
        0,
        info_y,
        crate::render::text::scaled_bitmap_font_height(
            super::constants::BREAK_OVERLAY_INFO_FONT_SIZE,
            render_scale,
        ),
        info_color,
        context.frame_layout.frame_width,
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
    render_scale: f64,
) {
    let track_width =
        py_round(layer.w as f64 * super::constants::BREAK_OVERLAY_BAR_WIDTH_RATIO) as f64;
    let track_height = super::constants::BREAK_OVERLAY_BAR_HEIGHT * render_scale;
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

fn draw_break_arrows(layer: &mut Img, alpha: f64, render_scale: f64) {
    let color = [238, 238, 238, py_round(80.0 * alpha).clamp(0, 255) as u8];
    let glow_color = [238, 238, 238, py_round(35.0 * alpha).clamp(0, 255) as u8];
    let center_y = layer.h as f64 / 2.0;
    for (offset, direction) in [(-0.22, 1.0), (0.22, -1.0)] {
        let center_x = layer.w as f64 / 2.0 + layer.w as f64 * offset;
        draw_chevron(
            layer,
            center_x,
            center_y,
            32.0 * render_scale,
            direction,
            glow_color,
            9.0 * render_scale,
        );
        draw_chevron(
            layer,
            center_x,
            center_y,
            20.0 * render_scale,
            direction,
            color,
            4.0 * render_scale,
        );
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
    if break_period.end_time - break_period.start_time < super::constants::BREAK_MIN_DURATION_MS {
        return 0.0;
    }
    if snapshot_time < break_period.start_time || snapshot_time > break_period.end_time {
        return 0.0;
    }
    if snapshot_time < break_period.start_time + super::constants::BREAK_FADE_DURATION_MS {
        return (snapshot_time - break_period.start_time) as f64
            / super::constants::BREAK_FADE_DURATION_MS as f64;
    }
    if snapshot_time > break_period.end_time - super::constants::BREAK_FADE_DURATION_MS {
        return (break_period.end_time - snapshot_time) as f64
            / super::constants::BREAK_FADE_DURATION_MS as f64;
    }
    1.0
}

fn break_remaining_bar_ratio(break_period: &BreakPeriod, snapshot_time: i64) -> f64 {
    let effective_duration =
        break_period.end_time - super::constants::BREAK_FADE_DURATION_MS - break_period.start_time;
    if effective_duration <= 0 {
        return 0.0;
    }
    let remaining =
        break_period.end_time - super::constants::BREAK_FADE_DURATION_MS - snapshot_time;
    (remaining as f64 / effective_duration as f64).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{HitObjects, KvSection};
    use crate::domain::shared::time_selection::TimeAxis;
    use crate::render::cpu::modes::standard::context::{
        build_render_context, build_video_render_context, RenderContext,
    };
    use crate::render::geometry::OutputFormat;

    /// 单个位于游戏坐标左边缘的圆圈；AR5 的 preempt 为 1200ms。
    fn edge_circle_beatmap() -> crate::domain::models::Beatmap {
        let mut general = KvSection::default();
        general.insert("Mode", "0".to_string());
        let mut difficulty = KvSection::default();
        difficulty.insert("CircleSize", "4".to_string());
        difficulty.insert("ApproachRate", "5".to_string());
        crate::domain::models::Beatmap {
            metadata: KvSection::default(),
            difficulty,
            general,
            timing_points: Vec::new(),
            hit_objects: HitObjects::Standard(vec![crate::domain::models::StandardHitObject {
                x: 0,
                y: 192,
                start_time: 5_000,
                end_time: 5_000,
                hit_type: 1,
                ..Default::default()
            }]),
            break_periods: Vec::new(),
            background_filename: None,
            combo_colors: Vec::new(),
            beat_divisor: 0,
        }
    }

    fn render_single(context: &RenderContext, snapshot_time: i64) -> Img {
        let mut cache = RenderCache::default();
        let indexes = vec![0usize];
        render_frame(context, &mut cache, snapshot_time, &[], &indexes, None)
    }

    /// 判定点附近是否存在不透明像素（接近圈描边）。
    fn has_stroke_pixel(frame: &Img, center: (f64, f64), radius: f64) -> bool {
        for dx in -3..=3i64 {
            for dy in -3..=3i64 {
                let x = (center.0 - radius).round() as i64 + dx;
                let y = center.1.round() as i64 + dy;
                if x < 0 || y < 0 || x >= frame.w as i64 || y >= frame.h as i64 {
                    continue;
                }
                if frame.get(x as u32, y as u32)[3] > 0 {
                    return true;
                }
            }
        }
        false
    }

    #[test]
    fn video_layer_draws_approach_circle_beyond_content_box() {
        let beatmap = edge_circle_beatmap();
        let video = build_video_render_context(
            &beatmap,
            beatmap.hit_objects.as_standard().unwrap().to_vec(),
            None,
            TimeAxis::new(0),
            OutputFormat::Mp4,
        );
        // progress = 0.5：接近圈直径缩到 2.5 倍物件直径，左边缘会越过内容框。
        let snapshot_time = 5_000 - video.settings.preempt_ms / 2;
        let frame = render_single(&video, snapshot_time);

        let radius = video.frame_circle_diameter as f64 * 2.5 / 2.0;
        let center = (
            video.frame_layout.playfield_left,
            video.frame_layout.playfield_top + 192.0 * video.frame_layout.scale,
        );
        assert!(
            center.0 - radius < video.frame_layout.content.x as f64,
            "测试前提：接近圈左边缘必须落在内容框之外"
        );
        assert!(
            has_stroke_pixel(&frame, center, radius),
            "视频物件层里内容框左侧仍应画出接近圈（此前会被内容框裁掉一半）"
        );
        assert_eq!(
            (frame.w as i64, frame.h as i64),
            (
                video.frame_layout.frame_width,
                video.frame_layout.frame_height
            )
        );
    }

    #[test]
    fn video_layer_fills_content_box_only_and_keeps_letterbox_transparent() {
        let beatmap = edge_circle_beatmap();
        let video = build_video_render_context(
            &beatmap,
            beatmap.hit_objects.as_standard().unwrap().to_vec(),
            None,
            TimeAxis::new(0),
            OutputFormat::Mp4,
        );
        // 该时刻没有任何可见物件，因此整帧只有底色。
        let frame = render_single(&video, 0);
        let content = video.frame_layout.content;
        let inside_x = content.x as u32;
        let inside_y = content.y as u32;
        assert_eq!(frame.get(inside_x, inside_y)[3], 255);
        // 左侧补边必须保持透明，让合成阶段的画布底色透出来（外观与修改前一致）。
        assert_eq!(frame.get(inside_x - 1, inside_y)[3], 0);
        assert_eq!(frame.get(frame.w - 1, frame.h - 1)[3], 0);
    }

    #[test]
    fn gif_layout_still_draws_inside_content_box() {
        let beatmap = edge_circle_beatmap();
        let gif = build_render_context(
            &beatmap,
            beatmap.hit_objects.as_standard().unwrap().to_vec(),
            None,
            TimeAxis::new(0),
            OutputFormat::Gif,
        );
        // PNG/GIF 的物件层就是内容框本身：底色覆盖整帧，行为不变。
        assert_eq!(
            gif.frame_layout.content,
            crate::render::geometry::PixelRect {
                x: 0,
                y: 0,
                width: gif.frame_layout.frame_width,
                height: gif.frame_layout.frame_height,
            }
        );
        let frame = render_single(&gif, 0);
        assert_eq!(frame.get(0, 0)[3], 255);
        assert_eq!((frame.w as i64, frame.h as i64), (530, 384));
    }

    /// 两个水平相距 300 个 playfield 单位、相隔 1000ms 的圆圈（AR5）。
    fn two_circle_beatmap() -> crate::domain::models::Beatmap {
        let mut general = KvSection::default();
        general.insert("Mode", "0".to_string());
        let mut difficulty = KvSection::default();
        difficulty.insert("CircleSize", "4".to_string());
        difficulty.insert("ApproachRate", "5".to_string());
        let circle = |x: i32, start_time: i64| crate::domain::models::StandardHitObject {
            x,
            y: 192,
            start_time,
            end_time: start_time,
            hit_type: 1,
            ..Default::default()
        };
        crate::domain::models::Beatmap {
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

    /// 统计 `center` 周围 `radius` 像素内符合 `predicate` 的像素数。
    fn count_pixels(
        frame: &Img,
        center: (f64, f64),
        radius: i64,
        predicate: impl Fn([u8; 4]) -> bool,
    ) -> usize {
        let mut count = 0;
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                let x = center.0.round() as i64 + dx;
                let y = center.1.round() as i64 + dy;
                if x < 0 || y < 0 || x >= frame.w as i64 || y >= frame.h as i64 {
                    continue;
                }
                if predicate(frame.get(x as u32, y as u32)) {
                    count += 1;
                }
            }
        }
        count
    }

    #[test]
    fn follow_points_are_drawn_between_hit_objects() {
        let beatmap = two_circle_beatmap();
        let context = build_render_context(
            &beatmap,
            beatmap.hit_objects.as_standard().unwrap().to_vec(),
            None,
            TimeAxis::new(0),
            OutputFormat::Gif,
        );
        // 第一个跟随点在连接上 48/300 处：1000ms 时已淡入完成并停在
        // 世界坐标 (148, 192)，位于左侧圆圈的判定圈之外。
        let center = to_frame_point(148.0, 192.0, &context.frame_layout);
        let is_pink = |pixel: [u8; 4]| pixel[0] > 100 && pixel[0] > pixel[1] && pixel[2] > pixel[1];
        let is_background = |pixel: [u8; 4]| pixel == [0, 0, 0, 255];

        let frame = render_single(&context, 1000);
        assert!(
            count_pixels(&frame, center, 4, is_pink) > 0,
            "两个圆圈之间应画出 Argon 跟随点"
        );
        // 淡入之前（fade_in = 360ms）同一位置不应有任何跟随点。
        let before = render_single(&context, 300);
        assert_eq!(count_pixels(&before, center, 4, is_background), 81);
    }
}
