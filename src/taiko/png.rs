//! osu!taiko PNG 静态图渲染器：多行滚动谱面 + 节拍线 + SV 标注。
//!
//! 行切分以小节线为锚点：每行的起点对齐到一条小节线（measure line），
//! 保证视觉上每行最左侧都是重拍位置。

use crate::canvas::Img;
use crate::composer;
use crate::errors::{PreviewError, Result};
use crate::models::{Beatmap, TaikoHitObject};
use crate::mods::ModSettings;
use crate::parser::round_half_even;
use crate::text::{draw_text, text_size};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::constants::*;
use super::notes::{
    cached_roll_tail, draw_note_disc, draw_track_background, RenderCache,
};
use super::timing::*;

#[inline]
fn pyround(v: f64) -> i64 {
    round_half_even(v)
}

// ─── PNG 布局 ───

#[derive(Debug, Clone)]
struct RenderLayout {
    row_count: i64,
    /// 每行的最大内容宽度（像素）；行的实际终点由小节线对齐决定。
    max_row_width: i64,
    content_width: i64,
    image_width: i64,
    image_height: i64,
    normal_note_diameter: i64,
    big_note_diameter: i64,
    /// 每行的滚动空间起始位置（已对齐到小节线）。
    row_start_positions: Vec<f64>,
}

impl RenderLayout {
    /// 给定绝对滚动位置，返回（行号, 行内局部位置）。
    fn locate(&self, position: f64) -> (i64, f64) {
        // row_start_positions 单调递增，找到最后一个 start <= position 的行
        let idx = self
            .row_start_positions
            .partition_point(|&s| s <= position)
            .saturating_sub(1) as i64;
        let local = position - self.row_start_positions[idx as usize];
        (idx, local)
    }
}

pub(crate) fn render_taiko_grid(
    beatmap: &Beatmap,
    output_path: &Path,
    mods: Option<&ModSettings>,
) -> Result<PathBuf> {
    let hit_objects = apply_taiko_object_mods(taiko_hit_objects(beatmap), mods);
    if hit_objects.is_empty() {
        return Err(PreviewError::new("taiko beatmap has no hit objects"));
    }

    let chart_end_time = hit_objects.iter().map(|h| h.end_time).max().unwrap();
    if chart_end_time >= MAX_SUPPORTED_DURATION_MS {
        return Err(PreviewError::new(
            "songs longer than 10 minutes are not supported",
        ));
    }

    let mut cache = RenderCache::default();
    let slider_multiplier = effective_slider_multiplier(beatmap, mods)?;
    let timing_points = effective_timing_points(beatmap, mods);
    // 静态图的 note 间距只跟随红线 BPM（绿线 SV 不影响排版）
    let spacing_timing_points = spacing_timing_points_for_png(&timing_points);
    let mapper = build_scroll_mapper(
        &spacing_timing_points,
        chart_end_time,
        slider_multiplier,
        SPACING_BPM,
    );
    let redline_sections = build_redline_sections(&timing_points, chart_end_time);
    let kiai_sections = build_kiai_sections(&timing_points, chart_end_time);
    let first_note_time = hit_objects.iter().map(|h| h.start_time).min().unwrap_or(0);
    let timing_lines = build_timing_lines(
        &redline_sections,
        &mapper,
        MIN_BEAT_LINE_SPACING,
        &kiai_sections,
        first_note_time,
    );
    let layout = build_png_layout(
        chart_end_time,
        mapper.end_position(),
        &redline_sections,
        &timing_lines,
    );
    let sv_changes = build_sv_changes(&timing_points, chart_end_time, &mapper);

    let mut image = Img::new(
        layout.image_width as u32,
        layout.image_height as u32,
        IMAGE_BACKGROUND,
    );

    for row_index in 0..layout.row_count {
        draw_row_background(&mut image, &layout, row_index);
    }

    let mut last_label_time: Option<i64> = None;
    for timing_line in timing_lines.iter().rev() {
        let mut tl = timing_line.clone();
        if tl.show_label {
            if let Some(prev) = last_label_time {
                if (tl.time - prev).abs() < TIME_LABEL_MIN_INTERVAL_MS {
                    tl.show_label = false;
                }
            }
            if tl.show_label {
                last_label_time = Some(tl.time);
            }
        }
        draw_timing_line(&mut image, &tl, &layout);
    }

    draw_sv_indicators(&mut image, &sv_changes, &layout);

    for hit_object in hit_objects.iter().rev() {
        draw_hit_object(&mut image, hit_object, &mapper, &layout, &mut cache);
    }

    composer::save_png(&image, output_path)?;
    Ok(output_path.to_path_buf())
}

