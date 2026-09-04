//! osu!catch PNG 静态图渲染器：纵向多列时间轴谱面图。
//!
//! 每列自上而下表示时间推进，水果按 playfield x 坐标横向分布。
//! 谱面总高度有上限（防止超长 / 高 AR 谱面导致内存爆炸），超出时
//! 按比例压缩纵向密度。

use crate::common::time_selection::TimeAxis;
use crate::core::errors::{PreviewError, Result};
use crate::core::models::{Beatmap, TimingPoint};
use crate::core::mods::ModSettings;
use crate::core::timeout::RequestDeadline;
use crate::parser::round_half_even;
use crate::render::canvas::Img;
use crate::render::composer::save_png;
use crate::render::text::{draw_text, text_size};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::drawing::draw_catch_object;
use super::objects::{
    build_catch_render_objects, effective_difficulty, object_order, ObjType, RenderObject,
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
    chart_start_time: i64,
}

/// AR 决定的纵向密度：AR 时间窗内的下落距离映射为像素。
fn pixels_per_ms_for_ar(approach_rate: f64, playfield_scale: f64) -> f64 {
    let time_range = super::objects::catch_time_range(approach_rate);
    let visible_fall_height = (crate::render::catch::constants::STABLE_CATCHER_Y
        - crate::render::catch::constants::STABLE_FRUIT_START_Y)
        * playfield_scale;
    visible_fall_height / time_range
}

fn resolve_max_area_height(beatmap_duration: i64) -> i64 {
    if beatmap_duration < 60_000 {
        crate::config::current()
            .layout
            .catch
            .png
            .MAX_AREA_HEIGHT_0_TO_1_MINUTES
    } else if beatmap_duration < 2 * 60_000 {
        crate::config::current()
            .layout
            .catch
            .png
            .MAX_AREA_HEIGHT_1_TO_2_MINUTES
    } else if beatmap_duration < 3 * 60_000 {
        crate::config::current()
            .layout
            .catch
            .png
            .MAX_AREA_HEIGHT_2_TO_3_MINUTES
    } else if beatmap_duration < 4 * 60_000 {
        crate::config::current()
            .layout
            .catch
            .png
            .MAX_AREA_HEIGHT_3_TO_4_MINUTES
    } else if beatmap_duration < 5 * 60_000 {
        crate::config::current()
            .layout
            .catch
            .png
            .MAX_AREA_HEIGHT_4_TO_5_MINUTES
    } else {
        crate::config::current()
            .layout
            .catch
            .png
            .MAX_AREA_HEIGHT_5_TO_6_MINUTES
    }
}

fn ceil_div(a: i64, b: i64) -> i64 {
    (a + b - 1) / b
}

fn build_layout(
    beatmap_duration: i64,
    circle_size: f64,
    approach_rate: f64,
    chart_start_time: i64,
    timing_lines: &[TimingLine],
) -> Result<RenderLayout> {
    if beatmap_duration
        >= crate::config::current()
            .layout
            .catch
            .png
            .MAX_SUPPORTED_DURATION_MS
    {
        return Err(PreviewError::render(
            "songs longer than 10 minutes are not supported",
        ));
    }
    let render_scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Catch,
        crate::render::geometry::OutputFormat::Png,
    );
    let visible_playfield_width = crate::render::geometry::scale_px(
        crate::render::catch::constants::PLAYFIELD_DISPLAY_WIDTH as f64,
        render_scale,
    );
    let playfield_scale =
        visible_playfield_width as f64 / crate::render::catch::constants::PLAYFIELD_WIDTH;
    let object_scale = super::objects::circle_scale(circle_size);

    // 纵向密度上限：限制谱面总像素高度，防止高 AR + 长曲导致内存爆炸
    let mut pixels_per_ms = pixels_per_ms_for_ar(approach_rate, playfield_scale);
    let natural_height = beatmap_duration as f64 * pixels_per_ms;
    if natural_height
        > crate::config::current()
            .layout
            .catch
            .png
            .MAX_TOTAL_CHART_HEIGHT as f64
    {
        pixels_per_ms *= crate::config::current()
            .layout
            .catch
            .png
            .MAX_TOTAL_CHART_HEIGHT as f64
            / natural_height;
    }

    let total_chart_height = rhe(beatmap_duration as f64 * pixels_per_ms).max(1);
    let max_area_height = resolve_max_area_height(beatmap_duration);
    let aligned_height =
        predominant_measure_aligned_height(timing_lines, pixels_per_ms, max_area_height)
            .unwrap_or(max_area_height);
    let total_column_height = total_chart_height.min(aligned_height).max(1);
    let column_count = ceil_div(total_chart_height, total_column_height).max(1);
    let config = &crate::config::current().layout.catch.png;
    let unit_width = config.INFO_MARGIN_LEFT + config.COLUMN_WIDTH + config.INFO_MARGIN_RIGHT;
    let image_width = config.PAGE_MARGIN_LEFT
        + config.PAGE_MARGIN_RIGHT
        + column_count * unit_width
        + (column_count - 1) * config.COLUMN_GAP;
    let image_height = config.PAGE_MARGIN_TOP
        + config.PAGE_MARGIN_BOTTOM
        + config.INFO_MARGIN_TOP
        + total_column_height
        + config.INFO_MARGIN_BOTTOM;
    Ok(RenderLayout {
        column_count,
        total_column_height,
        visible_playfield_width,
        image_width,
        image_height,
        playfield_scale,
        object_scale,
        pixels_per_ms,
        chart_start_time,
    })
}

