//! osu!catch PNG 场景绘制：根据 CLI 传入的布局和渲染对象绘制静态图。

use crate::processing::timeline::TimeAxis;
use crate::render::canvas::Img;
use crate::render::text::{draw_text, text_size};
use std::collections::BTreeMap;

use super::drawing::draw_catch_object;
use super::objects::{object_order, rhe, ObjType, RenderObject};

pub struct CatchPngLayout {
    pub column_count: i64,
    pub total_column_height: i64,
    pub visible_playfield_width: i64,
    pub image_width: i64,
    pub image_height: i64,
    pub playfield_scale: f64,
    pub object_scale: f64,
    pub pixels_per_ms: f64,
    pub chart_start_time: i64,
}

fn column_left(column_index: i64) -> i64 {
    let config = &crate::config::current().render.catch.png;
    config.sizing.PAGE_MARGIN_LEFT
        + config.sizing.INFO_MARGIN_LEFT
        + column_index
            * (config.sizing.INFO_MARGIN_LEFT
                + config.sizing.COLUMN_WIDTH
                + config.sizing.INFO_MARGIN_RIGHT
                + config.sizing.COLUMN_GAP)
}

fn playfield_left(column_index: i64) -> i64 {
    column_left(column_index)
        + crate::config::current()
            .render
            .catch
            .png
            .sizing
            .LEFT_PANEL_WIDTH
        + playfield_side_padding()
}

fn playfield_side_padding() -> i64 {
    let config = &crate::config::current().render.catch.png;
    let render_scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Catch,
        crate::render::geometry::OutputFormat::Png,
    );
    let playfield_width = crate::render::geometry::scale_px(
        super::constants::PLAYFIELD_DISPLAY_WIDTH as f64,
        render_scale,
    );
    playfield_side_padding_for(
        config.sizing.COLUMN_WIDTH,
        config.sizing.LEFT_PANEL_WIDTH,
        playfield_width,
    )
}

fn playfield_side_padding_for(
    column_width: i64,
    left_panel_width: i64,
    playfield_width: i64,
) -> i64 {
    (column_width - left_panel_width - playfield_width)
        .div_euclid(2)
        .max(0)
}

#[derive(Clone, Copy)]
pub struct TimingLine {
    pub time: i64,
    pub is_measure: bool,
    pub show_label: bool,
    pub bpm: Option<f64>,
}

/// 红线分段：每段持有固定的 beat_length 与 meter。
pub struct RedlineSection {
    pub start_time: f64,
    pub end_time: f64,
    pub beat_length: f64,
    pub meter: i32,
}

/// 从红线（uninherited timing point）构建分段，再按段内节拍生成节拍线。

fn draw_column_background(image: &mut Img, layout: &CatchPngLayout, column_index: i64) {
    let column_left = column_left(column_index);
    let chart_top = crate::config::current()
        .render
        .catch
        .png
        .sizing
        .PAGE_MARGIN_TOP
        + crate::config::current()
            .render
            .catch
            .png
            .sizing
            .INFO_MARGIN_TOP;
    let panel_width = crate::config::current()
        .render
        .catch
        .png
        .sizing
        .LEFT_PANEL_WIDTH;
    let side_padding = playfield_side_padding();
    let visible_left = column_left + panel_width + side_padding;
    let visible_right = visible_left + layout.visible_playfield_width;
    let border_width = crate::render::geometry::scale_stroke_px(
        1.0,
        crate::render::geometry::output_scale(
            crate::render::geometry::GameMode::Catch,
            crate::render::geometry::OutputFormat::Png,
        ),
    );
    let border_left = visible_left - side_padding;
    let border_right = visible_right + side_padding - border_width;

    image.set_rect_size(
        column_left,
        chart_top,
        panel_width,
        layout.total_column_height,
        crate::config::current()
            .render
            .catch
            .png
            .style
            .LEFT_PANEL_BACKGROUND,
    );
    image.set_rect_size(
        visible_left,
        chart_top,
        layout.visible_playfield_width,
        layout.total_column_height,
        crate::config::current()
            .render
            .catch
            .png
            .style
            .PLAYFIELD_BACKGROUND,
    );
    image.set_rect_size(
        border_left,
        chart_top,
        border_width,
        layout.total_column_height,
        crate::config::current()
            .render
            .catch
            .png
            .style
            .PLAYFIELD_BORDER,
    );
    image.set_rect_size(
        border_right,
        chart_top,
        border_width,
        layout.total_column_height,
        crate::config::current()
            .render
            .catch
            .png
            .style
            .PLAYFIELD_BORDER,
    );
}

