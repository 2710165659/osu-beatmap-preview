//! osu!mania PNG 网格渲染器。
//! 移植自 beatmap_preview/mania/renderer.py。

use crate::domain::errors::Result;
use crate::domain::models::{Beatmap, ManiaHitObject, TimingPoint};
use crate::domain::mods::ModSettings;
use crate::domain::parser::round_half_even;
use crate::domain::shared::time_selection::TimeAxis;
use crate::domain::timeout::RequestDeadline;
use crate::infrastructure::media::image::save_png;
use crate::render::canvas::{Img, Rgba};
use crate::render::text::{draw_text, text_size};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{
    apply_hold_off_mod, apply_inverse_mod, build_sv_changes, darken, is_native_mania,
    mania_objects, resolve_key_count,
};

#[derive(Clone)]
struct TimingLine {
    time: i64,
    color: Rgba,
    show_label: bool,
    bpm_label: Option<String>,
}

struct RenderLayout {
    column_count: i64,
    time_per_column: i64,
    column_height: i64,
    total_column_height: i64,
    lane_area_width: i64,
    column_width: i64,
    lane_widths: Vec<i64>,
    lane_left_offsets: Vec<i64>,
    top_buffer: i64,
    image_width: i64,
    image_height: i64,
    chart_start_time: i64,
}