/// 让每列高度尽量成为主要小节间隔的整数倍，使各列的主小节线纵向对齐。
fn predominant_measure_aligned_height(
    timing_lines: &[TimingLine],
    pixels_per_ms: f64,
    max_area_height: i64,
) -> Option<i64> {
    let measures: Vec<i64> = timing_lines
        .iter()
        .filter(|line| line.is_measure)
        .map(|line| line.time)
        .collect();
    let mut frequencies: BTreeMap<i64, usize> = BTreeMap::new();
    for pair in measures.windows(2) {
        let delta = pair[1] - pair[0];
        if delta > 100 {
            *frequencies.entry(delta).or_default() += 1;
        }
    }
    let dominant_delta = frequencies
        .into_iter()
        .max_by_key(|(delta, count)| (*count, std::cmp::Reverse(*delta)))?
        .0;
    let interval_height = rhe(dominant_delta as f64 * pixels_per_ms).max(1);
    let interval_count = max_area_height / interval_height;
    (interval_count > 3).then_some(interval_count * interval_height)
}

fn column_left(column_index: i64) -> i64 {
    let config = &crate::config::current().layout.catch.png;
    config.PAGE_MARGIN_LEFT
        + config.INFO_MARGIN_LEFT
        + column_index
            * (config.INFO_MARGIN_LEFT
                + config.COLUMN_WIDTH
                + config.INFO_MARGIN_RIGHT
                + config.COLUMN_GAP)
}

fn playfield_left(column_index: i64) -> i64 {
    column_left(column_index) + crate::config::current().layout.catch.png.LEFT_PANEL_WIDTH + 23
}

// ─── 节拍线 ───

#[derive(Clone, Copy)]
struct TimingLine {
    time: i64,
    is_measure: bool,
    show_label: bool,
    bpm: Option<f64>,
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
        let start = if index == 0 {
            point.time.min(0.0)
        } else {
            point.time
        };
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
    let mut last_bpm: Option<f64> = None;
    for section in &sections {
        let bpm = 60_000.0 / section.beat_length;
        let show_bpm = last_bpm.is_none_or(|last| (last - bpm).abs() > 0.01);
        last_bpm = Some(bpm);
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
                    show_label: true,
                    bpm: (show_bpm && beat_index == 0).then_some(bpm),
                });
            }
            beat_index += 1;
        }
    }
    if let Some(first_visible) = lines.iter_mut().find(|line| line.show_label) {
        if first_visible.bpm.is_none() {
            first_visible.bpm = crate::render::timing::bpm_at(timing_points, first_visible.time);
        }
    }
    lines
}

// ─── 对外接口 ───