/// 时间 → （列号, y 坐标）。时间从列底部向上递增（与游戏内下落方向一致）。
fn locate_time(time: i64, layout: &CatchPngLayout) -> (i64, i64) {
    let absolute_y = time as f64 * layout.pixels_per_ms;
    let column_index = ((absolute_y / layout.total_column_height as f64).floor() as i64)
        .clamp(0, layout.column_count - 1);
    let local_y_from_top = rhe(absolute_y - (column_index * layout.total_column_height) as f64);
    // 从列底部开始计算，时间 0 在底部，时间增大向上
    let chart_bottom = crate::config::current()
        .render
        .catch
        .png
        .sizing
        .PAGE_MARGIN_TOP
        + crate::config::current()
            .render
            .catch
            .png
            .sizing
            .INFO_MARGIN_TOP
        + layout.total_column_height;
    let y = chart_bottom - local_y_from_top;
    (column_index, y)
}

fn draw_timing_line_png(image: &mut Img, timing_line: &TimingLine, layout: &CatchPngLayout) {
    let (column_index, y) = locate_time(timing_line.time, layout);
    let left = playfield_left(column_index);
    let right = left + layout.visible_playfield_width;
    let y = y.clamp(
        crate::config::current()
            .render
            .catch
            .png
            .sizing
            .PAGE_MARGIN_TOP
            + crate::config::current()
                .render
                .catch
                .png
                .sizing
                .INFO_MARGIN_TOP,
        crate::config::current()
            .render
            .catch
            .png
            .sizing
            .PAGE_MARGIN_TOP
            + crate::config::current()
                .render
                .catch
                .png
                .sizing
                .INFO_MARGIN_TOP
            + layout.total_column_height,
    );

    let render_scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Catch,
        crate::render::geometry::OutputFormat::Png,
    );
    let (thickness, color) = if timing_line.is_measure {
        (
            crate::render::geometry::scale_stroke_px(2.0, render_scale),
            crate::config::current()
                .render
                .catch
                .png
                .style
                .MEASURE_LINE_COLOR,
        )
    } else {
        (
            crate::render::geometry::scale_stroke_px(1.0, render_scale),
            crate::config::current()
                .render
                .catch
                .png
                .style
                .BEAT_LINE_COLOR,
        )
    };
    image.set_rect_size(left, y, right - left, thickness, color);
}

fn draw_timing_label_png(
    image: &mut Img,
    timing_line: &TimingLine,
    layout: &CatchPngLayout,
    time_axis: TimeAxis,
) {
    let (column_index, y) = locate_time(timing_line.time, layout);
    let border_right = column_left(column_index)
        + crate::config::current()
            .render
            .catch
            .png
            .sizing
            .COLUMN_WIDTH;
    let y = y.clamp(
        crate::config::current()
            .render
            .catch
            .png
            .sizing
            .PAGE_MARGIN_TOP
            + crate::config::current()
                .render
                .catch
                .png
                .sizing
                .INFO_MARGIN_TOP,
        crate::config::current()
            .render
            .catch
            .png
            .sizing
            .PAGE_MARGIN_TOP
            + crate::config::current()
                .render
                .catch
                .png
                .sizing
                .INFO_MARGIN_TOP
            + layout.total_column_height,
    );
    let label = crate::render::text::format_seconds_tenths(
        time_axis.to_display(timing_line.time + layout.chart_start_time),
    );
    let (label_width, label_height) = text_size(
        &label,
        crate::config::current()
            .render
            .catch
            .png
            .sizing
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
            - crate::config::current()
                .render
                .catch
                .png
                .sizing
                .PAGE_MARGIN_LEFT,
    );
    let label_y = (y as f64 - label_height as f64 / 2.0).floor() as i64;
    let bpm_label = timing_line.bpm.map(crate::render::timing::format_bpm);
    let bpm_height = bpm_label.as_ref().map_or(0, |text| {
        text_size(
            text,
            crate::config::current()
                .render
                .catch
                .png
                .sizing
                .TIME_LABEL_FONT_SIZE,
        )
        .1 as i64
            + crate::config::current()
                .render
                .catch
                .png
                .sizing
                .BPM_LABEL_GAP
    });
    let group_height = label_height as i64 + bpm_height;
    let chart_top = crate::config::current()
        .render
        .catch
        .png
        .sizing
        .PAGE_MARGIN_TOP
        + crate::config::current()
            .render
            .catch
            .png
            .sizing
            .INFO_MARGIN_TOP;
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
            .render
            .catch
            .png
            .sizing
            .TIME_LABEL_FONT_SIZE,
        crate::config::current()
            .render
            .catch
            .png
            .style
            .TIME_LABEL_COLOR,
    );
    if let Some(bpm_label) = bpm_label {
        let (bpm_width, _) = text_size(
            &bpm_label,
            crate::config::current()
                .render
                .catch
                .png
                .sizing
                .TIME_LABEL_FONT_SIZE,
        );
        let bpm_x = (border_right + label_gap).min(
            layout.image_width
                - bpm_width as i64
                - crate::config::current()
                    .render
                    .catch
                    .png
                    .sizing
                    .PAGE_MARGIN_LEFT,
        );
        draw_text(
            image,
            bpm_x,
            label_y
                + label_height as i64
                + crate::config::current()
                    .render
                    .catch
                    .png
                    .sizing
                    .BPM_LABEL_GAP,
            &bpm_label,
            crate::config::current()
                .render
                .catch
                .png
                .sizing
                .TIME_LABEL_FONT_SIZE,
            crate::config::current()
                .render
                .catch
                .png
                .style
                .BPM_LABEL_COLOR,
        );
    }
}

