//! osu!catch PNG 静态图渲染器：纵向多列时间轴谱面图。
//!
//! 每列自上而下表示时间推进，水果按 playfield x 坐标横向分布。
//! 谱面总高度有上限（防止超长 / 高 AR 谱面导致内存爆炸），超出时
//! 按比例压缩纵向密度。

use crate::canvas::Img;
use crate::composer::save_png;
use crate::errors::{PreviewError, Result};
use crate::models::{Beatmap, HitObjects, TimingPoint};
use crate::mods::ModSettings;
use crate::parser::round_half_even;
use std::path::{Path, PathBuf};

use super::constants::*;
use super::drawing::draw_catch_object;
use super::objects::{
    build_catch_render_objects, effective_difficulty, object_order, RenderObject,
};

#[inline]
pub(crate) fn rhe(v: f64) -> i64 {
    round_half_even(v)
}

// ─── 布局 ───

struct RenderLayout {
    column_count: i64,
    total_column_height: i64,
    visible_playfield_width: i64,
    image_width: i64,
    image_height: i64,
    playfield_scale: f64,
    object_scale: f64,
    pixels_per_ms: f64,
}

/// AR 决定的纵向密度：AR 时间窗内的下落距离映射为像素。
fn pixels_per_ms_for_ar(approach_rate: f64, playfield_scale: f64) -> f64 {
    let time_range = super::objects::catch_time_range(approach_rate);
    let visible_fall_height = (STABLE_CATCHER_Y - STABLE_FRUIT_START_Y) * playfield_scale;
    visible_fall_height / time_range
}

fn resolve_max_area_height(beatmap_duration: i64) -> i64 {
    if beatmap_duration < 60_000 { MAX_AREA_HEIGHT_0_TO_1_MIN }
    else if beatmap_duration < 2 * 60_000 { MAX_AREA_HEIGHT_1_TO_2_MIN }
    else if beatmap_duration < 3 * 60_000 { MAX_AREA_HEIGHT_2_TO_3_MIN }
    else if beatmap_duration < 4 * 60_000 { MAX_AREA_HEIGHT_3_TO_4_MIN }
    else if beatmap_duration < 5 * 60_000 { MAX_AREA_HEIGHT_4_TO_5_MIN }
    else { MAX_AREA_HEIGHT_5_TO_6_MIN }
}

fn ceil_div(a: i64, b: i64) -> i64 {
    (a + b - 1) / b
}

fn build_layout(beatmap_duration: i64, circle_size: f64, approach_rate: f64) -> Result<RenderLayout> {
    if beatmap_duration >= MAX_SUPPORTED_DURATION_MS {
        return Err(PreviewError::new(
            "songs longer than 10 minutes are not supported",
        ));
    }
    let playfield_scale = (COLUMN_WIDTH - LEFT_PANEL_WIDTH) as f64 / PLAYFIELD_WIDTH;
    let object_scale = super::objects::circle_scale(circle_size);

    // 纵向密度上限：限制谱面总像素高度，防止高 AR + 长曲导致内存爆炸
    let mut pixels_per_ms = pixels_per_ms_for_ar(approach_rate, playfield_scale);
    let natural_height = beatmap_duration as f64 * pixels_per_ms;
    if natural_height > MAX_TOTAL_CHART_HEIGHT as f64 {
        pixels_per_ms *= MAX_TOTAL_CHART_HEIGHT as f64 / natural_height;
    }

    let total_chart_height = rhe(beatmap_duration as f64 * pixels_per_ms).max(1);
    let max_area_height = resolve_max_area_height(beatmap_duration);
    let column_count = ceil_div(total_chart_height, max_area_height).max(1);
    let total_column_height = ceil_div(total_chart_height, column_count);
    let image_width = PAGE_MARGIN_X * 2
        + column_count * (COLUMN_WIDTH + COLUMN_GAP)
        - COLUMN_GAP;
    let image_height = PAGE_MARGIN_Y * 2 + total_column_height;
    Ok(RenderLayout {
        column_count,
        total_column_height,
        visible_playfield_width: COLUMN_WIDTH - LEFT_PANEL_WIDTH,
        image_width,
        image_height,
        playfield_scale,
        object_scale,
        pixels_per_ms,
    })
}