pub(crate) fn render_catch_grid(
    beatmap: &Beatmap,
    output_path: &Path,
    mods: Option<&ModSettings>,
    time_axis: TimeAxis,
    deadline: &RequestDeadline,
) -> Result<PathBuf> {
    deadline.check()?;
    let hit_objects = match beatmap.hit_objects.as_catch() {
        Some(v) if !v.is_empty() => v,
        _ => return Err(PreviewError::render("catch beatmap has no hit objects")),
    };

    let difficulty = effective_difficulty(beatmap, mods);
    let mut render_objects = build_catch_render_objects(beatmap, hit_objects, mods, &difficulty)?;
    deadline.check()?;
    let chart_end_time = hit_objects.iter().map(|h| h.end_time).max().unwrap().max(1);

    // 裁剪开头静音：若第一个音符在 5 秒之后，则从其前 1 秒开始，
    // 并对齐到红线节拍网格。
    let first_note_time = hit_objects.iter().map(|h| h.start_time).min().unwrap_or(0);
    let chart_start_time = if first_note_time >= 5000 {
        crate::common::time_selection::snap_to_beat_grid(
            first_note_time - 1000,
            &beatmap.timing_points,
        )
    } else {
        0
    };

    let (effective_chart_end_time, timing_points_for_render): (i64, Vec<TimingPoint>) =
        if chart_start_time > 0 {
            for ro in &mut render_objects {
                ro.start_time = (ro.start_time - chart_start_time).max(0);
                if let Some(ref mut et) = ro.event_time {
                    *et = (*et - chart_start_time as f64).max(0.0);
                }
            }
            let tp = beatmap
                .timing_points
                .iter()
                .map(|tp| {
                    let mut tp = *tp;
                    tp.time -= chart_start_time as f64;
                    tp
                })
                .collect();
            ((chart_end_time - chart_start_time).max(0), tp)
        } else {
            (chart_end_time, beatmap.timing_points.clone())
        };

    let timing_lines = build_timing_lines(&timing_points_for_render, effective_chart_end_time);
    let layout = build_layout(
        effective_chart_end_time,
        difficulty.cs,
        difficulty.ar,
        chart_start_time,
        &timing_lines,
    )?;

    let mut image = Img::new(
        layout.image_width as u32,
        layout.image_height as u32,
        crate::config::current().layout.catch.png.IMAGE_BACKGROUND,
    );

    for column_index in 0..layout.column_count {
        deadline.check()?;
        draw_column_background(&mut image, &layout, column_index);
    }

    let mut last_label_time: Option<i64> = None;
    for timing_line in &timing_lines {
        deadline.check()?;
        let mut tl = *timing_line;
        if tl.show_label {
            if let Some(prev) = last_label_time {
                if (tl.time - prev).abs()
                    < crate::config::current()
                        .layout
                        .catch
                        .png
                        .TIME_LABEL_MIN_INTERVAL_MS
                {
                    tl.show_label = false;
                }
            }
            if tl.show_label {
                last_label_time = Some(tl.time);
            }
        }
        draw_timing_line_png(&mut image, &tl, &layout);
        if tl.show_label || tl.bpm.is_some() {
            draw_timing_label_png(&mut image, &tl, &layout, time_axis);
        }
    }

    // 引导线放在物件下层，避免遮住水果图形。
    draw_edge_guides(&mut image, &render_objects, &layout);

    // 后发生的对象先画（早出现的盖在上层），同时刻按 类型 排序
    let mut sorted_objects: Vec<&RenderObject> = render_objects.iter().collect();
    sorted_objects.sort_by_key(|o| (-o.start_time, object_order(o.object_type)));
    for (index, catch_object) in sorted_objects.into_iter().enumerate() {
        if index % 1024 == 0 {
            deadline.check()?;
        }
        draw_catch_object_png(&mut image, catch_object, &layout);
    }

    // combo 标签放在引导线和物件上层，确保密集段落中仍然清晰可读。
    draw_edge_combo_labels(&mut image, &render_objects, &layout);

    save_png(&image, output_path, deadline)?;
    Ok(output_path.to_path_buf())
}