/// 计算每行的起始滚动位置，使行首对齐到小节线。
///
/// 从位置 0 开始，每行最多容纳 `max_row_width` 像素；下一行的起点取
/// 「不超过 当前行起点 + max_row_width 的最后一条小节线」。如果该范围内
/// 没有小节线（极端情况），则退化为定宽切分以保证推进。
fn compute_row_start_positions(
    measure_positions: &[f64],
    chart_width: f64,
    max_row_width: i64,
) -> Vec<f64> {
    let mut starts = vec![0.0f64];
    let width = max_row_width as f64;
    loop {
        let current = *starts.last().unwrap();
        if current + width >= chart_width {
            break;
        }
        // 在 (current, current+width] 范围内找最后一条小节线作为下一行起点
        let next = measure_positions
            .iter()
            .copied()
            .filter(|&p| p > current + 1.0 && p <= current + width)
            .fold(f64::NEG_INFINITY, f64::max);
        if next.is_finite() {
            starts.push(next);
        } else {
            starts.push(current + width);
        }
    }
    starts
}

fn build_png_layout(
    beatmap_duration: i64,
    chart_width: f64,
    redline_sections: &[RedlineSection],
    timing_lines: &[TimingLine],
) -> RenderLayout {
    let base_row_width = resolve_base_row_width(beatmap_duration);
    let bpm_width_multiplier = if SPACING_BPM > 0.0 {
        1.0
    } else {
        resolve_row_width_bpm_multiplier(redline_sections)
    };
    let max_row_width = pyround(base_row_width as f64 * bpm_width_multiplier);

    // 行首对齐：以小节线位置作为换行锚点
    let measure_positions: Vec<f64> = timing_lines
        .iter()
        .filter(|l| l.is_measure)
        .map(|l| l.position)
        .collect();
    let row_start_positions =
        compute_row_start_positions(&measure_positions, chart_width, max_row_width);
    let row_count = row_start_positions.len() as i64;

    // 实际使用的最大行宽：小节对齐后每行都比 max_row_width 短，
    // 画布宽度按真实内容收缩，避免右侧留下整片空白。
    let mut used_row_width = 0.0f64;
    for (i, &start) in row_start_positions.iter().enumerate() {
        let end = if i + 1 < row_start_positions.len() {
            row_start_positions[i + 1]
        } else {
            chart_width
        };
        used_row_width = used_row_width.max(end - start);
    }
    let used_row_width = (used_row_width.ceil() as i64).clamp(1, max_row_width);

    let content_width = ROW_INNER_PADDING_X * 2 + used_row_width;
    let image_width = PAGE_MARGIN_X * 2 + content_width;
    let image_height = PAGE_MARGIN_Y * 2 + FIRST_ROW_SV_TOP_MARGIN + row_count * ROW_HEIGHT + row_count * ROW_GAP;
    let normal_note_diameter = pyround(ROW_HEIGHT as f64 * NORMAL_NOTE_SIZE_RATIO);
    let big_note_diameter = pyround(normal_note_diameter as f64 * BIG_NOTE_SCALE);
    RenderLayout {
        row_count,
        max_row_width,
        content_width,
        image_width,
        image_height,
        normal_note_diameter,
        big_note_diameter,
        row_start_positions,
    }
}

fn resolve_base_row_width(beatmap_duration: i64) -> i64 {
    if beatmap_duration < 60_000 {
        return BASE_ROW_WIDTH_0_TO_1_MIN;
    }
    if beatmap_duration < 2 * 60_000 {
        return BASE_ROW_WIDTH_1_TO_2_MIN;
    }
    if beatmap_duration < 3 * 60_000 {
        return BASE_ROW_WIDTH_2_TO_3_MIN;
    }
    if beatmap_duration < 4 * 60_000 {
        return BASE_ROW_WIDTH_3_TO_4_MIN;
    }
    if beatmap_duration < 5 * 60_000 {
        return BASE_ROW_WIDTH_4_TO_5_MIN;
    }
    if beatmap_duration < 6 * 60_000 {
        return BASE_ROW_WIDTH_5_TO_6_MIN;
    }
    BASE_ROW_WIDTH_6_TO_10_MIN
}