fn column_left(column_index: i64) -> i64 {
    PAGE_MARGIN_X + column_index * (COLUMN_WIDTH + COLUMN_GAP)
}

fn playfield_left(column_index: i64) -> i64 {
    column_left(column_index) + LEFT_PANEL_WIDTH
}

// ─── 节拍线 ───

struct TimingLine {
    time: i64,
    is_measure: bool,
}

/// 红线分段：每段持有固定的 beat_length 与 meter。
struct RedlineSection {
    start_time: f64,
    end_time: f64,
    beat_length: f64,
    meter: i32,
}

/// 从红线（uninherited timing point）构建分段，再按段内节拍生成节拍线。
/// 节拍从红线时间起步，避免旧实现「从 0 起步 + 红线重置」的死循环问题。
fn build_timing_lines(timing_points: &[TimingPoint], chart_end_time: i64) -> Vec<TimingLine> {
    let red_lines: Vec<&TimingPoint> = timing_points
        .iter()
        .filter(|p| p.uninherited && p.beat_length.is_finite() && p.beat_length > 0.0)
        .collect();
    if red_lines.is_empty() {
        return Vec::new();
    }

    // 切分红线区段（首段从 0 或首条红线之前开始，沿用首条红线参数）
    let mut sections: Vec<RedlineSection> = Vec::new();
    for (index, point) in red_lines.iter().enumerate() {
        let start = if index == 0 { point.time.min(0.0) } else { point.time };
        let end = if index + 1 < red_lines.len() {
            red_lines[index + 1].time
        } else {
            chart_end_time as f64
        };
        if end <= start {
            continue;
        }
        sections.push(RedlineSection {
            start_time: if index == 0 { point.time } else { start },
            end_time: end,
            beat_length: point.beat_length.max(1.0),
            meter: point.meter.max(1),
        });
    }

    let mut lines: Vec<TimingLine> = Vec::new();
    for section in &sections {
        let mut beat_index: i64 = 0;
        loop {
            let time = section.start_time + beat_index as f64 * section.beat_length;
            if time > section.end_time + 0.001 || time > chart_end_time as f64 {
                break;
            }
            if time >= 0.0 {
                lines.push(TimingLine {
                    time: rhe(time),
                    is_measure: beat_index % section.meter as i64 == 0,
                });
            }
            beat_index += 1;
        }
    }
    lines
}

// ─── 对外接口 ───

pub(crate) fn render_catch_grid(
    beatmap: &Beatmap, output_path: &Path, mods: Option<&ModSettings>,
) -> Result<PathBuf> {
    let hit_objects = match &beatmap.hit_objects {
        HitObjects::Catch(v) if !v.is_empty() => v,
        _ => return Err(PreviewError::new("catch beatmap has no hit objects")),
    };

    let difficulty = effective_difficulty(beatmap, mods);
    let render_objects = build_catch_render_objects(beatmap, hit_objects, mods, &difficulty)?;
    let chart_end_time = hit_objects.iter().map(|h| h.end_time).max().unwrap().max(1);
    let timing_lines = build_timing_lines(&beatmap.timing_points, chart_end_time);
    let layout = build_layout(chart_end_time, difficulty.cs, difficulty.ar)?;

    let mut image = Img::new(layout.image_width as u32, layout.image_height as u32, IMAGE_BACKGROUND);

    for column_index in 0..layout.column_count {
        draw_column_background(&mut image, &layout, column_index);
    }
    // 接手示意只画在第一列的判定位置
    draw_catcher(&mut image, &layout);

    for timing_line in &timing_lines {
        draw_timing_line_png(&mut image, timing_line, &layout);
    }

    // 后发生的对象先画（早出现的盖在上层），同时刻按 类型 排序
    let mut sorted_objects: Vec<&RenderObject> = render_objects.iter().collect();
    sorted_objects.sort_by_key(|o| (-o.start_time, object_order(o.object_type)));
    for catch_object in sorted_objects {
        draw_catch_object_png(&mut image, catch_object, &layout);
    }

    save_png(&image, output_path)?;
    Ok(output_path.to_path_buf())
}