/// 画单列背景：左侧灰条 + playfield 底色 + 左右边界线（与 playfield 区域留 23px）。
fn draw_column_background(image: &mut Img, layout: &RenderLayout, column_index: i64) {
    let column_left = column_left(column_index);
    let chart_top = crate::config::current().layout.catch.png.PAGE_MARGIN_TOP
        + crate::config::current().layout.catch.png.INFO_MARGIN_TOP;
    let chart_bottom = chart_top + layout.total_column_height;
    // 左侧灰条在最左边
    let panel_right = column_left + crate::config::current().layout.catch.png.LEFT_PANEL_WIDTH;
    // playfield 在灰条右侧 23px 处开始
    let visible_left = panel_right + 23;
    let visible_right = visible_left + layout.visible_playfield_width;
    let border_left = visible_left - 23;
    let border_right = visible_right + 23;

    image.set_rect(
        column_left,
        chart_top,
        panel_right,
        chart_bottom,
        crate::config::current()
            .layout
            .catch
            .png
            .LEFT_PANEL_BACKGROUND,
    );
    image.set_rect(
        visible_left,
        chart_top,
        visible_right,
        chart_bottom,
        crate::config::current()
            .layout
            .catch
            .png
            .PLAYFIELD_BACKGROUND,
    );
    image.set_rect(
        border_left,
        chart_top,
        border_left,
        chart_bottom,
        crate::config::current().layout.catch.png.PLAYFIELD_BORDER,
    );
    image.set_rect(
        border_right,
        chart_top,
        border_right,
        chart_bottom,
        crate::config::current().layout.catch.png.PLAYFIELD_BORDER,
    );
}

/// 时间 → （列号, y 坐标）。时间从列底部向上递增（与游戏内下落方向一致）。
fn locate_time(time: i64, layout: &RenderLayout) -> (i64, i64) {
    let absolute_y = time as f64 * layout.pixels_per_ms;
    let column_index = ((absolute_y / layout.total_column_height as f64).floor() as i64)
        .clamp(0, layout.column_count - 1);
    let local_y_from_top = rhe(absolute_y - (column_index * layout.total_column_height) as f64);
    // 从列底部开始计算，时间 0 在底部，时间增大向上
    let chart_bottom = crate::config::current().layout.catch.png.PAGE_MARGIN_TOP
        + crate::config::current().layout.catch.png.INFO_MARGIN_TOP
        + layout.total_column_height;
    let y = chart_bottom - local_y_from_top;
    (column_index, y)
}

fn draw_timing_line_png(image: &mut Img, timing_line: &TimingLine, layout: &RenderLayout) {
    let (column_index, y) = locate_time(timing_line.time, layout);
    let left = playfield_left(column_index);
    let right = left + layout.visible_playfield_width;
    let y = y.clamp(
        crate::config::current().layout.catch.png.PAGE_MARGIN_TOP
            + crate::config::current().layout.catch.png.INFO_MARGIN_TOP,
        crate::config::current().layout.catch.png.PAGE_MARGIN_TOP
            + crate::config::current().layout.catch.png.INFO_MARGIN_TOP
            + layout.total_column_height,
    );

    if timing_line.is_measure {
        image.set_rect(
            left,
            y,
            right,
            y + 1,
            crate::config::current().layout.catch.png.MEASURE_LINE_COLOR,
        );
    } else {
        image.set_rect(
            left,
            y,
            right,
            y,
            crate::config::current().layout.catch.png.BEAT_LINE_COLOR,
        );
    }
}