fn resolve_row_width_bpm_multiplier(redline_sections: &[RedlineSection]) -> f64 {
    let main_bpm = resolve_main_bpm(redline_sections);
    if main_bpm < 180.0 {
        return ROW_WIDTH_BPM_0_TO_180;
    }
    if main_bpm < 240.0 {
        return ROW_WIDTH_BPM_180_TO_240;
    }
    if main_bpm < 300.0 {
        return ROW_WIDTH_BPM_240_TO_300;
    }
    ROW_WIDTH_BPM_300_PLUS
}

fn resolve_main_bpm(redline_sections: &[RedlineSection]) -> f64 {
    // Weight each rounded BPM by section duration; pick the dominant one
    // (first-inserted wins ties, like Python's max over insertion order).
    let mut order: Vec<i64> = Vec::new();
    let mut weighted: HashMap<i64, i64> = HashMap::new();
    for section in redline_sections {
        let bpm = pyround(60_000.0 / section.beat_length);
        let duration = (section.end_time - section.start_time).max(0);
        if !weighted.contains_key(&bpm) {
            order.push(bpm);
        }
        *weighted.entry(bpm).or_insert(0) += duration;
    }

    if order.is_empty() {
        return 120.0;
    }
    let mut best_bpm = order[0];
    let mut best_duration = weighted[&order[0]];
    for &bpm in &order[1..] {
        if weighted[&bpm] > best_duration {
            best_duration = weighted[&bpm];
            best_bpm = bpm;
        }
    }
    best_bpm as f64
}

// ─── row helpers ───

fn png_row_top(row_index: i64) -> i64 {
    let base = PAGE_MARGIN_Y + row_index * (ROW_HEIGHT + ROW_GAP);
    if row_index == 0 {
        base + FIRST_ROW_SV_TOP_MARGIN
    } else {
        base
    }
}

fn png_row_center_y(row_index: i64) -> i64 {
    png_row_top(row_index) + ROW_HEIGHT / 2
}

fn png_row_chart_left(_layout: &RenderLayout, _row_index: i64) -> i64 {
    PAGE_MARGIN_X + ROW_INNER_PADDING_X
}

// ─── 行背景 ───

/// 绘制单行背景：所有行只画轨道（程序化，无图片）。
fn draw_row_background(image: &mut Img, layout: &RenderLayout, row_index: i64) {
    let row_top = png_row_top(row_index);
    draw_track_background(image, PAGE_MARGIN_X, row_top, layout.content_width, ROW_HEIGHT);
}

// ─── 节拍线绘制 ───

fn draw_timing_line(image: &mut Img, timing_line: &TimingLine, layout: &RenderLayout) {
    let (row_index, local_position) = layout.locate(timing_line.position);
    // 超出行宽的线（行尾与下一行行首之间的过渡区）不绘制
    if local_position > layout.max_row_width as f64 + 0.5 {
        return;
    }
    let line_x = pyround(png_row_chart_left(layout, row_index) as f64 + local_position);
    let line_y0 = png_row_top(row_index);
    let line_y1 = line_y0 + ROW_HEIGHT;

    if timing_line.is_measure {
        image.fill_rect(line_x, line_y0, line_x + 1, line_y1, MEASURE_LINE_COLOR);
    } else {
        image.set_rect(line_x, line_y0, line_x, line_y1, BEAT_LINE_COLOR);
    }

    if timing_line.show_label {
        draw_time_label(image, timing_line, line_x, line_y0, layout);
    }
}

