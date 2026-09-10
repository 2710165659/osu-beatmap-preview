//! osu!taiko PNG 场景绘制：根据 CLI 传入的布局计划把静态谱面绘制成图像。

use crate::model::TaikoHitObject;
use crate::processing::timeline::TimeAxis;
use crate::render::canvas::Img;
use crate::render::text::{draw_text, text_size};

use super::constants::*;
use super::notes::{cached_roll_tail, draw_note_disc, draw_track_background, RenderCache};
use super::timing::*;

#[derive(Debug, Clone)]
pub struct TaikoPngLayout {
    pub row_count: i64,
    /// 每行的最大内容宽度（像素）；行的实际终点由小节线对齐决定。
    pub max_row_width: i64,
    pub content_width: i64,
    pub image_width: i64,
    pub image_height: i64,
    pub normal_note_diameter: i64,
    pub big_note_diameter: i64,
    /// 每行的滚动空间起始位置（已对齐到小节线）。
    pub row_start_positions: Vec<f64>,
    pub chart_start_time: i64,
}

impl TaikoPngLayout {
    /// 给定绝对滚动位置，返回（行号, 行内局部位置）。
    pub fn locate(&self, position: f64) -> (i64, f64) {
        // row_start_positions 单调递增，找到最后一个 start <= position 的行
        let idx = self
            .row_start_positions
            .partition_point(|&s| s <= position)
            .saturating_sub(1) as i64;
        let local = position - self.row_start_positions[idx as usize];
        (idx, local)
    }
}

// ─── 行辅助函数 ───

fn png_row_top(row_index: i64) -> i64 {
    let config = &crate::config::current().render.taiko.png;
    config.sizing.PAGE_MARGIN_TOP
        + config.sizing.INFO_MARGIN_TOP
        + row_index
            * (config.sizing.ROW_HEIGHT + config.sizing.INFO_MARGIN_BOTTOM + config.sizing.ROW_GAP)
}

fn png_row_center_y(row_index: i64) -> i64 {
    png_row_top(row_index) + crate::config::current().render.taiko.png.sizing.ROW_HEIGHT / 2
}

fn png_row_chart_left(_layout: &TaikoPngLayout, _row_index: i64) -> i64 {
    crate::config::current()
        .render
        .taiko
        .png
        .sizing
        .PAGE_MARGIN_LEFT
        + crate::config::current()
            .render
            .taiko
            .png
            .sizing
            .ROW_INNER_PADDING_X
}

// ─── 行背景（已预渲染为 track_bg，逐行 alpha_composite） ───

// ─── 节拍线绘制 ───

fn draw_timing_line(
    image: &mut Img,
    timing_line: &TimingLine,
    layout: &TaikoPngLayout,
    time_axis: TimeAxis,
) {
    let (row_index, local_position) = layout.locate(timing_line.position);
    // 超出行宽的线（行尾与下一行行首之间的过渡区）不绘制
    if local_position > layout.max_row_width as f64 + 0.5 {
        return;
    }
    let line_x = pyround(png_row_chart_left(layout, row_index) as f64 + local_position);
    let line_y0 = png_row_top(row_index);
    let line_y1 = line_y0 + crate::config::current().render.taiko.png.sizing.ROW_HEIGHT;

    let render_scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Taiko,
        crate::render::geometry::OutputFormat::Png,
    );
    let (width, color) = if timing_line.is_measure {
        (
            crate::render::geometry::scale_stroke_px(2.0, render_scale),
            crate::config::current()
                .render
                .taiko
                .png
                .style
                .MEASURE_LINE_COLOR,
        )
    } else {
        (
            crate::render::geometry::scale_stroke_px(1.0, render_scale),
            crate::config::current()
                .render
                .taiko
                .png
                .style
                .BEAT_LINE_COLOR,
        )
    };
    image.fill_rect_size(line_x, line_y0, width, line_y1 - line_y0, color);

    if timing_line.show_label {
        draw_time_label(image, timing_line, line_x, line_y0, layout, time_axis);
    }
}

