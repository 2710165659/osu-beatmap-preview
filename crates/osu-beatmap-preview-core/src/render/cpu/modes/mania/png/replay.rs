//! Mania 静态回放：按列绘制真实按键时长、分级判定背景及独立成绩快照。

use super::*;
use crate::{
    gameplay::{
        events::{EventKind, GameplayEvent, HitResult},
        timeline::GameplayTimeline,
    },
    render::{
        geometry::{output_scale, scale_px, GameMode, OutputFormat},
        shared::replay::{colors, static_state},
    },
    replay::preparation::mania::PreparedMania,
};

const INPUT: Rgba = [252, 97, 143, 255];
const OUTLINE: Rgba = [20, 8, 14, 255];

fn scale() -> f64 {
    output_scale(GameMode::Mania, OutputFormat::Png)
}
fn px(value: f64) -> i64 {
    scale_px(value, scale()).max(1)
}

fn locate(time: f64, layout: &ManiaPngLayout) -> (i64, i64) {
    let local = (time - layout.chart_start_time as f64).max(0.0);
    let column =
        ((local / layout.time_per_column as f64).floor() as i64).clamp(0, layout.column_count - 1);
    (column, y_at(local, column, layout))
}

fn y_at(local: f64, column: i64, layout: &ManiaPngLayout) -> i64 {
    png_chart_top() + layout.top_buffer + layout.column_height
        - round_half_even(
            (local - (column * layout.time_per_column) as f64)
                * crate::config::current()
                    .render
                    .mania
                    .png
                    .sizing
                    .PIXELS_PER_MS,
        )
}

fn lane_bounds(column: i64, lane: usize, layout: &ManiaPngLayout) -> (i64, i64) {
    let sizing = mania_constants::png::sizing();
    let left = png_column_left(column, layout)
        + sizing.LEFT_PANEL_WIDTH
        + layout.lane_left_offsets[lane]
        + lane as i64 * sizing.LANE_GAP;
    (left, left + layout.lane_widths[lane])
}

fn result_color(result: HitResult) -> Option<Rgba> {
    match result {
        HitResult::Great
        | HitResult::Good
        | HitResult::Ok
        | HitResult::Meh
        | HitResult::Miss
        | HitResult::IgnoreMiss => Some(colors::hit_result(result)),
        _ => None,
    }
}

fn judgements<'a>(
    prepared: &'a PreparedMania,
    gameplay: &'a GameplayTimeline,
    layout: &'a ManiaPngLayout,
    end: i64,
) -> impl Iterator<Item = (&'a ManiaHitObject, &'a GameplayEvent, f64)> {
    gameplay.events.iter().filter_map(move |event| {
        if event.kind != EventKind::Judgement || event.time > gameplay.settlement_time {
            return None;
        }
        let object = prepared.objects.get(event.object_id)?;
        // 超时判定仍标在原 note 上；长条中断使用实际松手时间，头尾分别对应各自节点。
        let time = match event.node_id {
            Some(0) => object.end_time as f64,
            Some(_) => event.time,
            None => object.start_time as f64,
        };
        (time >= layout.chart_start_time as f64 && time <= end as f64)
            .then_some((object, event, time))
    })
}

#[allow(clippy::too_many_arguments)]
pub fn render_scene(
    layout: &ManiaPngLayout,
    palette: &[Rgba],
    hit_objects: &[ManiaHitObject],
    timing_lines: &[TimingLine],
    sv_changes: &[(i64, f64)],
    time_axis: TimeAxis,
    prepared: &PreparedMania,
    gameplay: &GameplayTimeline,
    end: i64,
) -> Img {
    let mut image = render_background(layout, prepared.keys, timing_lines, sv_changes, time_axis);
    draw_bands(&mut image, layout, prepared, gameplay, end);
    for object in hit_objects {
        draw_png_hit_object(&mut image, object, palette, layout);
    }
    draw_states(&mut image, layout, gameplay, end, time_axis);
    draw_judgements(&mut image, layout, prepared, gameplay, end);
    draw_inputs(&mut image, layout, gameplay, end);
    draw_timing_signs(&mut image, layout, prepared, gameplay, end, sv_changes);
    image
}