fn draw_time_label(
    image: &mut Img,
    timing_line: &TimingLine,
    line_x: i64,
    row_top: i64,
    layout: &RenderLayout,
) {
    let label = time_label_text(timing_line.time);
    let note: Option<&str> = if timing_line.is_kiai_start {
        Some("Kiai Start")
    } else {
        None
    };
    let label_color = if timing_line.is_kiai {
        ACCENT_LABEL_COLOR
    } else {
        RULER_TEXT_COLOR
    };
    let (label_width, label_height) = text_size(&label, TIME_LABEL_FONT_SIZE);
    let label_x = pyround(line_x as f64 - label_width as f64 / 2.0)
        .min(PAGE_MARGIN_X + layout.content_width - label_width as i64 - LABEL_RIGHT_PADDING)
        .max(PAGE_MARGIN_X);
    let label_y = row_top + ROW_HEIGHT + TIME_LABEL_TOP_GAP;

    draw_text(image, label_x, label_y, &label, TIME_LABEL_FONT_SIZE, label_color);

    let mut next_y = label_y + label_height as i64;
    if let Some(note) = note {
        let (note_width, note_height) = text_size(note, TIME_LABEL_NOTE_FONT_SIZE);
        let note_x = pyround(line_x as f64 - note_width as f64 / 2.0)
            .min(PAGE_MARGIN_X + layout.content_width - note_width as i64 - LABEL_RIGHT_PADDING)
            .max(PAGE_MARGIN_X);
        let note_y = next_y + TIME_LABEL_NOTE_TOP_GAP;
        draw_text(
            image,
            note_x,
            note_y,
            note,
            TIME_LABEL_NOTE_FONT_SIZE,
            ACCENT_LABEL_COLOR,
        );
        next_y = note_y + note_height as i64;
    }

    if let Some(bpm) = timing_line.bpm {
        let bpm_label = format!("{bpm:.0}BPM");
        let (bpm_width, _) = text_size(&bpm_label, BPM_FONT_SIZE);
        let bpm_x = pyround(line_x as f64 - bpm_width as f64 / 2.0)
            .min(PAGE_MARGIN_X + layout.content_width - bpm_width as i64 - LABEL_RIGHT_PADDING)
            .max(PAGE_MARGIN_X);
        let bpm_y = next_y + BPM_TOP_GAP;
        let bpm_color = if timing_line.is_kiai {
            ACCENT_LABEL_COLOR
        } else {
            RULER_TEXT_COLOR
        };
        draw_text(image, bpm_x, bpm_y, &bpm_label, BPM_FONT_SIZE, bpm_color);
    }
}

// ─── SV 标注 ───

fn draw_sv_indicators(image: &mut Img, sv_changes: &[SvChange], layout: &RenderLayout) {
    for sv_change in sv_changes.iter().rev() {
        let (row_index, local_position) = layout.locate(sv_change.position);
        let x = pyround(png_row_chart_left(layout, row_index) as f64 + local_position);
        let row_top = png_row_top(row_index);

        let label = format_sv_label(sv_change.sv);
        let (label_width, label_height) = text_size(&label, SV_TEXT_FONT_SIZE);

        let label_x = pyround(x as f64 - label_width as f64 / 2.0);
        let label_y = (row_top - SV_TOP_GAP - label_height as i64).max(PAGE_MARGIN_Y);
        draw_text(image, label_x, label_y, &label, SV_TEXT_FONT_SIZE, SV_TEXT_COLOR);
    }
}

fn format_sv_label(sv: f64) -> String {
    let rounded_1: f64 = format!("{sv:.1}").parse().unwrap_or(sv);
    if sv == rounded_1 {
        format!("{sv:.1}x")
    } else {
        format!("{sv:.2}x")
    }
}

// ─── hit object drawing ───

fn draw_hit_object(
    image: &mut Img,
    hit_object: &TaikoHitObject,
    mapper: &ScrollPositionMapper,
    layout: &RenderLayout,
    cache: &mut RenderCache,
) {
    if hit_object.hit_type & SWELL_FLAG != 0 {
        draw_span_object(image, hit_object, mapper, layout, cache, true, SWELL_COLOR, true);
        return;
    }
    if hit_object.hit_type & DRUMROLL_FLAG != 0 {
        let is_big_roll = hit_object.hitsound & HIT_SOUNDS_STRONG != 0;
        draw_span_object(
            image,
            hit_object,
            mapper,
            layout,
            cache,
            is_big_roll,
            ROLL_COLOR,
            false,
        );
        return;
    }
    draw_circle_object(image, hit_object, mapper, layout, cache);
}