fn draw_time_label(
    image: &mut Img,
    timing_line: &TimingLine,
    line_x: i64,
    row_top: i64,
    layout: &TaikoPngLayout,
    time_axis: TimeAxis,
) {
    let label = crate::render::text::format_seconds_tenths(
        time_axis.to_display(timing_line.time + layout.chart_start_time),
    );
    let note: Option<&str> = if timing_line.is_kiai_start {
        Some("Kiai Start")
    } else {
        None
    };
    let label_color = if timing_line.is_kiai {
        crate::config::current()
            .render
            .taiko
            .png
            .style
            .ACCENT_LABEL_COLOR
    } else {
        crate::config::current()
            .render
            .taiko
            .png
            .style
            .RULER_TEXT_COLOR
    };
    let (label_width, label_height) = text_size(
        &label,
        crate::config::current()
            .render
            .taiko
            .png
            .sizing
            .TIME_LABEL_FONT_SIZE,
    );
    let label_x = pyround(line_x as f64 - label_width as f64 / 2.0)
        .min(
            crate::config::current()
                .render
                .taiko
                .png
                .sizing
                .PAGE_MARGIN_LEFT
                + layout.content_width
                - label_width as i64
                - crate::config::current()
                    .render
                    .taiko
                    .png
                    .sizing
                    .LABEL_RIGHT_PADDING,
        )
        .max(
            crate::config::current()
                .render
                .taiko
                .png
                .sizing
                .PAGE_MARGIN_LEFT,
        );
    let label_y = row_top
        + crate::config::current().render.taiko.png.sizing.ROW_HEIGHT
        + crate::config::current()
            .render
            .taiko
            .png
            .sizing
            .TIME_LABEL_TOP_GAP;

    draw_text(
        image,
        label_x,
        label_y,
        &label,
        crate::config::current()
            .render
            .taiko
            .png
            .sizing
            .TIME_LABEL_FONT_SIZE,
        label_color,
    );

    let mut next_y = label_y + label_height as i64;
    if let Some(note) = note {
        let (note_width, note_height) = text_size(
            note,
            crate::config::current()
                .render
                .taiko
                .png
                .sizing
                .TIME_LABEL_NOTE_FONT_SIZE,
        );
        let note_x = pyround(line_x as f64 - note_width as f64 / 2.0)
            .min(
                crate::config::current()
                    .render
                    .taiko
                    .png
                    .sizing
                    .PAGE_MARGIN_LEFT
                    + layout.content_width
                    - note_width as i64
                    - crate::config::current()
                        .render
                        .taiko
                        .png
                        .sizing
                        .LABEL_RIGHT_PADDING,
            )
            .max(
                crate::config::current()
                    .render
                    .taiko
                    .png
                    .sizing
                    .PAGE_MARGIN_LEFT,
            );
        let note_y = next_y
            + crate::config::current()
                .render
                .taiko
                .png
                .sizing
                .TIME_LABEL_NOTE_TOP_GAP;
        draw_text(
            image,
            note_x,
            note_y,
            note,
            crate::config::current()
                .render
                .taiko
                .png
                .sizing
                .TIME_LABEL_NOTE_FONT_SIZE,
            crate::config::current()
                .render
                .taiko
                .png
                .style
                .ACCENT_LABEL_COLOR,
        );
        next_y = note_y + note_height as i64;
    }

    if let Some(bpm) = timing_line.bpm {
        let bpm_label = format!("{bpm:.0}BPM");
        let (bpm_width, _) = text_size(
            &bpm_label,
            crate::config::current()
                .render
                .taiko
                .png
                .sizing
                .BPM_FONT_SIZE,
        );
        let bpm_x = pyround(line_x as f64 - bpm_width as f64 / 2.0)
            .min(
                crate::config::current()
                    .render
                    .taiko
                    .png
                    .sizing
                    .PAGE_MARGIN_LEFT
                    + layout.content_width
                    - bpm_width as i64
                    - crate::config::current()
                        .render
                        .taiko
                        .png
                        .sizing
                        .LABEL_RIGHT_PADDING,
            )
            .max(
                crate::config::current()
                    .render
                    .taiko
                    .png
                    .sizing
                    .PAGE_MARGIN_LEFT,
            );
        let bpm_y = next_y + crate::config::current().render.taiko.png.sizing.BPM_TOP_GAP;
        let bpm_color = if timing_line.is_kiai {
            crate::config::current()
                .render
                .taiko
                .png
                .style
                .ACCENT_LABEL_COLOR
        } else {
            crate::config::current()
                .render
                .taiko
                .png
                .style
                .RULER_TEXT_COLOR
        };
        draw_text(
            image,
            bpm_x,
            bpm_y,
            &bpm_label,
            crate::config::current()
                .render
                .taiko
                .png
                .sizing
                .BPM_FONT_SIZE,
            bpm_color,
        );
    }
}

// ─── SV 标注 ───

