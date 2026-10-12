//! osu!taiko PNG 场景绘制：根据 CLI 传入的布局计划把静态谱面绘制成图像。

use crate::model::TaikoHitObject;
use crate::processing::timeline::TimeAxis;
use crate::render::canvas::Img;
use crate::render::text::{draw_text, text_size};

use super::animation::required_swell_hits;
use super::constants::*;
use super::notes::{
    cached_roll_tail, cached_swell_disc, draw_note_disc, draw_track_background, RenderCache,
};
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
    pub overall_difficulty: f64,
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
        draw_swell_object(
            image,
            hit_object,
            mapper,
            layout,
            cache,
            layout.overall_difficulty,
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

/// 转盘圆心对齐轨道，圆内标注总击打数，圆下标注持续时间。
fn draw_swell_object(
    image: &mut Img,
    hit_object: &TaikoHitObject,
    mapper: &ScrollPositionMapper,
    layout: &TaikoPngLayout,
    cache: &mut RenderCache,
    overall_difficulty: f64,
) {
    let (row_index, local_position) =
        layout.locate(mapper.position_at(hit_object.start_time as f64));
    let center_x = pyround(png_row_chart_left(layout, row_index) as f64 + local_position);
    let runtime = crate::config::current();
    let config = &runtime.render.taiko.png.sizing;
    let center_y = png_row_center_y(row_index);
    let diameter = layout.big_note_diameter;
    image.alpha_composite(
        cached_swell_disc(cache, diameter),
        pyround(center_x as f64 - diameter as f64 / 2.0),
        pyround(center_y as f64 - diameter as f64 / 2.0),
    );

    let duration_ms = (hit_object.end_time - hit_object.start_time).max(0);
    let label = required_swell_hits(duration_ms, overall_difficulty).to_string();
    let mut font_size = config.SWELL_HIT_COUNT_FONT_SIZE;
    // 长转盘的多位数字缩小到圆内；字号与圆直径均已应用 PNG 的 SCALE。
    let available = pyround(diameter as f64 * 0.66).max(1) as u32;
    let (width, height) = text_size(&label, font_size);
    if width.max(height) > available {
        font_size =
            ((font_size as u64 * available as u64) / width.max(height) as u64).max(1) as u32;
    }
    let (width, height) = text_size(&label, font_size);
    draw_text(
        image,
        center_x - width as i64 / 2,
        center_y - height as i64 / 2,
        &label,
        font_size,
        NOTE_RING_COLOR,
    );

    let duration = crate::render::text::format_seconds_tenths(duration_ms);
    let duration_y =
        pyround(center_y as f64 + diameter as f64 / 2.0) + config.SWELL_DURATION_TOP_GAP;
    // 圆心保持在轨道中线，只缩小时长文字以避开轨道外的时间轴标签。
    let duration_font_size = config
        .SWELL_DURATION_FONT_SIZE
        .min((png_row_top(row_index) + config.ROW_HEIGHT - duration_y).max(1) as u32);
    let (duration_width, _) = text_size(&duration, duration_font_size);
    let duration_x = (center_x - duration_width as i64 / 2)
        .min(config.PAGE_MARGIN_LEFT + layout.content_width - duration_width as i64)
        .max(config.PAGE_MARGIN_LEFT);
    draw_text(
        image,
        duration_x,
        duration_y,
        &duration,
        duration_font_size,
        runtime.render.taiko.png.style.RULER_TEXT_COLOR,
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_swell_centers_the_circle_and_shows_total_hits_and_duration() {
        let mut runtime = crate::config::CoreConfig::default();
        runtime.render.taiko.png.sizing.SWELL_HIT_COUNT_FONT_SIZE = 24;
        crate::config::with_config(std::sync::Arc::new(runtime), || {
            let mut layout = TaikoPngLayout {
                row_count: 1,
                max_row_width: 400,
                content_width: 466,
                image_width: 482,
                image_height: 200,
                normal_note_diameter: 48,
                big_note_diameter: 74,
                row_start_positions: vec![0.0],
                chart_start_time: 0,
                overall_difficulty: 5.0,
            };
            let mapper = ScrollPositionMapper::new(vec![ScrollSegment {
                start_time: 0,
                end_time: 10000,
                pixels_per_ms: 0.05,
                start_position: 0.0,
            }]);
            let object = TaikoHitObject {
                start_time: 3000,
                end_time: 6000,
                hit_type: SWELL_FLAG,
                hitsound: 0,
                samples: Vec::new(),
            };
            let x = png_row_chart_left(&layout, 0) + 150;
            let y = png_row_center_y(0);
            let config = crate::config::current();
            for (od, label) in [(5.0, "24"), (7.0, "29")] {
                layout.overall_difficulty = od;
                let image = render_taiko_png_scene(
                    &layout,
                    &[object.clone()],
                    &mapper,
                    &[],
                    &[],
                    TimeAxis::new(0),
                );
                assert_ne!(
                    image.get(x as u32, (y - 36) as u32),
                    image.get((x + 50) as u32, y as u32)
                );
                assert_ne!(
                    image.get(x as u32, (y + 36) as u32),
                    image.get((x + 50) as u32, y as u32)
                );
                assert_eq!(
                    image.get((x + 100) as u32, y as u32),
                    image.get((x + 50) as u32, y as u32),
                    "转盘不得保留长条尾部"
                );
                let size = config.render.taiko.png.sizing.SWELL_HIT_COUNT_FONT_SIZE;
                let (width, height) = text_size(label, size);
                let label_x = x - width as i64 / 2;
                let label_y = y - height as i64 / 2;
                let mut expected = Img::new(
                    width,
                    height,
                    config.render.taiko.png.style.TRACK_BACKGROUND_COLOR,
                );
                draw_text(&mut expected, 0, 0, label, size, NOTE_RING_COLOR);
                assert_eq!(
                    image
                        .crop(
                            label_x as u32,
                            label_y as u32,
                            label_x as u32 + width,
                            label_y as u32 + height
                        )
                        .data,
                    expected.data
                );
                let below_ring = y
                    + layout.big_note_diameter / 2
                    + config.render.taiko.png.sizing.SWELL_DURATION_TOP_GAP;
                assert!(
                    (below_ring..png_row_top(0) + config.render.taiko.png.sizing.ROW_HEIGHT).any(
                        |row| {
                            (x - 20..x + 20).any(|col| {
                                image.get(col as u32, row as u32)
                                    == config.render.taiko.png.style.RULER_TEXT_COLOR
                            })
                        }
                    ),
                    "圆下必须显示时长"
                );
            }
        });
    }
}