fn draw_bands(
    image: &mut Img,
    layout: &ManiaPngLayout,
    prepared: &PreparedMania,
    gameplay: &GameplayTimeline,
    end: i64,
) {
    // 同一分列共用覆盖位图；同类判定去重，不同判定保留透明叠色。
    let top = png_chart_top().max(0);
    let bottom = (png_chart_top() + layout.total_column_height).min(i64::from(image.h));
    let height = (bottom - top).max(0) as usize;
    if height == 0 {
        return;
    }
    let mut coverage = vec![0_u8; height * layout.column_count as usize];
    let radius = px(7.0);
    for (_, event, time) in judgements(prepared, gameplay, layout, end) {
        let flag = match event.result {
            HitResult::Good => 1,
            HitResult::Ok => 2,
            HitResult::Meh => 4,
            HitResult::Miss => 8,
            HitResult::IgnoreMiss => 16,
            _ => continue,
        };
        let (column, y) = locate(time, layout);
        let index = column as usize * height;
        let from = (y - radius).clamp(top, bottom) - top;
        let to = (y + radius + 1).clamp(top, bottom) - top;
        for value in &mut coverage[index + from as usize..index + to as usize] {
            *value |= flag;
        }
    }
    let sizing = mania_constants::png::sizing();
    for column in 0..layout.column_count {
        let left = png_column_left(column, layout) + sizing.LEFT_PANEL_WIDTH;
        let index = column as usize * height;
        let rows = &coverage[index..index + height];
        // 相邻覆盖相同的行合成一个矩形，避免逐行重复进行混色和边界检查。
        let mut from = 0;
        while from < height {
            let flags = rows[from];
            let mut to = from + 1;
            while to < height && rows[to] == flags {
                to += 1;
            }
            // 与 Catch 一致，从 Miss 开始叠色；左侧实色条按相同顺序横向分区。
            let grades = [
                (8, HitResult::Miss),
                (16, HitResult::IgnoreMiss),
                (4, HitResult::Meh),
                (2, HitResult::Ok),
                (1, HitResult::Good),
            ];
            let count = flags.count_ones() as i64;
            let panel_left = png_column_left(column, layout);
            let panel_width = sizing.LEFT_PANEL_WIDTH.max(1);
            let mut grade_index = 0;
            for (flag, result) in grades {
                if flags & flag != 0 {
                    let color = result_color(result).unwrap();
                    let panel_from = panel_width * grade_index / count;
                    let panel_to = panel_width * (grade_index + 1) / count;
                    image.fill_rect_size(
                        panel_left + panel_from,
                        top + from as i64,
                        panel_to - panel_from,
                        (to - from) as i64,
                        color,
                    );
                    grade_index += 1;
                    let background = colors::with_alpha(color, 72);
                    image.fill_rect_size(
                        left,
                        top + from as i64,
                        layout.lane_area_width,
                        (to - from) as i64,
                        background,
                    );
                }
            }
            from = to;
        }
    }
}

fn draw_inputs(image: &mut Img, layout: &ManiaPngLayout, gameplay: &GameplayTimeline, end: i64) {
    let end = (end as f64).min(gameplay.settlement_time);
    let mut pressed = vec![None; layout.lane_widths.len()];
    let mut previous = 0;
    for frame in gameplay
        .input
        .frames
        .iter()
        .take_while(|f| f.time_ms as f64 <= end)
    {
        for (lane, start) in pressed.iter_mut().enumerate() {
            let bit = 1 << lane;
            if previous & bit == 0 && frame.keys & bit != 0 {
                *start = Some(frame.time_ms as f64);
            } else if previous & bit != 0 && frame.keys & bit == 0 {
                if let Some(from) = start.take() {
                    draw_press(image, layout, lane, from, frame.time_ms as f64, end);
                }
            }
        }
        previous = frame.keys;
    }
    for (lane, start) in pressed.into_iter().enumerate() {
        if let Some(from) = start {
            draw_press(image, layout, lane, from, end, end);
        }
    }
}

