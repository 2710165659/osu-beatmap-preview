//! osu!mania PNG 场景绘制：根据 CLI 传入的布局和对象绘制静态谱面图。

use crate::model::ManiaHitObject;
use crate::processing::parse::round_half_even;
use crate::processing::timeline::TimeAxis;
use crate::render::canvas::{Img, Rgba};
use crate::render::text::{draw_text, text_size};

use super::{darken, format_sv_label};

#[derive(Clone)]
pub struct TimingLine {
    pub time: i64,
    pub color: Rgba,
    pub show_label: bool,
    pub bpm_label: Option<String>,
}

pub struct ManiaPngLayout {
    pub column_count: i64,
    pub time_per_column: i64,
    pub column_height: i64,
    pub total_column_height: i64,
    pub lane_area_width: i64,
    pub column_width: i64,
    pub lane_widths: Vec<i64>,
    pub lane_left_offsets: Vec<i64>,
    pub top_buffer: i64,
    pub image_width: i64,
    pub image_height: i64,
    pub chart_start_time: i64,
}

fn png_column_left(column_index: i64, layout: &ManiaPngLayout) -> i64 {
    let config = &crate::config::current().render.mania.png;
    config.sizing.PAGE_MARGIN_LEFT
        + config.sizing.INFO_MARGIN_LEFT
        + column_index
            * (config.sizing.INFO_MARGIN_LEFT
                + layout.column_width
                + config.sizing.INFO_MARGIN_RIGHT
                + config.sizing.COLUMN_GAP)
}

fn png_chart_top() -> i64 {
    crate::config::current()
        .render
        .mania
        .png
        .sizing
        .PAGE_MARGIN_TOP
        + crate::config::current()
            .render
            .mania
            .png
            .sizing
            .INFO_MARGIN_TOP
}

fn draw_column_background(
    image: &mut Img,
    key_count: i32,
    column_index: i64,
    layout: &ManiaPngLayout,
) {
    let column_left = png_column_left(column_index, layout);
    let chart_top = png_chart_top();
    let lane_area_left = column_left
        + crate::config::current()
            .render
            .mania
            .png
            .sizing
            .LEFT_PANEL_WIDTH;

    image.set_rect_size(
        column_left,
        chart_top,
        crate::config::current()
            .render
            .mania
            .png
            .sizing
            .LEFT_PANEL_WIDTH,
        layout.total_column_height,
        crate::config::current()
            .render
            .mania
            .png
            .style
            .LEFT_PANEL_BACKGROUND,
    );

    for lane_index in 0..key_count as i64 {
        let lane_left = lane_area_left
            + layout.lane_left_offsets[lane_index as usize]
            + lane_index * crate::config::current().render.mania.png.sizing.LANE_GAP;
        let lane_right = lane_left + layout.lane_widths[lane_index as usize];
        image.set_rect_size(
            lane_left,
            chart_top,
            lane_right - lane_left,
            layout.total_column_height,
            crate::config::current()
                .render
                .mania
                .png
                .style
                .LANE_BACKGROUND,
        );
        if lane_index > 0 {
            let separator_width = crate::render::geometry::scale_stroke_px(
                1.0,
                crate::render::geometry::output_scale(
                    crate::render::geometry::GameMode::Mania,
                    crate::render::geometry::OutputFormat::Png,
                ),
            );
            image.set_rect_size(
                lane_left,
                chart_top,
                separator_width,
                layout.total_column_height,
                crate::config::current()
                    .render
                    .mania
                    .png
                    .style
                    .LANE_SEPARATOR,
            );
        }
    }
}