pub(crate) fn render_mania_grid(
    beatmap: &Beatmap,
    output_path: &Path,
    mods: Option<&ModSettings>,
    time_axis: TimeAxis,
    deadline: &RequestDeadline,
) -> Result<PathBuf> {
    deadline.check()?;
    // 键数直接来自谱面 CS（模组不会改变原生 mania 轨道数）。
    let key_count = resolve_key_count(beatmap)?;
    let palette = super::lane_palette(key_count);

    let mut hit_objects = mania_objects(beatmap);
    if mods.is_some_and(|m| m.inverse) {
        hit_objects = apply_inverse_mod(&hit_objects, &beatmap.timing_points);
    }
    if mods.is_some_and(|m| m.hold_off) {
        hit_objects = apply_hold_off_mod(&hit_objects);
    }

    let cs_mode = mods.is_some_and(|m| m.cs_override);
    let native_mania = is_native_mania(beatmap);

    // 裁剪开头静音：若第一个音符在 5 秒之后，则从其前 1 秒开始，
    // 并对齐到红线节拍网格。
    let first_note_time = hit_objects
        .iter()
        .map(|ho| ho.start_time)
        .min()
        .unwrap_or(0);
    let chart_start_time = if first_note_time >= 5000 {
        crate::domain::shared::time_selection::snap_to_beat_grid(
            first_note_time - 1000,
            &beatmap.timing_points,
        )
    } else {
        0
    };

    if chart_start_time > 0 {
        for ho in &mut hit_objects {
            ho.start_time = (ho.start_time - chart_start_time).max(0);
            ho.end_time = (ho.end_time - chart_start_time).max(ho.start_time);
        }
    }

    let beatmap_duration = hit_objects.iter().map(|ho| ho.end_time).max().unwrap_or(0);
    let chart_end_time = beatmap_duration
        + crate::infrastructure::config::current()
            .render
            .mania
            .png
            .style
            .BOTTOM_PADDING_MS;
    let timing_points_for_render: Vec<TimingPoint> = if chart_start_time > 0 {
        beatmap
            .timing_points
            .iter()
            .map(|tp| {
                let mut tp = *tp;
                tp.time -= chart_start_time as f64;
                tp
            })
            .collect()
    } else {
        beatmap.timing_points.clone()
    };
    let timing_lines = build_timing_lines(
        &timing_points_for_render,
        chart_end_time,
        beatmap.beat_divisor,
        hit_objects
            .iter()
            .map(|ho| ho.start_time)
            .min()
            .unwrap_or(0),
    );
    let sv_changes = if cs_mode
        || !native_mania
        || !crate::infrastructure::config::current()
            .render
            .mania
            .png
            .style
            .SHOW_SV_LABEL
    {
        Vec::new()
    } else {
        build_sv_changes(&timing_points_for_render, chart_end_time)
    };
    let layout = build_png_layout(
        key_count,
        beatmap_duration,
        chart_end_time,
        chart_start_time,
    )?;
    deadline.check()?;

    let mut image = Img::new(
        layout.image_width as u32,
        layout.image_height as u32,
        crate::infrastructure::config::current()
            .render
            .mania
            .png
            .style
            .IMAGE_BACKGROUND,
    );

    for column_index in 0..layout.column_count {
        deadline.check()?;
        draw_column_background(&mut image, key_count, column_index, &layout);
    }
    let mut last_label_time: Option<i64> = None;
    for timing_line in &timing_lines {
        deadline.check()?;
        let mut tl = timing_line.clone();
        if tl.show_label {
            if let Some(prev) = last_label_time {
                if (tl.time - prev).abs()
                    < crate::infrastructure::config::current()
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
        draw_timing_line(&mut image, &tl, &layout, time_axis);
    }
    for (index, sv_change) in sv_changes.iter().enumerate() {
        if index % 1024 == 0 {
            deadline.check()?;
        }
        draw_sv_indicator(&mut image, *sv_change, &layout);
    }
    for (index, hit_object) in hit_objects.iter().enumerate() {
        if index % 1024 == 0 {
            deadline.check()?;
        }
        draw_png_hit_object(&mut image, hit_object, &palette, &layout);
    }

    save_png(&image, output_path, deadline)?;
    Ok(output_path.to_path_buf())
}

fn build_png_layout(
    key_count: i32,
    beatmap_duration: i64,
    chart_end_time: i64,
    chart_start_time: i64,
) -> Result<RenderLayout> {
    let skin_config =
        super::skin::load_mania_skin_config(key_count, crate::render::geometry::OutputFormat::Png);
    let output_scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Mania,
        crate::render::geometry::OutputFormat::Png,
    );
    let (lane_widths, lane_left_offsets, skin_lane_area_width) =
        super::animation::build_scaled_columns(
            &skin_config.column_widths,
            &skin_config.column_line_widths,
            output_scale,
        );
    let logical_pixels_per_ms = crate::infrastructure::config::current()
        .render
        .mania
        .png
        .sizing
        .PIXELS_PER_MS
        / output_scale;
    let logical_chart_height = (chart_end_time as f64 * logical_pixels_per_ms).ceil() as i64;
    let total_chart_height =
        crate::render::geometry::scale_px(logical_chart_height as f64, output_scale).max(1);
    let column_count = calculate_column_count(beatmap_duration, total_chart_height)?;
    let time_per_column = ceil_div(chart_end_time, column_count);
    let logical_column_height = (time_per_column as f64 * logical_pixels_per_ms).ceil() as i64;
    let column_height =
        crate::render::geometry::scale_px(logical_column_height as f64, output_scale).max(1);
    let top_buffer = crate::render::geometry::scale_px(
        crate::render::geometry::scale_px(
            crate::render::modes::mania::constants::TOP_BUFFER as f64,
            0.5,
        ) as f64,
        output_scale,
    );
    let total_column_height = top_buffer + column_height;
    let lane_area_width = skin_lane_area_width
        + (key_count as i64 - 1)
            * crate::infrastructure::config::current()
                .render
                .mania
                .png
                .sizing
                .LANE_GAP;
    let column_width = crate::infrastructure::config::current()
        .render
        .mania
        .png
        .sizing
        .LEFT_PANEL_WIDTH
        + lane_area_width;
    let image_width = crate::infrastructure::config::current()
        .render
        .mania
        .png
        .sizing
        .PAGE_MARGIN_LEFT
        + crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .PAGE_MARGIN_RIGHT
        + column_count
            * (crate::infrastructure::config::current()
                .render
                .mania
                .png
                .sizing
                .INFO_MARGIN_LEFT
                + column_width
                + crate::infrastructure::config::current()
                    .render
                    .mania
                    .png
                    .sizing
                    .INFO_MARGIN_RIGHT)
        + (column_count - 1)
            * crate::infrastructure::config::current()
                .render
                .mania
                .png
                .sizing
                .COLUMN_GAP;
    let image_height = crate::infrastructure::config::current()
        .render
        .mania
        .png
        .sizing
        .PAGE_MARGIN_TOP
        + crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .PAGE_MARGIN_BOTTOM
        + crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .INFO_MARGIN_TOP
        + total_column_height
        + crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .INFO_MARGIN_BOTTOM;
    Ok(RenderLayout {
        column_count,
        time_per_column,
        column_height,
        total_column_height,
        lane_area_width,
        column_width,
        lane_widths,
        lane_left_offsets,
        top_buffer,
        image_width,
        image_height,
        chart_start_time,
    })
}

fn png_column_left(column_index: i64, layout: &RenderLayout) -> i64 {
    let config = &crate::infrastructure::config::current().render.mania.png;
    config.sizing.PAGE_MARGIN_LEFT
        + config.sizing.INFO_MARGIN_LEFT
        + column_index
            * (config.sizing.INFO_MARGIN_LEFT
                + layout.column_width
                + config.sizing.INFO_MARGIN_RIGHT
                + config.sizing.COLUMN_GAP)
}

fn png_chart_top() -> i64 {
    crate::infrastructure::config::current()
        .render
        .mania
        .png
        .sizing
        .PAGE_MARGIN_TOP
        + crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .INFO_MARGIN_TOP
}

fn ceil_div(a: i64, b: i64) -> i64 {
    (a + b - 1).div_euclid(b)
}

fn calculate_column_count(beatmap_duration: i64, total_chart_height: i64) -> Result<i64> {
    if beatmap_duration
        >= crate::infrastructure::config::current()
            .render
            .mania
            .png
            .style
            .MAX_SUPPORTED_DURATION_MS
    {
        return Err(crate::domain::errors::PreviewError::render(
            "songs longer than 10 minutes are not supported",
        ));
    }
    if beatmap_duration >= 6 * 60 * 1000 {
        return Ok(crate::infrastructure::config::current()
            .render
            .mania
            .png
            .structure
            .FIXED_COLUMN_COUNT_6_TO_10_MINUTES);
    }
    let max_area_height = resolve_max_area_height(beatmap_duration);
    Ok(ceil_div(total_chart_height, max_area_height).max(1))
}

fn resolve_max_area_height(beatmap_duration: i64) -> i64 {
    if beatmap_duration < 60 * 1000 {
        crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .MAX_AREA_HEIGHT_0_TO_1_MINUTES
    } else if beatmap_duration < 2 * 60 * 1000 {
        crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .MAX_AREA_HEIGHT_1_TO_2_MINUTES
    } else if beatmap_duration < 3 * 60 * 1000 {
        crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .MAX_AREA_HEIGHT_2_TO_3_MINUTES
    } else if beatmap_duration < 4 * 60 * 1000 {
        crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .MAX_AREA_HEIGHT_3_TO_4_MINUTES
    } else if beatmap_duration < 5 * 60 * 1000 {
        crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .MAX_AREA_HEIGHT_4_TO_5_MINUTES
    } else {
        crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .MAX_AREA_HEIGHT_5_TO_6_MINUTES
    }
}

fn draw_column_background(
    image: &mut Img,
    key_count: i32,
    column_index: i64,
    layout: &RenderLayout,
) {
    let column_left = png_column_left(column_index, layout);
    let chart_top = png_chart_top();
    let lane_area_left = column_left
        + crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .LEFT_PANEL_WIDTH;

    image.set_rect_size(
        column_left,
        chart_top,
        crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .LEFT_PANEL_WIDTH,
        layout.total_column_height,
        crate::infrastructure::config::current()
            .render
            .mania
            .png
            .style
            .LEFT_PANEL_BACKGROUND,
    );

    for lane_index in 0..key_count as i64 {
        let lane_left = lane_area_left
            + layout.lane_left_offsets[lane_index as usize]
            + lane_index
                * crate::infrastructure::config::current()
                    .render
                    .mania
                    .png
                    .sizing
                    .LANE_GAP;
        let lane_right = lane_left + layout.lane_widths[lane_index as usize];
        image.set_rect_size(
            lane_left,
            chart_top,
            lane_right - lane_left,
            layout.total_column_height,
            crate::infrastructure::config::current()
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
                crate::infrastructure::config::current()
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
    layout: &RenderLayout,
    time_axis: TimeAxis,
) {
    let column_index =
        (timing_line.time.div_euclid(layout.time_per_column)).min(layout.column_count - 1);
    let local_time = timing_line.time - column_index * layout.time_per_column;
    let column_left = png_column_left(column_index, layout);
    let lane_area_left = column_left
        + crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .LEFT_PANEL_WIDTH;
    let chart_top = png_chart_top() + layout.top_buffer;
    let y = chart_top + layout.column_height
        - round_half_even(
            local_time as f64
                * crate::infrastructure::config::current()
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
            crate::infrastructure::config::current()
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
            crate::infrastructure::config::current()
                .render
                .mania
                .png
                .sizing
                .TIME_LABEL_FONT_SIZE,
            crate::infrastructure::config::current()
                .render
                .mania
                .png
                .style
                .RULER_TEXT_COLOR,
        );

        if let Some(ref bpm_label) = timing_line.bpm_label {
            let (_, bpm_h) = text_size(
                bpm_label,
                crate::infrastructure::config::current()
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
                crate::infrastructure::config::current()
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
                crate::infrastructure::config::current()
                    .render
                    .mania
                    .png
                    .sizing
                    .TIME_LABEL_FONT_SIZE,
                crate::infrastructure::config::current()
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
    layout: &RenderLayout,
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
            + crate::infrastructure::config::current()
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
            + lane as i64
                * crate::infrastructure::config::current()
                    .render
                    .mania
                    .png
                    .sizing
                    .LANE_GAP
            + crate::infrastructure::config::current()
                .render
                .mania
                .png
                .sizing
                .NOTE_SIDE_PADDING;
        let lane_right = lane_left + layout.lane_widths[lane]
            - crate::infrastructure::config::current()
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
                    * crate::infrastructure::config::current()
                        .render
                        .mania
                        .png
                        .sizing
                        .PIXELS_PER_MS,
            );
        let y_end = chart_axis_top + layout.column_height
            - round_half_even(
                (segment_end - column_index * layout.time_per_column) as f64
                    * crate::infrastructure::config::current()
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
                        - crate::infrastructure::config::current()
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
                        - crate::infrastructure::config::current()
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
                    - crate::infrastructure::config::current()
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

fn build_timing_lines(
    timing_points: &[TimingPoint],
    chart_end_time: i64,
    beat_divisor: i32,
    first_note_time: i64,
) -> Vec<TimingLine> {
    let base_points: Vec<&TimingPoint> = timing_points.iter().filter(|p| p.uninherited).collect();
    if base_points.is_empty() {
        return Vec::new();
    }

    let mut ordered_unique: BTreeMap<i64, TimingLine> = BTreeMap::new();
    for (index, point) in base_points.iter().enumerate() {
        let segment_end = if index + 1 < base_points.len() {
            base_points[index + 1].time.trunc() as i64 as f64
        } else {
            chart_end_time as f64
        };

        let beat_pixels = point.beat_length
            * crate::infrastructure::config::current()
                .render
                .mania
                .png
                .sizing
                .PIXELS_PER_MS;
        let subdivision: i64 = if beat_divisor > 0 {
            (beat_divisor as i64).max(1)
        } else if beat_pixels >= 72.0 {
            4
        } else if beat_pixels >= 28.0 {
            2
        } else {
            1
        };
        let step = point.beat_length / subdivision as f64;
        // NaN 步长直接继续（与 Python 一样只输出一条线）；零或负值另行处理。
        // 该步长会造成无限循环，因此跳过。
        if step <= 0.0 {
            continue;
        }
        let bar_modulo = (subdivision * point.meter as i64).max(1);
        let mut step_index: i64 = 0;
        let mut current = point.time;

        while current <= segment_end + 0.001 {
            if current >= 0.0 {
                let is_bar = step_index % bar_modulo == 0;
                let is_beat = step_index % subdivision == 0;
                ordered_unique.insert(
                    round_half_even(current),
                    TimingLine {
                        time: round_half_even(current),
                        color: if is_bar {
                            crate::infrastructure::config::current()
                                .render
                                .mania
                                .png
                                .style
                                .MEASURE_LINE_COLOR
                        } else if is_beat {
                            crate::infrastructure::config::current()
                                .render
                                .mania
                                .png
                                .style
                                .BEAT_LINE_COLOR
                        } else {
                            crate::infrastructure::config::current()
                                .render
                                .mania
                                .png
                                .style
                                .SUBDIVISION_LINE
                        },
                        show_label: is_bar || is_beat,
                        bpm_label: None,
                    },
                );
            }
            step_index += 1;
            current = point.time + step_index as f64 * step;
        }
    }
    // 添加 BPM 标签：BPM 变化时标在每条红线的第一条小节线，
    // 并在最接近首个音符的小节线上标记。
    if !ordered_unique.is_empty() {
        let mut last_bpm: Option<f64> = None;
        for point in &base_points {
            let bpm = 60_000.0 / point.beat_length;
            let bpm_changed = last_bpm.is_none_or(|prev| (bpm - prev).abs() > 0.01);
            last_bpm = Some(bpm);

            if bpm_changed {
                // 查找该红线时间点或之后的第一条小节线。
                let rounded = round_half_even(point.time);
                let key = ordered_unique
                    .range(rounded..)
                    .next()
                    .map(|(&k, _)| k)
                    .unwrap_or(rounded);
                if let Some(line) = ordered_unique.get_mut(&key) {
                    line.bpm_label = Some(format!("{:.0}BPM", bpm.round()));
                }
            }
        }

        // 首个音符 BPM：使用 first_note_time 时生效的 BPM。
        if first_note_time > 0 {
            let bpm = 60_000.0
                / base_points
                    .iter()
                    .rfind(|p| p.time <= first_note_time as f64)
                    .map_or(base_points[0].beat_length, |p| p.beat_length);
            let key = ordered_unique
                .range(first_note_time..)
                .next()
                .map(|(&k, _)| k);
            if let Some(k) = key {
                if let Some(line) = ordered_unique.get_mut(&k) {
                    // 仅当同一时刻尚未因 BPM 变化添加标签时才附加。
                    if line.bpm_label.is_none() {
                        line.bpm_label = Some(format!("{:.0}BPM", bpm.round()));
                    }
                }
            }
        }
    }

    ordered_unique.into_values().collect()
}

fn draw_sv_indicator(image: &mut Img, sv_change: (i64, f64), layout: &RenderLayout) {
    let (time, sv) = sv_change;
    let column_index = (time.div_euclid(layout.time_per_column)).min(layout.column_count - 1);
    let local_time = time - column_index * layout.time_per_column;
    let column_left = png_column_left(column_index, layout);
    let chart_top = png_chart_top() + layout.top_buffer;
    let y = chart_top + layout.column_height
        - round_half_even(
            local_time as f64
                * crate::infrastructure::config::current()
                    .render
                    .mania
                    .png
                    .sizing
                    .PIXELS_PER_MS,
        );

    let label = super::format_sv_label(sv);
    let (label_width, label_height) = text_size(
        &label,
        crate::infrastructure::config::current()
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
        crate::infrastructure::config::current()
            .render
            .mania
            .png
            .sizing
            .SV_TEXT_FONT_SIZE,
        crate::infrastructure::config::current()
            .render
            .mania
            .png
            .style
            .SV_TEXT_COLOR,
    );
}