fn draw_press(
    image: &mut Img,
    layout: &ManiaPngLayout,
    lane: usize,
    start: f64,
    stop: f64,
    end: f64,
) {
    let origin = layout.chart_start_time as f64;
    if stop < origin || start > end {
        return;
    }
    let from = start.max(origin) - origin;
    let to = stop.min(end) - origin;
    let first = locate(from + origin, layout).0;
    let last = locate(to + origin, layout).0;
    for column in first..=last {
        let (left, right) = lane_bounds(column, lane, layout);
        let center = (left + right) / 2;
        let a = from.max((column * layout.time_per_column) as f64);
        let b = to.min(((column + 1) * layout.time_per_column) as f64);
        if b > a {
            let top = y_at(b, column, layout);
            let bottom = y_at(a, column, layout);
            image.fill_rect_size(center - px(2.0), top, px(4.0), bottom - top, OUTLINE);
            image.fill_rect_size(center - px(1.0), top, px(2.0), bottom - top, INPUT);
        }
    }
    // 选段中已按住的列只保留拖尾，不伪造窗口起点的按下边沿。
    if start >= origin {
        let (column, y) = locate(start, layout);
        let (left, right) = lane_bounds(column, lane, layout);
        let center = (left + right) / 2;
        let half = ((right - left) / 3).max(1);
        image.fill_rect_size(
            center - half - px(1.0),
            y - px(2.0),
            half * 2 + px(2.0),
            px(5.0),
            OUTLINE,
        );
        image.fill_rect_size(center - half, y - px(1.0), half * 2, px(2.0), INPUT);
    }
}

fn draw_judgements(
    image: &mut Img,
    layout: &ManiaPngLayout,
    prepared: &PreparedMania,
    gameplay: &GameplayTimeline,
    end: i64,
) {
    for (object, event, time) in judgements(prepared, gameplay, layout, end) {
        let Some(color) = result_color(event.result) else {
            continue;
        };
        let (column, y) = locate(time, layout);
        let (left, right) = lane_bounds(column, object.lane as usize, layout);
        let padding = mania_constants::png::sizing().NOTE_SIDE_PADDING;
        let width = (right - left - padding * 2).max(1);
        let height = mania_constants::png::sizing().NOTE_HEAD_HEIGHT.max(px(3.0));
        // 判定外框位于背景上方，真实按键随后叠在最上层。
        image.fill_rect_size(left + padding, y - height, width, px(2.0), color);
        image.fill_rect_size(left + padding, y - px(2.0), width, px(2.0), color);
        image.fill_rect_size(left + padding, y - height, px(2.0), height, color);
        image.fill_rect_size(
            right - padding - px(2.0),
            y - height,
            px(2.0),
            height,
            color,
        );
    }
}

fn timing_sign(event: &GameplayEvent) -> &'static str {
    // 自动超时与长条中断没有击打误差，不能用判定触发时刻反推出“晚”。
    match event.error {
        Some(error) if error < 0.0 => "+",
        Some(error) if error > 0.0 => "-",
        _ => "o",
    }
}

fn draw_timing_signs(
    image: &mut Img,
    layout: &ManiaPngLayout,
    prepared: &PreparedMania,
    gameplay: &GameplayTimeline,
    end: i64,
    sv_changes: &[(i64, f64)],
) {
    let sizing = mania_constants::png::sizing();
    let font = crate::render::text::scaled_bitmap_font_height(16, scale());
    let gap = px(2.0);
    let mut rows = std::collections::BTreeMap::<(i64, i64), Vec<(Rgba, &str)>>::new();
    for (_, event, time) in judgements(prepared, gameplay, layout, end) {
        let Some(color) = result_color(event.result) else {
            continue;
        };
        let mark = (color, timing_sign(event));
        let row = rows.entry(locate(time, layout)).or_default();
        if !row.contains(&mark) {
            row.push(mark);
        }
    }
    // 复用 SV 的现有空白；预计算标签位置，绘制时只查询与符号相交的标签。
    let mut sv_labels = vec![Vec::new(); layout.column_count as usize];
    for &(time, sv) in sv_changes {
        let (column, y) = locate(time as f64 + layout.chart_start_time as f64, layout);
        let (width, height) = text_size(&format_sv_label(sv), sizing.SV_TEXT_FONT_SIZE);
        let top = ((png_chart_top() + layout.top_buffer) as f64)
            .max(y as f64 - f64::from(height) / 2.0)
            .floor() as i64;
        let left = (png_column_left(column, layout) - px(1.0) - i64::from(width)).max(0);
        sv_labels[column as usize].push((top, top + i64::from(height), left));
    }
    for labels in &mut sv_labels {
        labels.sort_unstable();
    }
    for ((column, y), mut marks) in rows {
        // 同刻多级判定按背景顺序排列，符号颜色与判定一致。
        marks.sort_by_key(|(color, sign)| {
            let grade = [
                HitResult::Miss,
                HitResult::Meh,
                HitResult::Ok,
                HitResult::Good,
                HitResult::Great,
            ]
            .iter()
            .position(|&r| result_color(r) == Some(*color))
            .unwrap_or(0);
            (grade, *sign)
        });
        let top = (y - i64::from(font) / 2).clamp(
            png_chart_top(),
            (png_chart_top() + layout.total_column_height - i64::from(font)).max(png_chart_top()),
        );
        let mut right = png_column_left(column, layout) - gap;
        let labels = &sv_labels[column as usize];
        let first = labels.partition_point(|&(_, bottom, _)| bottom <= top);
        for &(_, _, left) in labels[first..]
            .iter()
            .take_while(|&&(sv_top, _, _)| sv_top < top + i64::from(font))
        {
            right = right.min(left - gap);
        }
        for (color, sign) in marks.into_iter().rev() {
            let width = i64::from(text_size(sign, font).0);
            draw_text(image, right - width, top, sign, font, color);
            right -= width + gap;
        }
    }
}