/// 画单列背景：左侧灰条 + playfield 底色 + 左右边界线。
fn draw_column_background(image: &mut Img, layout: &RenderLayout, column_index: i64) {
    let column_left = column_left(column_index);
    let chart_top = PAGE_MARGIN_Y;
    let chart_bottom = PAGE_MARGIN_Y + layout.total_column_height;
    let visible_left = column_left + LEFT_PANEL_WIDTH;
    let visible_right = visible_left + layout.visible_playfield_width;

    image.set_rect(column_left, chart_top, visible_left, chart_bottom, LEFT_PANEL_BACKGROUND);
    image.set_rect(visible_left, chart_top, visible_right, chart_bottom, PLAYFIELD_BACKGROUND);
    image.set_rect(visible_left, chart_top, visible_left, chart_bottom, PLAYFIELD_BORDER);
    image.set_rect(visible_right, chart_top, visible_right, chart_bottom, PLAYFIELD_BORDER);
}

/// 时间 → （列号, 列内 y 坐标）。
fn locate_time(time: i64, layout: &RenderLayout) -> (i64, i64) {
    let absolute_y = time as f64 * layout.pixels_per_ms;
    let column_index = ((absolute_y / layout.total_column_height as f64).floor() as i64)
        .clamp(0, layout.column_count - 1);
    let local_y = rhe(absolute_y - (column_index * layout.total_column_height) as f64);
    (column_index, local_y)
}

fn draw_timing_line_png(image: &mut Img, timing_line: &TimingLine, layout: &RenderLayout) {
    let (column_index, local_y) = locate_time(timing_line.time, layout);
    let left = playfield_left(column_index);
    let right = left + layout.visible_playfield_width;
    let y = (PAGE_MARGIN_Y + local_y).clamp(PAGE_MARGIN_Y, PAGE_MARGIN_Y + layout.total_column_height);

    if timing_line.is_measure {
        image.set_rect(left, y, right, y + 1, MEASURE_LINE);
    } else {
        image.set_rect(left, y, right, y, BEAT_LINE);
    }
}

fn draw_catch_object_png(image: &mut Img, catch_object: &RenderObject, layout: &RenderLayout) {
    let (column_index, local_y) = locate_time(catch_object.start_time, layout);
    let center_x = playfield_left(column_index) as f64
        + catch_object.x * layout.playfield_scale;
    let center_y = (PAGE_MARGIN_Y + local_y) as f64;
    let diameter = super::drawing::object_diameter(
        layout.object_scale, layout.playfield_scale, catch_object.scale_factor,
    );

    draw_catch_object(image, catch_object, center_x, center_y, diameter);
}

/// 在第一列顶部画接手示意（表示判定线相对宽度）。
fn draw_catcher(image: &mut Img, layout: &RenderLayout) {
    let catcher_x = playfield_left(0) as f64 + PLAYFIELD_WIDTH * layout.playfield_scale / 2.0;
    let catcher_y = PAGE_MARGIN_Y as f64;
    let catcher_scale = layout.object_scale * 2.0 * layout.playfield_scale;
    let catcher_width =
        (CATCHER_SPRITE_LOGICAL_WIDTH * LEGACY_CATCHER_VISUAL_SCALE * catcher_scale).max(1.0);
    super::drawing::draw_argon_catcher(image, catcher_x, catcher_y, catcher_width, catcher_scale);
}