fn draw_sv_indicators(image: &mut Img, sv_changes: &[SvChange], layout: &TaikoPngLayout) {
    for sv_change in sv_changes.iter().rev() {
        let (row_index, local_position) = layout.locate(sv_change.position);
        let x = pyround(png_row_chart_left(layout, row_index) as f64 + local_position);
        let row_top = png_row_top(row_index);

        let label = format_sv_label(sv_change.sv);
        let (label_width, label_height) = text_size(
            &label,
            crate::config::current()
                .render
                .taiko
                .png
                .sizing
                .SV_TEXT_FONT_SIZE,
        );

        let label_x = pyround(x as f64 - label_width as f64 / 2.0);
        let label_y = (row_top
            - crate::config::current().render.taiko.png.sizing.SV_TOP_GAP
            - label_height as i64)
            .max(
                crate::config::current()
                    .render
                    .taiko
                    .png
                    .sizing
                    .PAGE_MARGIN_TOP,
            );
        draw_text(
            image,
            label_x,
            label_y,
            &label,
            crate::config::current()
                .render
                .taiko
                .png
                .sizing
                .SV_TEXT_FONT_SIZE,
            crate::config::current()
                .render
                .taiko
                .png
                .style
                .SV_TEXT_COLOR,
        );
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

// ─── 音符绘制 ───

fn draw_hit_object(
    image: &mut Img,
    hit_object: &TaikoHitObject,
    mapper: &ScrollPositionMapper,
    layout: &TaikoPngLayout,
    cache: &mut RenderCache,
) {
    if hit_object.hit_type & SWELL_FLAG != 0 {
        draw_span_object(
            image,
            hit_object,
            mapper,
            layout,
            cache,
            true,
            super::constants::SWELL_COLOR,
            true,
        );
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
            super::constants::ROLL_COLOR,
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
    layout: &TaikoPngLayout,
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
        super::constants::RIM_NOTE_COLOR
    } else {
        super::constants::CENTRE_NOTE_COLOR
    };

    draw_note_disc(
        image,
        cache,
        color,
        diameter,
        center_x,
        center_y,
        crate::render::geometry::scale_stroke_px(
            1.0,
            crate::render::geometry::output_scale(
                crate::render::geometry::GameMode::Taiko,
                crate::render::geometry::OutputFormat::Png,
            ),
        ),
        false,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_span_object(
    image: &mut Img,
    hit_object: &TaikoHitObject,
    mapper: &ScrollPositionMapper,
    layout: &TaikoPngLayout,
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
        super::constants::SWELL_BODY_HEIGHT_RATIO
    } else {
        super::constants::SPAN_BODY_HEIGHT_RATIO
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
        crate::render::geometry::scale_stroke_px(
            1.0,
            crate::render::geometry::output_scale(
                crate::render::geometry::GameMode::Taiko,
                crate::render::geometry::OutputFormat::Png,
            ),
        ),
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
    image.fill_rect_size(
        start_x,
        y0,
        end_x - start_x,
        height,
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

/// 按 `TaikoPngLayout` 绘制整张静态谱面图。
pub fn render_taiko_png_scene(
    layout: &TaikoPngLayout,
    hit_objects: &[TaikoHitObject],
    mapper: &ScrollPositionMapper,
    timing_lines: &[TimingLine],
    sv_changes: &[SvChange],
    time_axis: TimeAxis,
) -> Img {
    let render_scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Taiko,
        crate::render::geometry::OutputFormat::Png,
    );
    let mut image = Img::new(
        layout.image_width as u32,
        layout.image_height as u32,
        crate::config::current()
            .render
            .taiko
            .png
            .style
            .IMAGE_BACKGROUND,
    );

    // 预渲染轨道背景条，每一行都相同。
    let track_bg = {
        let mut bg = Img::new(
            layout.content_width as u32,
            crate::config::current().render.taiko.png.sizing.ROW_HEIGHT as u32,
            [0, 0, 0, 0],
        );
        draw_track_background(
            &mut bg,
            0,
            0,
            layout.content_width,
            crate::config::current().render.taiko.png.sizing.ROW_HEIGHT,
            crate::config::current()
                .render
                .taiko
                .png
                .style
                .TRACK_BACKGROUND_COLOR,
            crate::config::current()
                .render
                .taiko
                .png
                .style
                .TRACK_EDGE_COLOR,
            crate::render::geometry::scale_stroke_px(1.0, render_scale),
        );
        bg
    };

    for row_index in 0..layout.row_count {
        let row_top = png_row_top(row_index);
        image.alpha_composite(
            &track_bg,
            crate::config::current()
                .render
                .taiko
                .png
                .sizing
                .PAGE_MARGIN_LEFT,
            row_top,
        );
    }

    let mut last_label_time: Option<i64> = None;
    for timing_line in timing_lines.iter().rev() {
        let mut tl = timing_line.clone();
        if tl.show_label {
            if let Some(prev) = last_label_time {
                if (tl.time - prev).abs()
                    < crate::config::current()
                        .render
                        .taiko
                        .png
                        .style
                        .TIME_LABEL_MIN_INTERVAL_MS
                {
                    tl.show_label = false;
                }
            }
            if tl.show_label {
                last_label_time = Some(tl.time);
            }
        }
        draw_timing_line(&mut image, &tl, layout, time_axis);
    }

    draw_sv_indicators(&mut image, sv_changes, layout);

    let mut cache = RenderCache::default();
    for hit_object in hit_objects.iter().rev() {
        draw_hit_object(&mut image, hit_object, mapper, layout, &mut cache);
    }

    image
}