fn draw_catch_object_png(image: &mut Img, catch_object: &RenderObject, layout: &CatchPngLayout) {
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
    layout: &CatchPngLayout,
) -> Vec<LineSegment> {
    time_axis_segments(
        current.event_time_or_start(),
        current.x,
        next.event_time_or_start(),
        next.x,
        layout,
    )
}

fn time_axis_segments(
    start_time: f64,
    start_x: f64,
    end_time: f64,
    end_x: f64,
    layout: &CatchPngLayout,
) -> Vec<LineSegment> {
    if end_time <= start_time || layout.pixels_per_ms <= 0.0 {
        return Vec::new();
    }

    let column_duration = layout.total_column_height as f64 / layout.pixels_per_ms;
    let start_column = (start_time / column_duration).floor() as i64;
    let end_column = (end_time / column_duration).floor() as i64;
    let chart_bottom = (crate::config::current()
        .render
        .catch
        .png
        .sizing
        .PAGE_MARGIN_TOP
        + crate::config::current()
            .render
            .catch
            .png
            .sizing
            .INFO_MARGIN_TOP
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
            let object_x = start_x + (end_x - start_x) * progress;
            let x = playfield_left(column) as f64 + object_x * layout.playfield_scale;
            let local_height = (time - column_start) * layout.pixels_per_ms;
            let y = chart_bottom - local_height;
            (x, y)
        };
        segments.push((point_at(segment_start), point_at(segment_end)));
    }

    segments
}

/// 绘制动态规划实际采用的接盘中心轨迹；未接取的香蕉时刻也保留过渡位置。
fn draw_banana_routes(image: &mut Img, render_objects: &[RenderObject], layout: &CatchPngLayout) {
    let mut previous: Option<(usize, f64, f64)> = None;

    for current in render_objects {
        let (Some(shower_id), Some(current_x)) = (current.banana_shower_id, current.banana_route_x)
        else {
            continue;
        };

        if let Some((previous_shower_id, previous_time, previous_x)) = previous {
            if previous_shower_id == shower_id {
                for (start, end) in time_axis_segments(
                    previous_time,
                    previous_x,
                    current.event_time_or_start(),
                    current_x,
                    layout,
                ) {
                    image.draw_line(
                        start.0,
                        start.1,
                        end.0,
                        end.1,
                        (super::constants::BANANA_ROUTE_LINE_WIDTH * layout.playfield_scale)
                            .max(1.0),
                        super::constants::BANANA_ROUTE_LINE_COLOR,
                    );
                }
            }
        }
        previous = Some((shower_id, current.event_time_or_start(), current_x));
    }
}