fn draw_timing_line(
    image: &mut Img,
    timing_line: &TimingLine,
    layout: &ManiaPngLayout,
    time_axis: TimeAxis,
) {
    let column_index =
        (timing_line.time.div_euclid(layout.time_per_column)).min(layout.column_count - 1);
    let local_time = timing_line.time - column_index * layout.time_per_column;
    let column_left = png_column_left(column_index, layout);
    let lane_area_left = column_left
        + crate::config::current()
            .render
            .mania
            .png
            .sizing
            .LEFT_PANEL_WIDTH;
    let chart_top = png_chart_top() + layout.top_buffer;
    let y = chart_top + layout.column_height
        - round_half_even(
            local_time as f64
                * crate::config::current()
                    .render
                    .mania
                    .png
                    .sizing
                    .PIXELS_PER_MS,
        );

    let line_height = crate::render::geometry::scale_stroke_px(
        1.0,
        crate::render::geometry::output_scale(
            crate::render::geometry::GameMode::Mania,
            crate::render::geometry::OutputFormat::Png,
        ),
    );
    image.set_rect_size(
        lane_area_left,
        y,
        layout.lane_area_width,
        line_height,
        timing_line.color,
    );

    if timing_line.show_label {
        let label = crate::render::text::format_seconds_tenths(
            time_axis.to_display(timing_line.time + layout.chart_start_time),
        );
        let (_, label_height) = text_size(
            &label,
            crate::config::current()
                .render
                .mania
                .png
                .sizing
                .TIME_LABEL_FONT_SIZE,
        );
        let text_mid_y = label_height as f64 / 2.0;
        // 右侧信息区从轨道右边缘开始左对齐，避免长文本被推回轨道内部。
        let text_gap = crate::render::geometry::scale_px(
            4.0,
            crate::render::geometry::output_scale(
                crate::render::geometry::GameMode::Mania,
                crate::render::geometry::OutputFormat::Png,
            ),
        );
        let label_x = column_left + layout.column_width + text_gap;
        let label_y = (chart_top as f64).max(y as f64 - text_mid_y).floor() as i64;
        draw_text(
            image,
            label_x,
            label_y,
            &label,
            crate::config::current()
                .render
                .mania
                .png
                .sizing
                .TIME_LABEL_FONT_SIZE,
            crate::config::current()
                .render
                .mania
                .png
                .style
                .RULER_TEXT_COLOR,
        );

        if let Some(ref bpm_label) = timing_line.bpm_label {
            let (_, bpm_h) = text_size(
                bpm_label,
                crate::config::current()
                    .render
                    .mania
                    .png
                    .sizing
                    .TIME_LABEL_FONT_SIZE,
            );
            let bpm_x = column_left + layout.column_width + text_gap;
            let bpm_gap = crate::render::geometry::scale_px(
                3.0,
                crate::render::geometry::output_scale(
                    crate::render::geometry::GameMode::Mania,
                    crate::render::geometry::OutputFormat::Png,
                ),
            );
            let bpm_y = (label_y + label_height as i64 + bpm_gap).min(
                crate::config::current()
                    .render
                    .mania
                    .png
                    .sizing
                    .PAGE_MARGIN_TOP
                    + layout.total_column_height
                    - bpm_h as i64,
            );
            draw_text(
                image,
                bpm_x,
                bpm_y,
                bpm_label,
                crate::config::current()
                    .render
                    .mania
                    .png
                    .sizing
                    .TIME_LABEL_FONT_SIZE,
                crate::config::current()
                    .render
                    .mania
                    .png
                    .style
                    .RULER_TEXT_COLOR,
            );
        }
    }
}

fn draw_png_hit_object(
    image: &mut Img,
    hit_object: &ManiaHitObject,
    palette: &[Rgba],
    layout: &ManiaPngLayout,
) {
    let start_column =
        (hit_object.start_time.div_euclid(layout.time_per_column)).min(layout.column_count - 1);
    let end_column =
        (hit_object.end_time.div_euclid(layout.time_per_column)).min(layout.column_count - 1);
    let lane = (hit_object.lane.max(0) as usize).min(palette.len() - 1);
    let lane_color = palette[lane];
    let hold_color = darken(lane_color, 0.5);

    for column_index in start_column..=end_column {
        let column_left = png_column_left(column_index, layout);
        let lane_area_left = column_left
            + crate::config::current()
                .render
                .mania
                .png
                .sizing
                .LEFT_PANEL_WIDTH;
        let chart_top = png_chart_top();
        let chart_axis_top = chart_top + layout.top_buffer;
        let chart_bottom = chart_axis_top + layout.column_height;
        let lane_left = lane_area_left
            + layout.lane_left_offsets[lane]
            + lane as i64 * crate::config::current().render.mania.png.sizing.LANE_GAP
            + crate::config::current()
                .render
                .mania
                .png
                .sizing
                .NOTE_SIDE_PADDING;
        let lane_right = lane_left + layout.lane_widths[lane]
            - crate::config::current()
                .render
                .mania
                .png
                .sizing
                .NOTE_SIDE_PADDING
                * 2;
        let segment_start = hit_object
            .start_time
            .max(column_index * layout.time_per_column);
        let segment_end = hit_object
            .end_time
            .min((column_index + 1) * layout.time_per_column);
        let y_start = chart_axis_top + layout.column_height
            - round_half_even(
                (segment_start - column_index * layout.time_per_column) as f64
                    * crate::config::current()
                        .render
                        .mania
                        .png
                        .sizing
                        .PIXELS_PER_MS,
            );
        let y_end = chart_axis_top + layout.column_height
            - round_half_even(
                (segment_end - column_index * layout.time_per_column) as f64
                    * crate::config::current()
                        .render
                        .mania
                        .png
                        .sizing
                        .PIXELS_PER_MS,
            );

        if hit_object.is_long_note {
            let body_top = chart_top.max(
                y_end.min(
                    y_start
                        - crate::config::current()
                            .render
                            .mania
                            .png
                            .sizing
                            .NOTE_HEAD_HEIGHT,
                ),
            );
            let body_bottom = chart_bottom.min(y_start);
            if body_top < body_bottom {
                image.set_rect_size(
                    lane_left,
                    body_top,
                    lane_right - lane_left,
                    body_bottom - body_top,
                    hold_color,
                );
            }
            if column_index == start_column {
                let head_top = chart_top.max(
                    y_start
                        - crate::config::current()
                            .render
                            .mania
                            .png
                            .sizing
                            .NOTE_HEAD_HEIGHT,
                );
                let head_bottom = chart_bottom.min(y_start);
                if head_top < head_bottom {
                    image.set_rect_size(
                        lane_left,
                        head_top,
                        lane_right - lane_left,
                        head_bottom - head_top,
                        lane_color,
                    );
                }
            }
        } else {
            let head_top = chart_top.max(
                y_start
                    - crate::config::current()
                        .render
                        .mania
                        .png
                        .sizing
                        .NOTE_HEAD_HEIGHT,
            );
            let head_bottom = chart_bottom.min(y_start);
            if head_top < head_bottom {
                image.set_rect_size(
                    lane_left,
                    head_top,
                    lane_right - lane_left,
                    head_bottom - head_top,
                    lane_color,
                );
            }
        }
    }
}