fn draw_timing_label_png(
    image: &mut Img,
    timing_line: &TimingLine,
    layout: &RenderLayout,
    time_axis: TimeAxis,
) {
    let (column_index, y) = locate_time(timing_line.time, layout);
    let border_right =
        column_left(column_index) + crate::config::current().layout.catch.png.COLUMN_WIDTH;
    let y = y.clamp(
        crate::config::current().layout.catch.png.PAGE_MARGIN_TOP
            + crate::config::current().layout.catch.png.INFO_MARGIN_TOP,
        crate::config::current().layout.catch.png.PAGE_MARGIN_TOP
            + crate::config::current().layout.catch.png.INFO_MARGIN_TOP
            + layout.total_column_height,
    );
    let label = crate::render::text::format_seconds_tenths(
        time_axis.to_display(timing_line.time + layout.chart_start_time),
    );
    let (label_width, label_height) = text_size(
        &label,
        crate::config::current()
            .layout
            .catch
            .png
            .TIME_LABEL_FONT_SIZE,
    );
    let label_gap = crate::render::geometry::scale_px(
        4.0,
        crate::render::geometry::output_scale(
            crate::render::geometry::GameMode::Catch,
            crate::render::geometry::OutputFormat::Png,
        ),
    );
    let label_x = (border_right + label_gap).min(
        layout.image_width
            - label_width as i64
            - crate::config::current().layout.catch.png.PAGE_MARGIN_LEFT,
    );
    let label_y = (y as f64 - label_height as f64 / 2.0).floor() as i64;
    let bpm_label = timing_line.bpm.map(crate::render::timing::format_bpm);
    let bpm_height = bpm_label.as_ref().map_or(0, |text| {
        text_size(
            text,
            crate::config::current()
                .layout
                .catch
                .png
                .TIME_LABEL_FONT_SIZE,
        )
        .1 as i64
            + crate::config::current().layout.catch.png.BPM_LABEL_GAP
    });
    let group_height = label_height as i64 + bpm_height;
    let chart_top = crate::config::current().layout.catch.png.PAGE_MARGIN_TOP
        + crate::config::current().layout.catch.png.INFO_MARGIN_TOP;
    let chart_bottom = chart_top + layout.total_column_height;
    let label_y = (label_y - bpm_height / 2)
        .max(chart_top)
        .min(chart_bottom - group_height);
    draw_text(
        image,
        label_x,
        label_y,
        &label,
        crate::config::current()
            .layout
            .catch
            .png
            .TIME_LABEL_FONT_SIZE,
        crate::config::current().layout.catch.png.TIME_LABEL_COLOR,
    );
    if let Some(bpm_label) = bpm_label {
        let (bpm_width, _) = text_size(
            &bpm_label,
            crate::config::current()
                .layout
                .catch
                .png
                .TIME_LABEL_FONT_SIZE,
        );
        let bpm_x = (border_right + label_gap).min(
            layout.image_width
                - bpm_width as i64
                - crate::config::current().layout.catch.png.PAGE_MARGIN_LEFT,
        );
        draw_text(
            image,
            bpm_x,
            label_y + label_height as i64 + crate::config::current().layout.catch.png.BPM_LABEL_GAP,
            &bpm_label,
            crate::config::current()
                .layout
                .catch
                .png
                .TIME_LABEL_FONT_SIZE,
            crate::config::current().layout.catch.png.BPM_LABEL_COLOR,
        );
    }
}

fn draw_catch_object_png(image: &mut Img, catch_object: &RenderObject, layout: &RenderLayout) {
    let (column_index, y) = locate_time(catch_object.start_time, layout);
    let center_x = playfield_left(column_index) as f64 + catch_object.x * layout.playfield_scale;
    let center_y = y as f64;
    let diameter = super::drawing::object_diameter(
        layout.object_scale,
        layout.playfield_scale,
        catch_object.scale_factor,
    );

    draw_catch_object(image, catch_object, center_x, center_y, diameter);
}

type LineSegment = ((f64, f64), (f64, f64));

/// 在列边界处分割引导线，使时间轴从前一列顶部延续到后一列底部，
/// 而不是让线条斜穿整张图片。
fn edge_guide_segments(
    current: &RenderObject,
    next: &RenderObject,
    layout: &RenderLayout,
) -> Vec<LineSegment> {
    let start_time = current.event_time_or_start();
    let end_time = next.event_time_or_start();
    if end_time <= start_time || layout.pixels_per_ms <= 0.0 {
        return Vec::new();
    }

    let column_duration = layout.total_column_height as f64 / layout.pixels_per_ms;
    let start_column = (start_time / column_duration).floor() as i64;
    let end_column = (end_time / column_duration).floor() as i64;
    let chart_bottom = (crate::config::current().layout.catch.png.PAGE_MARGIN_TOP
        + crate::config::current().layout.catch.png.INFO_MARGIN_TOP
        + layout.total_column_height) as f64;
    let mut segments = Vec::new();

    for column in start_column..=end_column {
        if !(0..layout.column_count).contains(&column) {
            continue;
        }
        let column_start = column as f64 * column_duration;
        let column_end = (column + 1) as f64 * column_duration;
        let segment_start = start_time.max(column_start);
        let segment_end = end_time.min(column_end);
        if segment_end <= segment_start {
            continue;
        }

        let point_at = |time: f64| {
            let progress = (time - start_time) / (end_time - start_time);
            let object_x = current.x + (next.x - current.x) * progress;
            let x = playfield_left(column) as f64 + object_x * layout.playfield_scale;
            let local_height = (time - column_start) * layout.pixels_per_ms;
            let y = chart_bottom - local_height;
            (x, y)
        };
        segments.push((point_at(segment_start), point_at(segment_end)));
    }

    segments
}