fn draw_edge_guides(image: &mut Img, render_objects: &[RenderObject], layout: &CatchPngLayout) {
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
                crate::config::current()
                    .render
                    .catch
                    .png
                    .sizing
                    .EDGE_GUIDE_WIDTH,
                crate::config::current()
                    .render
                    .catch
                    .png
                    .style
                    .EDGE_GUIDE_COLOR,
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

fn draw_edge_combo_labels(
    image: &mut Img,
    render_objects: &[RenderObject],
    layout: &CatchPngLayout,
) {
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

        let config = &crate::config::current().render.catch.png;
        let label = format!("{combo}x");
        let (label_width, label_height) =
            text_size(&label, config.sizing.EDGE_COMBO_LABEL_FONT_SIZE);
        let (column_index, center_y) = locate_time(current.start_time, layout);
        let center_x = playfield_left(column_index) as f64 + current.x * layout.playfield_scale;
        let radius = super::drawing::object_diameter(
            layout.object_scale,
            layout.playfield_scale,
            current.scale_factor,
        ) / 2.0;
        let left_x =
            rhe(center_x - radius - config.sizing.EDGE_COMBO_LABEL_GAP - label_width as f64);
        let right_x = rhe(center_x + radius + config.sizing.EDGE_COMBO_LABEL_GAP);
        let min_x = playfield_left(column_index);
        let max_x = min_x + layout.visible_playfield_width - label_width as i64;
        let prefer_left = next.x >= current.x;
        let label_x = if prefer_left && left_x >= min_x {
            left_x
        } else if right_x <= max_x {
            right_x
        } else {
            left_x.max(min_x)
        };
        let chart_top = config.sizing.PAGE_MARGIN_TOP + config.sizing.INFO_MARGIN_TOP;
        let chart_bottom = chart_top + layout.total_column_height;
        let label_y = (center_y - label_height as i64 / 2)
            .max(chart_top)
            .min(chart_bottom - label_height as i64);

        // 深色底板可避免边界迫使标签与白色引导线同侧时难以辨认。
        image.fill_rect_size(
            label_x - config.sizing.EDGE_COMBO_LABEL_PADDING,
            label_y - config.sizing.EDGE_COMBO_LABEL_PADDING,
            label_width as i64 + config.sizing.EDGE_COMBO_LABEL_PADDING * 2,
            label_height as i64 + config.sizing.EDGE_COMBO_LABEL_PADDING * 2,
            config.style.EDGE_COMBO_LABEL_BACKGROUND,
        );
        draw_text(
            image,
            label_x + config.sizing.EDGE_COMBO_LABEL_SHADOW_GAP,
            label_y + config.sizing.EDGE_COMBO_LABEL_SHADOW_GAP,
            &label,
            config.sizing.EDGE_COMBO_LABEL_FONT_SIZE,
            config.style.EDGE_COMBO_LABEL_SHADOW,
        );
        draw_text(
            image,
            label_x,
            label_y,
            &label,
            config.sizing.EDGE_COMBO_LABEL_FONT_SIZE,
            config.style.EDGE_COMBO_LABEL_COLOR,
        );
    }
}

pub fn ceil_div(a: i64, b: i64) -> i64 {
    (a + b - 1) / b
}