fn draw_sv_indicator(image: &mut Img, sv_change: (i64, f64), layout: &ManiaPngLayout) {
    let (time, sv) = sv_change;
    let column_index = (time.div_euclid(layout.time_per_column)).min(layout.column_count - 1);
    let local_time = time - column_index * layout.time_per_column;
    let column_left = png_column_left(column_index, layout);
    let chart_top = png_chart_top() + layout.top_buffer;
    let y = chart_top + layout.column_height
        - round_half_even(
            local_time as f64
                * crate::config::current()
                    .render
                    .mania
                    .png
                    .sizing
                    .PIXELS_PER_MS,
        );

    let label = format_sv_label(sv);
    let (label_width, label_height) = text_size(
        &label,
        crate::config::current()
            .render
            .mania
            .png
            .sizing
            .SV_TEXT_FONT_SIZE,
    );
    let text_mid_y = label_height as f64 / 2.0;
    let label_gap = crate::render::geometry::scale_px(
        1.0,
        crate::render::geometry::output_scale(
            crate::render::geometry::GameMode::Mania,
            crate::render::geometry::OutputFormat::Png,
        ),
    );
    let label_x = (column_left - label_gap - label_width as i64).max(0);
    let label_y = (chart_top as f64).max(y as f64 - text_mid_y).floor() as i64;
    draw_text(
        image,
        label_x,
        label_y,
        &label,
        crate::config::current()
            .render
            .mania
            .png
            .sizing
            .SV_TEXT_FONT_SIZE,
        crate::config::current()
            .render
            .mania
            .png
            .style
            .SV_TEXT_COLOR,
    );
}

/// 按 `ManiaPngLayout` 绘制整张 mania 静态谱面图。
#[allow(clippy::too_many_arguments)]
pub fn render_mania_png_scene(
    layout: &ManiaPngLayout,
    key_count: i32,
    palette: &[Rgba],
    hit_objects: &[ManiaHitObject],
    timing_lines: &[TimingLine],
    sv_changes: &[(i64, f64)],
    time_axis: TimeAxis,
) -> Img {
    let mut image = Img::new(
        layout.image_width as u32,
        layout.image_height as u32,
        crate::config::current()
            .render
            .mania
            .png
            .style
            .IMAGE_BACKGROUND,
    );

    for column_index in 0..layout.column_count {
        draw_column_background(&mut image, key_count, column_index, layout);
    }

    let mut last_label_time: Option<i64> = None;
    for timing_line in timing_lines {
        let mut tl = timing_line.clone();
        if tl.show_label {
            if let Some(prev) = last_label_time {
                if (tl.time - prev).abs()
                    < crate::config::current()
                        .render
                        .mania
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

    for sv_change in sv_changes {
        draw_sv_indicator(&mut image, *sv_change, layout);
    }

    for hit_object in hit_objects {
        draw_png_hit_object(&mut image, hit_object, palette, layout);
    }

    image
}