fn state_font() -> u32 {
    crate::render::text::scaled_bitmap_font_height(24, scale())
}

pub fn state_panel_width() -> i64 {
    i64::from(text_size("100.00%", state_font()).0 + text_size("SS", state_font()).0) + px(6.0) * 7
}

/// 在每列预留时间标尺和成绩区域，长时间标签不会与 ACC 相撞。
pub fn reserve_state_panel(layout: &mut ManiaPngLayout, lines: &[TimingLine], time_axis: TimeAxis) {
    let sizing = mania_constants::png::sizing();
    let ruler_width = lines
        .iter()
        .filter(|line| line.show_label)
        .map(|line| {
            let label = crate::render::text::format_seconds_tenths(
                time_axis.to_display(line.time + layout.chart_start_time),
            );
            let time_width = text_size(&label, sizing.TIME_LABEL_FONT_SIZE).0;
            let bpm_width = line
                .bpm_label
                .as_deref()
                .map_or(0, |bpm| text_size(bpm, sizing.TIME_LABEL_FONT_SIZE).0);
            i64::from(time_width.max(bpm_width))
        })
        .max()
        .unwrap_or(0);
    let extra = (ruler_width + px(12.0) - sizing.INFO_MARGIN_RIGHT).max(0);
    layout.state_panel_width = state_panel_width() + extra;
    layout.image_width += layout.column_count * layout.state_panel_width;
}

fn state_sampling() -> static_state::StateSampling {
    static_state::StateSampling {
        min_interval_ms: 3500.0,
        max_per_column: 6,
        max_total: 36,
        min_accuracy_change: 0.0005,
    }
}