fn draw_edge_guides(image: &mut Img, render_objects: &[RenderObject], layout: &RenderLayout) {
    for (index, current) in render_objects.iter().enumerate() {
        if !current.edge {
            continue;
        }
        let Some(next) = render_objects[index + 1..].iter().find(|candidate| {
            !matches!(
                candidate.object_type,
                ObjType::Banana | ObjType::TinyDroplet
            )
        }) else {
            continue;
        };

        for (start, end) in edge_guide_segments(current, next, layout) {
            image.draw_line(
                start.0,
                start.1,
                end.0,
                end.1,
                crate::config::current().layout.catch.png.EDGE_GUIDE_WIDTH,
                crate::config::current().layout.catch.png.EDGE_GUIDE_COLOR,
            );
        }
    }
}

/// 返回接到每个 edge 物件时的全连击数；小水滴和香蕉不增加 Catch combo。
fn edge_combo_numbers(render_objects: &[RenderObject]) -> Vec<(usize, usize)> {
    let mut combo = 0;

    render_objects
        .iter()
        .enumerate()
        .filter_map(|(index, object)| {
            if matches!(object.object_type, ObjType::Fruit | ObjType::Droplet) {
                combo += 1;
            }
            object.edge.then_some((index, combo))
        })
        .collect()
}