pub fn predominant_measure_aligned_height(
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

/// 按 `CatchPngLayout` 与预计算对象绘制整张 catch 静态图。
pub fn render_catch_png_scene(
    layout: &CatchPngLayout,
    render_objects: &[RenderObject],
    timing_lines: &[TimingLine],
    time_axis: TimeAxis,
) -> Img {
    let mut image = Img::new(
        layout.image_width as u32,
        layout.image_height as u32,
        crate::config::current()
            .render
            .catch
            .png
            .style
            .IMAGE_BACKGROUND,
    );

    for column_index in 0..layout.column_count {
        draw_column_background(&mut image, layout, column_index);
    }

    let mut last_label_time: Option<i64> = None;
    for timing_line in timing_lines {
        let mut tl = *timing_line;
        if tl.show_label {
            if let Some(prev) = last_label_time {
                if (tl.time - prev).abs()
                    < crate::config::current()
                        .render
                        .catch
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
        draw_timing_line_png(&mut image, &tl, layout);
        if tl.show_label || tl.bpm.is_some() {
            draw_timing_label_png(&mut image, &tl, layout, time_axis);
        }
    }

    if crate::config::current()
        .render
        .catch
        .png
        .style
        .SHOW_BANANA_ROUTE
    {
        draw_banana_routes(&mut image, render_objects, layout);
    }
    draw_edge_guides(&mut image, render_objects, layout);

    let mut sorted_objects: Vec<&RenderObject> = render_objects.iter().collect();
    sorted_objects.sort_by_key(|o| (-o.start_time, object_order(o.object_type)));
    for catch_object in sorted_objects {
        draw_catch_object_png(&mut image, catch_object, layout);
    }

    draw_edge_combo_labels(&mut image, render_objects, layout);

    image
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_layout(column_count: i64) -> CatchPngLayout {
        CatchPngLayout {
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
            color: crate::render::cpu::modes::catch::constants::LAZER_COMBO_COLORS[0],
            scale_factor: 1.0,
            event_time: Some(time as f64),
            hyper_dash: false,
            edge: true,
            banana_shower_id: None,
            banana_route_x: None,
        }
    }

    fn route_banana(x: f64, route_x: f64, time: i64, shower_id: usize) -> RenderObject {
        let mut object = edge_fruit(x, time);
        object.object_type = ObjType::Banana;
        object.edge = false;
        object.banana_shower_id = Some(shower_id);
        object.banana_route_x = Some(route_x);
        object
    }

    #[test]
    fn edge_guide_is_split_at_column_boundary() {
        let current = edge_fruit(0.0, 90);
        let next = edge_fruit(200.0, 110);

        let segments = edge_guide_segments(&current, &next, &test_layout(2));

        assert_eq!(segments.len(), 2);
        let chart_top = (crate::config::current()
            .render
            .catch
            .png
            .sizing
            .PAGE_MARGIN_TOP
            + crate::config::current()
                .render
                .catch
                .png
                .sizing
                .INFO_MARGIN_TOP) as f64;
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
        let chart_bottom = crate::config::current()
            .render
            .catch
            .png
            .sizing
            .PAGE_MARGIN_TOP
            + crate::config::current()
                .render
                .catch
                .png
                .sizing
                .INFO_MARGIN_TOP
            + layout.total_column_height;
        assert_eq!(
            image.get(midpoint_x as u32, (chart_bottom - 15) as u32),
            crate::config::current()
                .render
                .catch
                .png
                .style
                .EDGE_GUIDE_COLOR
        );
    }

    #[test]
    fn banana_route_draws_catcher_center_instead_of_banana_centers() {
        let layout = test_layout(1);
        let current = route_banana(0.0, 100.0, 10, 0);
        let next = route_banana(400.0, 100.0, 20, 0);
        let background = [7, 7, 7, 255];
        let mut image = Img::new(400, 130, background);

        draw_banana_routes(&mut image, &[current, next], &layout);

        let route_x = playfield_left(0) + 50;
        let banana_midpoint_x = playfield_left(0) + 100;
        let chart_bottom = crate::config::current()
            .render
            .catch
            .png
            .sizing
            .PAGE_MARGIN_TOP
            + crate::config::current()
                .render
                .catch
                .png
                .sizing
                .INFO_MARGIN_TOP
            + layout.total_column_height;
        assert_eq!(
            image.get(route_x as u32, (chart_bottom - 15) as u32),
            crate::render::cpu::modes::catch::constants::BANANA_ROUTE_LINE_COLOR
        );
        assert_eq!(
            image.get(banana_midpoint_x as u32, (chart_bottom - 15) as u32),
            background
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
    fn derived_playfield_padding_scales_with_the_column() {
        for scale in [0.5, 1.0, 1.5, 2.0] {
            let column_width = crate::render::geometry::scale_px(315.0, scale);
            let panel_width = crate::render::geometry::scale_px(9.0, scale);
            let playfield_width = crate::render::geometry::scale_px(260.0, scale);
            let padding = playfield_side_padding_for(column_width, panel_width, playfield_width);

            assert!((padding as f64 / scale - 23.0).abs() <= 1.0);
        }
    }

    #[test]
    fn aligned_column_count_is_stable_across_output_scales() {
        let timing_lines: Vec<TimingLine> = (0..10)
            .map(|index| TimingLine {
                time: index * 2_000,
                is_measure: true,
                show_label: true,
                bpm: None,
            })
            .collect();

        for scale in [0.5, 1.0, 1.5, 2.0] {
            let pixels_per_ms = 0.5 * scale;
            let max_area_height = crate::render::geometry::scale_px(5_500.0, scale);
            let aligned =
                predominant_measure_aligned_height(&timing_lines, pixels_per_ms, max_area_height)
                    .unwrap();
            let total_height = crate::render::geometry::scale_px(9_000.0, scale);

            assert_eq!(ceil_div(total_height, aligned), 2);
            assert!((aligned as f64 / scale - 5_000.0).abs() <= 1.0);
        }
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
            crate::config::current()
                .render
                .catch
                .png
                .style
                .IMAGE_BACKGROUND,
        );

        draw_edge_combo_labels(&mut image, &[current, next], &layout);

        let has_white_label_pixel = (0..image.h).any(|y| {
            (0..image.w).any(|x| {
                image.get(x, y)
                    == crate::config::current()
                        .render
                        .catch
                        .png
                        .style
                        .EDGE_COMBO_LABEL_COLOR
            })
        });
        assert!(has_white_label_pixel);
    }
}