fn draw_states(
    image: &mut Img,
    layout: &ManiaPngLayout,
    gameplay: &GameplayTimeline,
    end: i64,
    time_axis: TimeAxis,
) {
    let mut seed = 1337_u32.to_le_bytes();
    let _ = getrandom::getrandom(&mut seed);
    let states = static_state::select_states(
        gameplay,
        layout.chart_start_time as f64,
        (end as f64).min(gameplay.settlement_time),
        u32::from_le_bytes(seed),
        scale(),
        |time| locate(time, layout),
        state_sampling(),
    );
    let padding = px(6.0);
    let font = state_font();
    let top = png_chart_top();
    let time_font = mania_constants::png::sizing().TIME_LABEL_FONT_SIZE;
    for (time, state) in states {
        let (column, y) = locate(time, layout);
        let x = png_column_left(column, layout)
            + layout.column_width
            + mania_constants::png::sizing().INFO_MARGIN_RIGHT
            + layout.state_panel_width
            - state_panel_width();
        let label_y = (y - i64::from(font) / 2).clamp(
            top + padding,
            (top + layout.total_column_height - i64::from(font) - padding).max(top + padding),
        );
        let label =
            crate::render::text::format_seconds_tenths(time_axis.to_display(round_half_even(time)));
        draw_text(
            image,
            png_column_left(column, layout) + layout.column_width + px(4.0),
            label_y + (i64::from(font) - i64::from(time_font)) / 2,
            &label,
            time_font,
            mania_constants::png::style::RULER_TEXT_COLOR,
        );
        static_state::draw_badge(image, x, label_y, state, font, padding, scale());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{gameplay::engine::compile_mania, replay::preparation::mania::tests::fixture};
    use std::sync::Arc;

    fn layout() -> ManiaPngLayout {
        let width = px(48.0);
        let gap = mania_constants::png::sizing().LANE_GAP;
        let lane_area_width = width * 4 + gap * 3;
        let column_height = round_half_even(
            2000.0
                * crate::config::current()
                    .render
                    .mania
                    .png
                    .sizing
                    .PIXELS_PER_MS,
        );
        let mut layout = ManiaPngLayout {
            column_count: 3,
            time_per_column: 2000,
            column_height,
            total_column_height: column_height + px(12.0),
            lane_area_width,
            column_width: mania_constants::png::sizing().LEFT_PANEL_WIDTH + lane_area_width,
            lane_widths: vec![width; 4],
            lane_left_offsets: (0..4).map(|i| i * width).collect(),
            top_buffer: px(12.0),
            image_width: 0,
            image_height: png_chart_top() + column_height + px(40.0),
            chart_start_time: 1000,
            state_panel_width: state_panel_width(),
        };
        layout.image_width = png_column_left(2, &layout)
            + layout.column_width
            + mania_constants::png::sizing().INFO_MARGIN_RIGHT
            + layout.state_panel_width
            + px(20.0);
        layout
    }

    fn blank(layout: &ManiaPngLayout) -> Img {
        Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            [0; 4],
        )
    }

    #[test]
    fn input_edges_hold_across_columns_and_clip_to_window_and_failure() {
        for scale in [0.75, 1.25] {
            let mut config = crate::config::CoreConfig::default();
            config.render.mania.png.SCALE = scale;
            config.render.mania.png.sizing.PIXELS_PER_MS = 0.1 * scale;
            config.render.mania.gif.SCALE = 2.0;
            config.render.mania.mp4.SCALE = 0.5;
            crate::config::with_config(Arc::new(config), || {
                let (prepared, document) = fixture(
                    "64,192,1000,128,0,6500:0:0:0:0:",
                    &[(500, 1), (1500, 3), (1500, 1), (1600, 1), (5500, 1)],
                );
                let gameplay = compile_mania(&prepared, document).unwrap();
                let layout = layout();
                let mut image = blank(&layout);
                draw_inputs(&mut image, &layout, &gameplay, 7000);
                for time in [1100.0, 2900.0, 3100.0, 4900.0, 5400.0] {
                    let (column, y) = locate(time, &layout);
                    let (left, right) = lane_bounds(column, 0, &layout);
                    assert_eq!(image.get(((left + right) / 2) as u32, y as u32), INPUT);
                }
                let (column, y) = locate(5600.0, &layout);
                let (left, right) = lane_bounds(column, 0, &layout);
                assert_eq!(image.get(((left + right) / 2) as u32, y as u32), [0; 4]);
                // 同刻按下再松开仍保留边沿；后续同状态帧不重复画横杠。
                let (column, y) = locate(1500.0, &layout);
                let (left, right) = lane_bounds(column, 1, &layout);
                assert_eq!(
                    image.get(((left + right) / 2 + px(8.0)) as u32, y as u32),
                    INPUT
                );
                let (column, y) = locate(1000.0, &layout);
                let (left, right) = lane_bounds(column, 0, &layout);
                assert_eq!(
                    image.get(((left + right) / 2 + px(8.0)) as u32, y as u32),
                    [0; 4]
                );
                let mut after = layout;
                after.chart_start_time = 6000;
                let mut image = blank(&after);
                draw_inputs(&mut image, &after, &gameplay, 7000);
                assert!(image.data.iter().all(|v| *v == 0));
            });
        }
    }

    #[test]
    fn judgement_bands_span_all_lanes_and_keep_great_on_note_only() {
        let mut config = crate::config::CoreConfig::default();
        config.render.mania.png.sizing.PIXELS_PER_MS = 0.1;
        crate::config::with_config(Arc::new(config), || {
            let (prepared, document) = fixture(
                "64,192,1200,128,0,4200:0:0:0:0:",
                &[(0, 0), (1200, 1), (4200, 0), (7500, 0)],
            );
            let mut gameplay = compile_mania(&prepared, document).unwrap();
            let head = gameplay
                .events
                .iter()
                .find(|e| e.kind == EventKind::Judgement && e.node_id.is_none())
                .unwrap()
                .clone();
            let tail = gameplay
                .events
                .iter()
                .find(|e| e.kind == EventKind::Judgement && e.node_id == Some(0))
                .unwrap()
                .clone();
            let layout = layout();
            for result in [
                HitResult::Perfect,
                HitResult::Great,
                HitResult::Good,
                HitResult::Ok,
                HitResult::Meh,
                HitResult::Miss,
                HitResult::IgnoreMiss,
            ] {
                gameplay.events = vec![
                    GameplayEvent {
                        result,
                        ..head.clone()
                    },
                    GameplayEvent {
                        result,
                        ..tail.clone()
                    },
                ];
                let mut image = blank(&layout);
                draw_bands(&mut image, &layout, &prepared, &gameplay, 7000);
                for time in [1200.0, 4200.0] {
                    let (column, y) = locate(time, &layout);
                    let (left, _) = lane_bounds(column, 0, &layout);
                    let pixel = image.get((left + px(3.0)) as u32, y as u32);
                    if matches!(result, HitResult::Perfect | HitResult::Great) {
                        assert_eq!(pixel, [0; 4]);
                    } else {
                        let mut color = result_color(result).unwrap();
                        color[3] = 72;
                        let mut expected = Img::new(1, 1, [0; 4]);
                        expected.fill_rect_size(0, 0, 1, 1, color);
                        assert_eq!(pixel, expected.get(0, 0));
                    }
                    let (last_lane, right) = lane_bounds(column, 3, &layout);
                    assert_eq!(image.get((last_lane + px(3.0)) as u32, y as u32), pixel);
                    assert_eq!(image.get((right - 1) as u32, y as u32), pixel);
                    assert_eq!(image.get(right as u32, y as u32), [0; 4]);
                    let panel = png_column_left(column, &layout);
                    assert_eq!(
                        image.get(panel as u32, y as u32),
                        if matches!(result, HitResult::Perfect | HitResult::Great) {
                            [0; 4]
                        } else {
                            result_color(result).unwrap()
                        }
                    );
                }
                let mut repeated = blank(&layout);
                gameplay.events.push(gameplay.events[0].clone());
                draw_bands(&mut repeated, &layout, &prepared, &gameplay, 7000);
                assert_eq!(image.data, repeated.data);
                draw_judgements(&mut image, &layout, &prepared, &gameplay, 7000);
                if result == HitResult::Miss {
                    assert!(!image
                        .data
                        .chunks_exact(4)
                        .any(|p| p == [255, 255, 255, 255]));
                }
                if result == HitResult::Great {
                    assert!(image
                        .data
                        .chunks_exact(4)
                        .any(|p| p == result_color(result).unwrap()));
                }
            }
        });
    }

    #[test]
    fn overlapping_grades_follow_catch_blending_and_split_the_side_panel() {
        for scale in [0.75, 1.25] {
            let mut config = crate::config::CoreConfig::default();
            config.render.mania.png.SCALE = scale;
            config.render.mania.png.sizing.PIXELS_PER_MS = 0.1 * scale;
            crate::config::with_config(Arc::new(config), || {
                let (prepared, document) =
                    fixture("64,192,1200,1,0,0:0:0:0:", &[(0, 0), (1200, 1), (7500, 0)]);
                let mut gameplay = compile_mania(&prepared, document).unwrap();
                let event = gameplay
                    .events
                    .iter()
                    .find(|e| e.kind == EventKind::Judgement)
                    .unwrap()
                    .clone();
                // 输入事件顺序不决定透明背景的叠加顺序，同级重复只着色一次。
                gameplay.events = [
                    HitResult::Good,
                    HitResult::Miss,
                    HitResult::Ok,
                    HitResult::Meh,
                    HitResult::Miss,
                ]
                .into_iter()
                .map(|result| GameplayEvent {
                    result,
                    ..event.clone()
                })
                .collect();
                let layout = layout();
                let mut image = blank(&layout);
                draw_bands(&mut image, &layout, &prepared, &gameplay, 7000);
                let (column, y) = locate(1200.0, &layout);
                let panel = png_column_left(column, &layout);
                let width = mania_constants::png::sizing().LEFT_PANEL_WIDTH.max(1);
                let mut expected = Img::new(1, 1, [0; 4]);
                for (index, result) in [
                    HitResult::Miss,
                    HitResult::Meh,
                    HitResult::Ok,
                    HitResult::Good,
                ]
                .into_iter()
                .enumerate()
                {
                    let color = result_color(result).unwrap();
                    let from = width * index as i64 / 4;
                    let to = width * (index as i64 + 1) / 4;
                    for x in from..to {
                        assert_eq!(image.get((panel + x) as u32, y as u32), color);
                    }
                    let mut background = color;
                    background[3] = 72;
                    expected.fill_rect_size(0, 0, 1, 1, background);
                }
                let (left, _) = lane_bounds(column, 0, &layout);
                assert_eq!(image.get(left as u32, y as u32), expected.get(0, 0));
                assert_eq!(image.get((panel - 1) as u32, y as u32), [0; 4]);
            });
        }
    }

    #[test]
    fn timing_signs_use_actual_edges_and_leave_automatic_misses_neutral() {
        for (objects, frames, node, expected) in [
            (
                "64,192,1200,1,0",
                vec![(0, 0), (1040, 1), (2000, 0)],
                None,
                "+",
            ),
            (
                "64,192,1200,1,0",
                vec![(0, 0), (1360, 1), (2000, 0)],
                None,
                "-",
            ),
            ("64,192,1200,1,0", vec![(0, 0), (2000, 0)], None, "o"),
            (
                "64,192,1200,128,0,2200:0:0:0:0:",
                vec![(0, 0), (1200, 1), (1960, 0), (3000, 0)],
                Some(0),
                "+",
            ),
            (
                "64,192,1200,128,0,2200:0:0:0:0:",
                vec![(0, 0), (1200, 1), (2440, 0), (3000, 0)],
                Some(0),
                "-",
            ),
            (
                "64,192,1200,128,0,2200:0:0:0:0:",
                vec![(0, 0), (1200, 1), (1700, 0), (3000, 0)],
                Some(1),
                "o",
            ),
            (
                "64,192,1200,128,0,2200:0:0:0:0:",
                vec![(0, 0), (1200, 1), (3000, 0)],
                Some(0),
                "o",
            ),
        ] {
            let (prepared, document) = fixture(objects, &frames);
            let gameplay = compile_mania(&prepared, document).unwrap();
            let event = gameplay
                .events
                .iter()
                .find(|e| e.kind == EventKind::Judgement && e.node_id == node)
                .unwrap();
            assert!(!event.result.is_hit());
            assert_eq!(timing_sign(event), expected);
        }
    }

    #[test]
    fn timing_signs_share_sv_space_without_changing_layout_or_drawing_in_lanes() {
        let (prepared, document) = fixture("64,192,1200,1,0", &[(0, 0), (1360, 1), (2000, 0)]);
        let gameplay = compile_mania(&prepared, document).unwrap();
        let layout = layout();
        let mut image = blank(&layout);
        draw_timing_signs(
            &mut image,
            &layout,
            &prepared,
            &gameplay,
            7000,
            &[(200, 1.5)],
        );
        let (column, _) = locate(1200.0, &layout);
        let panel = png_column_left(column, &layout);
        assert!(image.data.chunks_exact(4).any(|p| p[3] > 0));
        assert!(image
            .data
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, p)| p[3] > 0)
            .all(|(i, _)| (i % image.w as usize) < panel as usize));
        assert_eq!(image.w, layout.image_width as u32);
    }

    #[test]
    fn state_samples_are_more_frequent_and_still_keep_final_state_and_spacing() {
        use crate::gameplay::state::{GameplayState, ReplayStatus};
        let (prepared, document) = fixture("64,192,1200,1,0", &[(0, 0), (1200, 1), (2000, 0)]);
        let mut gameplay = compile_mania(&prepared, document).unwrap();
        gameplay.checkpoints = (0..=60)
            .map(|i| {
                (
                    i as f64 * 1000.0,
                    GameplayState {
                        accuracy: Some(1.0 - i as f64 * 0.0005),
                        rank: Some("S".into()),
                        status: if i == 60 {
                            ReplayStatus::Completed
                        } else {
                            ReplayStatus::Playing
                        },
                        ..Default::default()
                    },
                )
            })
            .collect();
        let locate = |time: f64| ((time / 20000.0) as i64, (time % 20000.0) as i64 / 10);
        let previous = static_state::select_states(
            &gameplay,
            0.0,
            60000.0,
            1,
            1.0,
            locate,
            Default::default(),
        );
        let current =
            static_state::select_states(&gameplay, 0.0, 60000.0, 1, 1.0, locate, state_sampling());
        assert!(current.len() > previous.len());
        assert_eq!(current.last().unwrap().1.status, ReplayStatus::Completed);
        assert!(current
            .windows(2)
            .all(|pair| pair[1].0 - pair[0].0 >= 3500.0));
    }
}