fn draw_circle_object(
    image: &mut Img,
    hit_object: &TaikoHitObject,
    mapper: &ScrollPositionMapper,
    layout: &RenderLayout,
    cache: &mut RenderCache,
) {
    let absolute_position = mapper.position_at(hit_object.start_time as f64);
    let (row_index, local_position) = layout.locate(absolute_position);
    let center_x = pyround(png_row_chart_left(layout, row_index) as f64 + local_position);
    let center_y = png_row_center_y(row_index);
    let is_strong = hit_object.hitsound & HIT_SOUNDS_STRONG != 0;
    let is_rim = hit_object.hitsound & HIT_SOUNDS_RIM != 0;
    let diameter = if is_strong {
        layout.big_note_diameter
    } else {
        layout.normal_note_diameter
    };
    let color = if is_rim {
        RIM_NOTE_COLOR
    } else {
        CENTRE_NOTE_COLOR
    };

    draw_note_disc(image, cache, color, diameter, center_x, center_y, false);
}

#[allow(clippy::too_many_arguments)]
fn draw_span_object(
    image: &mut Img,
    hit_object: &TaikoHitObject,
    mapper: &ScrollPositionMapper,
    layout: &RenderLayout,
    cache: &mut RenderCache,
    is_swell: bool,
    span_color: [u8; 3],
    draw_swell_marker: bool,
) {
    let absolute_start = mapper.position_at(hit_object.start_time as f64);
    let absolute_end = mapper
        .position_at(hit_object.end_time as f64)
        .max(absolute_start);
    let (row_start, head_local) = layout.locate(absolute_start);
    let (row_end, tail_local) = layout.locate(absolute_end);
    let head_diameter = if is_swell {
        layout.big_note_diameter
    } else {
        layout.normal_note_diameter
    };
    let body_ratio = if is_swell {
        SWELL_BODY_HEIGHT_RATIO
    } else {
        SPAN_BODY_HEIGHT_RATIO
    };
    let body_height = pyround(head_diameter as f64 * body_ratio);

    // 逐行绘制长条主体：每行的可见区间为 [行起点, 下一行起点)
    for row_index in row_start..=row_end {
        let row_origin = layout.row_start_positions[row_index as usize];
        let row_limit = if (row_index as usize) + 1 < layout.row_start_positions.len() {
            layout.row_start_positions[row_index as usize + 1]
        } else {
            row_origin + layout.max_row_width as f64
        };
        let segment_start = absolute_start.max(row_origin);
        let segment_end = absolute_end.min(row_limit);
        let start_x =
            pyround(png_row_chart_left(layout, row_index) as f64 + (segment_start - row_origin));
        let end_x =
            pyround(png_row_chart_left(layout, row_index) as f64 + (segment_end - row_origin));
        draw_roll_body(
            image,
            span_color,
            start_x,
            end_x,
            png_row_center_y(row_index),
            body_height,
        );
    }

    let head_center_x = pyround(png_row_chart_left(layout, row_start) as f64 + head_local);
    let tail_join_x = pyround(png_row_chart_left(layout, row_end) as f64 + tail_local);
    draw_note_disc(
        image,
        cache,
        span_color,
        head_diameter,
        head_center_x,
        png_row_center_y(row_start),
        draw_swell_marker,
    );
    draw_span_tail(
        image,
        span_color,
        tail_join_x,
        png_row_center_y(row_end),
        body_height,
        cache,
    );
}

fn draw_roll_body(
    image: &mut Img,
    color: [u8; 3],
    start_x: i64,
    end_x: i64,
    center_y: i64,
    height: i64,
) {
    if end_x <= start_x {
        return;
    }
    let y0 = pyround(center_y as f64 - height as f64 / 2.0);
    image.fill_rect(
        start_x,
        y0,
        end_x - 1,
        y0 + height - 1,
        [color[0], color[1], color[2], 255],
    );
}

fn draw_span_tail(
    image: &mut Img,
    color: [u8; 3],
    join_x: i64,
    center_y: i64,
    height: i64,
    cache: &mut RenderCache,
) {
    let y = pyround(center_y as f64 - height as f64 / 2.0);
    let tail = cached_roll_tail(cache, color, height);
    image.alpha_composite(tail, join_x, y);
}