fn draw_edge_combo_labels(image: &mut Img, render_objects: &[RenderObject], layout: &RenderLayout) {
    for (index, combo) in edge_combo_numbers(render_objects) {
        let current = &render_objects[index];
        let Some(next) = render_objects[index + 1..].iter().find(|candidate| {
            !matches!(
                candidate.object_type,
                ObjType::Banana | ObjType::TinyDroplet
            )
        }) else {
            continue;
        };

        let config = &crate::config::current().layout.catch.png;
        let label = format!("{combo}x");
        let (label_width, label_height) = text_size(&label, config.EDGE_COMBO_LABEL_FONT_SIZE);
        let (column_index, center_y) = locate_time(current.start_time, layout);
        let center_x = playfield_left(column_index) as f64 + current.x * layout.playfield_scale;
        let radius = super::drawing::object_diameter(
            layout.object_scale,
            layout.playfield_scale,
            current.scale_factor,
        ) / 2.0;
        let left_x = rhe(center_x - radius - config.EDGE_COMBO_LABEL_GAP - label_width as f64);
        let right_x = rhe(center_x + radius + config.EDGE_COMBO_LABEL_GAP);
        let min_x = playfield_left(column_index);
        let max_x = min_x + layout.visible_playfield_width - label_width as i64;
        let prefer_left = next.x >= current.x;
        let label_x = if prefer_left && left_x >= min_x {
            left_x
        } else if !prefer_left && right_x <= max_x {
            right_x
        } else if right_x <= max_x {
            right_x
        } else {
            left_x.max(min_x)
        };
        let chart_top = config.PAGE_MARGIN_TOP + config.INFO_MARGIN_TOP;
        let chart_bottom = chart_top + layout.total_column_height;
        let label_y = (center_y - label_height as i64 / 2)
            .max(chart_top)
            .min(chart_bottom - label_height as i64);

        // 深色底板可避免边界迫使标签与白色引导线同侧时难以辨认。
        image.fill_rect(
            label_x - config.EDGE_COMBO_LABEL_PADDING,
            label_y - config.EDGE_COMBO_LABEL_PADDING,
            label_x + label_width as i64 - 1 + config.EDGE_COMBO_LABEL_PADDING,
            label_y + label_height as i64 - 1 + config.EDGE_COMBO_LABEL_PADDING,
            config.EDGE_COMBO_LABEL_BACKGROUND,
        );
        draw_text(
            image,
            label_x + config.EDGE_COMBO_LABEL_SHADOW_GAP,
            label_y + config.EDGE_COMBO_LABEL_SHADOW_GAP,
            &label,
            config.EDGE_COMBO_LABEL_FONT_SIZE,
            config.EDGE_COMBO_LABEL_SHADOW,
        );
        draw_text(
            image,
            label_x,
            label_y,
            &label,
            config.EDGE_COMBO_LABEL_FONT_SIZE,
            config.EDGE_COMBO_LABEL_COLOR,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_layout(column_count: i64) -> RenderLayout {
        RenderLayout {
            column_count,
            total_column_height: 100,
            visible_playfield_width: 260,
            image_width: 1_000,
            image_height: 130,
            playfield_scale: 0.5,
            object_scale: 0.5,
            pixels_per_ms: 1.0,
            chart_start_time: 0,
        }
    }

    fn edge_fruit(x: f64, time: i64) -> RenderObject {
        RenderObject {
            object_type: ObjType::Fruit,
            x,
            start_time: time,
            color: crate::render::catch::constants::LAZER_COMBO_COLORS[0],
            scale_factor: 1.0,
            event_time: Some(time as f64),
            hyper_dash: false,
            edge: true,
        }
    }

    #[test]
    fn edge_guide_is_split_at_column_boundary() {
        let current = edge_fruit(0.0, 90);
        let next = edge_fruit(200.0, 110);

        let segments = edge_guide_segments(&current, &next, &test_layout(2));

        assert_eq!(segments.len(), 2);
        let chart_top = (crate::config::current().layout.catch.png.PAGE_MARGIN_TOP
            + crate::config::current().layout.catch.png.INFO_MARGIN_TOP)
            as f64;
        let chart_bottom = chart_top + 100.0;
        let ((_, first_start_y), (first_end_x, first_end_y)) = segments[0];
        let ((second_start_x, second_start_y), (_, second_end_y)) = segments[1];
        assert_eq!(first_start_y, chart_bottom - 90.0);
        assert_eq!(first_end_y, chart_top);
        assert_eq!(second_start_y, chart_bottom);
        assert_eq!(second_end_y, chart_bottom - 10.0);
        assert!(second_start_x > first_end_x);
    }

    #[test]
    fn edge_guide_draws_configured_pixels_behind_objects() {
        let layout = test_layout(1);
        let current = edge_fruit(0.0, 10);
        let mut next = edge_fruit(200.0, 20);
        next.edge = false;
        let mut image = Img::new(400, 130, [7, 7, 7, 255]);

        draw_edge_guides(&mut image, &[current, next], &layout);

        let midpoint_x = playfield_left(0) + 50;
        let chart_bottom = crate::config::current().layout.catch.png.PAGE_MARGIN_TOP
            + crate::config::current().layout.catch.png.INFO_MARGIN_TOP
            + layout.total_column_height;
        assert_eq!(
            image.get(midpoint_x as u32, (chart_bottom - 15) as u32),
            crate::config::current().layout.catch.png.EDGE_GUIDE_COLOR
        );
    }

    #[test]
    fn column_height_is_aligned_to_dominant_measure_interval() {
        let timing_lines: Vec<TimingLine> = (0..10)
            .map(|index| TimingLine {
                time: index * 2_000,
                is_measure: true,
                show_label: true,
                bpm: None,
            })
            .collect();

        let height = predominant_measure_aligned_height(&timing_lines, 0.5, 5_500).unwrap();
        assert_eq!(height, 5_000);
        assert_eq!(height % 1_000, 0);
    }
    #[test]
    fn edge_combo_numbers_ignore_tiny_droplets_and_bananas() {
        let mut first = edge_fruit(20.0, 10);
        first.edge = false;
        let mut tiny = edge_fruit(30.0, 15);
        tiny.object_type = ObjType::TinyDroplet;
        tiny.edge = false;
        let mut droplet = edge_fruit(40.0, 20);
        droplet.object_type = ObjType::Droplet;
        let mut banana = edge_fruit(50.0, 25);
        banana.object_type = ObjType::Banana;
        banana.edge = false;
        let last = edge_fruit(60.0, 30);

        let labels = edge_combo_numbers(&[first, tiny, droplet, banana, last]);

        assert_eq!(labels, vec![(2, 2), (4, 3)]);
    }

    #[test]
    fn edge_combo_label_is_drawn_next_to_the_edge_object() {
        let layout = test_layout(1);
        let current = edge_fruit(100.0, 10);
        let mut next = edge_fruit(200.0, 20);
        next.edge = false;
        let mut image = Img::new(
            400,
            130,
            crate::config::current().layout.catch.png.IMAGE_BACKGROUND,
        );

        draw_edge_combo_labels(&mut image, &[current, next], &layout);

        let has_white_label_pixel = (0..image.h).any(|y| {
            (0..image.w).any(|x| {
                image.get(x, y)
                    == crate::config::current()
                        .layout
                        .catch
                        .png
                        .EDGE_COMBO_LABEL_COLOR
            })
        });
        assert!(has_white_label_pixel);
    }
}
