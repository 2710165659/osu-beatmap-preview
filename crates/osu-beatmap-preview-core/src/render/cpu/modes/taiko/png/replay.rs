//! 太鼓静态回放：保留原谱面排版，叠加判定背景、击打早晚、左右手击打及下方 ACC 轨道。

use super::*;
use crate::{
    gameplay::{
        events::{EventKind, GameplayEvent, HitResult},
        timeline::{GameplayTimeline, TaikoPress},
    },
    render::{
        geometry::{output_scale, scale_px, GameMode, OutputFormat},
        shared::replay::{
            colors,
            static_state::{badge_width, draw_badge},
        },
        text::scaled_bitmap_font_height,
    },
    replay::preparation::taiko::{PreparedObject, PreparedTaiko, TaikoObjectKind},
};

const MISS: [u8; 4] = colors::hit_result(HitResult::Miss);
const OK: [u8; 4] = colors::hit_result(HitResult::Ok);
const MISS_BAND: [u8; 4] = colors::with_alpha(MISS, 72);
const OK_BAND: [u8; 4] = colors::with_alpha(OK, 72);
const INPUT_COLORS: [[u8; 4]; 2] = [[164, 30, 34, 255], [27, 83, 158, 255]];

struct ReplayLayout<'a> {
    chart: &'a TaikoPngLayout,
    scale: f64,
    extra: i64,
    stride: i64,
}

impl<'a> ReplayLayout<'a> {
    fn new(chart: &'a TaikoPngLayout) -> Self {
        let scale = output_scale(GameMode::Taiko, OutputFormat::Png);
        let sizing = taiko_constants::png::sizing();
        Self {
            chart,
            scale,
            extra: scale_px(44.0, scale),
            stride: sizing.ROW_HEIGHT + sizing.INFO_MARGIN_BOTTOM + sizing.ROW_GAP,
        }
    }

    fn px(&self, value: f64) -> i64 {
        scale_px(value, self.scale)
    }

    fn strip_end(&self, row: i64) -> i64 {
        if row + 1 == self.chart.row_count {
            self.chart.image_height
        } else {
            (row + 1) * self.stride
        }
    }

    fn locate(&self, mapper: &ScrollPositionMapper, time: f64) -> (i64, f64) {
        let (row, position) = self
            .chart
            .locate(mapper.position_at(time - self.chart.chart_start_time as f64));
        (row, png_row_chart_left(self.chart, row) as f64 + position)
    }

    fn row_right(&self, mapper: &ScrollPositionMapper, row: i64) -> i64 {
        let end = self
            .chart
            .row_start_positions
            .get(row as usize + 1)
            .copied()
            .unwrap_or(mapper.end_position());
        let width = (end - self.chart.row_start_positions[row as usize])
            .ceil()
            .max(1.0) as i64;
        png_row_chart_left(self.chart, row)
            + width
            + taiko_constants::png::sizing().ROW_INNER_PADDING_X
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_scene(
    chart: &TaikoPngLayout,
    hit_objects: &[TaikoHitObject],
    mapper: &ScrollPositionMapper,
    timing_lines: &[TimingLine],
    sv_changes: &[SvChange],
    time_axis: TimeAxis,
    prepared: &PreparedTaiko,
    gameplay: &GameplayTimeline,
    end: i64,
) -> Img {
    let layout = ReplayLayout::new(chart);
    let result_end = if end as f64 >= prepared.last_end {
        gameplay.settlement_time
    } else {
        end as f64
    };
    let mut base = render_background(chart, mapper, timing_lines, sv_changes, time_axis);
    draw_bands(
        &mut base, &layout, mapper, prepared, gameplay, end, result_end,
    );
    let mut cache = RenderCache::default();
    for object in hit_objects.iter().rev() {
        draw_hit_object(&mut base, object, mapper, chart, &mut cache);
    }
    draw_judgements(
        &mut base, &layout, mapper, prepared, gameplay, end, result_end,
    );
    draw_inputs(&mut base, &layout, mapper, &gameplay.taiko_inputs, end);
    draw_timing_signs(
        &mut base, &layout, mapper, prepared, gameplay, end, result_end,
    );

    let mut image = Img::new(
        chart.image_width as u32,
        (chart.image_height + chart.row_count * layout.extra) as u32,
        taiko_constants::png::style::IMAGE_BACKGROUND,
    );
    // 原谱面只按行平移，为 ACC 留出空间；不增加顶栏、侧栏或独立输入面板。
    for row in 0..chart.row_count {
        for y in row * layout.stride..layout.strip_end(row) {
            let source = y as usize * base.w as usize * 4;
            let target = (y + row * layout.extra) as usize * image.w as usize * 4;
            let bytes = base.w as usize * 4;
            image.data[target..target + bytes].copy_from_slice(&base.data[source..source + bytes]);
        }
    }
    drop(base);
    draw_states(&mut image, &layout, mapper, gameplay, result_end, time_axis);
    image
}

fn visible_judgements<'a>(
    prepared: &'a PreparedTaiko,
    gameplay: &'a GameplayTimeline,
    start: i64,
    end: i64,
    result_end: f64,
) -> impl Iterator<Item = (&'a PreparedObject, &'a GameplayEvent, f64)> {
    gameplay.events.iter().filter_map(move |event| {
        if event.kind != EventKind::Judgement || event.time > result_end {
            return None;
        }
        let object = &prepared.objects[event.object_id];
        let time = judgement_time(object, event)?;
        (time >= start as f64 && time <= end as f64).then_some((object, event, time))
    })
}

fn draw_bands(
    image: &mut Img,
    layout: &ReplayLayout<'_>,
    mapper: &ScrollPositionMapper,
    prepared: &PreparedTaiko,
    gameplay: &GameplayTimeline,
    end: i64,
    result_end: f64,
) {
    let width = image.w as usize;
    let mut coverage = vec![0_u8; width * layout.chart.row_count as usize];
    for (object, event, time) in visible_judgements(
        prepared,
        gameplay,
        layout.chart.chart_start_time,
        end,
        result_end,
    ) {
        let flag = match event.result {
            HitResult::Ok => 2,
            HitResult::IgnoreMiss => 4,
            result if !result.is_hit() => 1,
            _ => continue,
        };
        let (row, x) = layout.locate(mapper, time);
        let radius = if matches!(object.kind, TaikoObjectKind::Hit) && event.node_id.is_none() {
            note_diameter(layout.chart, object) / 2 + layout.px(4.0)
        } else {
            layout.px(6.0)
        };
        let left = taiko_constants::png::sizing().PAGE_MARGIN_LEFT.max(0);
        let right = layout.row_right(mapper, row).min(image.w as i64);
        let from = (x.round() as i64 - radius).clamp(left, right) as usize;
        let to = (x.round() as i64 + radius + 1).clamp(left, right) as usize;
        for column in &mut coverage[row as usize * width + from..row as usize * width + to] {
            *column |= flag;
        }
    }
    // 同一种背景每列只混合一次，密集判定不会反复叠色盖住谱面。
    let height = taiko_constants::png::sizing().ROW_HEIGHT;
    for row in 0..layout.chart.row_count {
        for (x, &flag) in coverage[row as usize * width..(row as usize + 1) * width]
            .iter()
            .enumerate()
        {
            for (mask, color) in [
                (1, MISS_BAND),
                (2, OK_BAND),
                (4, colors::with_alpha(colors::BLUE, 72)),
            ] {
                if flag & mask != 0 {
                    image.fill_rect_size(x as i64, png_row_top(row), 1, height, color);
                }
            }
        }
    }
}

fn input_marker(key: u32, diameter: i64) -> Img {
    let diameter = diameter.max(3);
    let mut marker = Img::new(diameter as u32, diameter as u32, [0; 4]);
    let center = diameter as f64 / 2.0;
    let color = INPUT_COLORS[usize::from(key & 10 != 0)];
    marker.fill_circle_aa(center, center, center, [18, 18, 18, 240]);
    let edge = (diameter as f64 / 15.0).max(0.5);
    marker.fill_circle_aa(center, center, (center - edge).max(0.5), color);
    // 按鼓面中线切成左右半圆，同刻双手可组成完整圆，内外仍由红蓝区分。
    let right = key & 12 != 0;
    for y in 0..marker.h {
        for x in 0..marker.w {
            if (x as f64 + 0.5 >= center) != right {
                marker.put(x, y, [0; 4]);
            }
        }
    }
    marker
}

fn draw_inputs(
    image: &mut Img,
    layout: &ReplayLayout<'_>,
    mapper: &ScrollPositionMapper,
    presses: &[TaikoPress],
    end: i64,
) {
    let diameter = (layout.chart.normal_note_diameter as f64 * 0.32)
        .round()
        .max(3.0) as i64;
    let markers = [1, 2, 4, 8].map(|key| input_marker(key, diameter));
    for press in presses {
        if press.time < layout.chart.chart_start_time as f64 || press.time > end as f64 {
            continue;
        }
        let (row, x) = layout.locate(mapper, press.time);
        let marker = &markers[press.key.trailing_zeros() as usize];
        image.alpha_composite(
            marker,
            pyround(x - diameter as f64 / 2.0),
            png_row_center_y(row) - diameter / 2,
        );
    }
}

fn note_diameter(chart: &TaikoPngLayout, object: &PreparedObject) -> i64 {
    if object.strong() {
        chart.big_note_diameter
    } else {
        chart.normal_note_diameter
    }
}

fn draw_judgements(
    image: &mut Img,
    layout: &ReplayLayout<'_>,
    mapper: &ScrollPositionMapper,
    prepared: &PreparedTaiko,
    gameplay: &GameplayTimeline,
    end: i64,
    result_end: f64,
) {
    for (object, event, time) in visible_judgements(
        prepared,
        gameplay,
        layout.chart.chart_start_time,
        end,
        result_end,
    ) {
        if !matches!(event.result, HitResult::Ok | HitResult::Miss) {
            continue;
        }
        let (row, x) = layout.locate(mapper, time);
        let diameter = if matches!(object.kind, TaikoObjectKind::Hit) && event.node_id.is_none() {
            note_diameter(layout.chart, object) as f64
        } else {
            layout.px(12.0).max(3) as f64
        };
        draw_note_judgement(
            image,
            [x, png_row_center_y(row) as f64],
            diameter,
            layout.scale,
            event.result,
        );
    }
}

fn draw_note_judgement(
    image: &mut Img,
    center: [f64; 2],
    diameter: f64,
    scale: f64,
    result: HitResult,
) {
    let color = match result {
        HitResult::Ok => OK,
        HitResult::Miss => MISS,
        _ => return,
    };
    let [x, y] = center;
    let edge = crate::render::geometry::scale_stroke_px(1.0, scale) as f64;
    let radius = (diameter / 2.0 - edge).max(0.5);
    image.stroke_circle_aa(
        x,
        y,
        radius,
        (diameter * NOTE_RING_THICKNESS_RATIO).max(1.0),
        color,
    );
}

fn timing_sign(event: &GameplayEvent, great_window: f64) -> Option<&'static str> {
    match event.error {
        Some(error) if error.abs() <= great_window => None,
        Some(error) if error < 0.0 => Some("+"),
        Some(error) if error > 0.0 => Some("-"),
        _ => None,
    }
}

fn wrong_colour_hit(
    object: &PreparedObject,
    event: &GameplayEvent,
    presses: &[TaikoPress],
) -> bool {
    if event.result != HitResult::Miss || event.error.is_none() {
        return false;
    }
    let first = presses.partition_point(|press| press.time < event.time);
    // 同刻可能有多次击打，只检查真正触发主体判定的首个关联输入。
    presses[first..]
        .iter()
        .take_while(|press| press.time == event.time)
        .find(|press| press.object_id == Some(event.object_id) && press.node_id.is_none())
        .is_some_and(|press| (press.key & 10 != 0) != object.rim())
}

fn draw_wrong_colour_mark(image: &mut Img, x: i64, y: i64, size: i64) {
    // 位图字体不含 ×，直接绘制两条对角线，避免回退为问号。
    let stroke = (size as f64 / 6.0).max(1.0);
    let left = x as f64 + stroke / 2.0;
    let right = (x + size) as f64 - stroke / 2.0;
    let top = y as f64 + stroke / 2.0;
    let bottom = (y + size) as f64 - stroke / 2.0;
    image.draw_line(left, top, right, bottom, stroke, MISS);
    image.draw_line(left, bottom, right, top, stroke, MISS);
}

fn draw_timing_signs(
    image: &mut Img,
    layout: &ReplayLayout<'_>,
    mapper: &ScrollPositionMapper,
    prepared: &PreparedTaiko,
    gameplay: &GameplayTimeline,
    end: i64,
    result_end: f64,
) {
    let great_window = crate::domain::rulesets::taiko::gameplay::hit_windows(prepared.od)[0];
    for (object, event, time) in visible_judgements(
        prepared,
        gameplay,
        layout.chart.chart_start_time,
        end,
        result_end,
    ) {
        if !matches!(object.kind, TaikoObjectKind::Hit)
            || event.node_id.is_some()
            || !matches!(event.result, HitResult::Miss | HitResult::Ok)
        {
            continue;
        }
        let sign = timing_sign(event, great_window);
        let wrong_colour = wrong_colour_hit(object, event, &gameplay.taiko_inputs);
        if sign.is_none() && !wrong_colour {
            continue;
        }
        let (row, x) = layout.locate(mapper, time);
        let bottom = png_row_top(row) + taiko_constants::png::sizing().ROW_HEIGHT;
        let top =
            png_row_center_y(row) + note_diameter(layout.chart, object) / 2 + layout.px(2.0).max(1);
        // 大饼下方的空白更窄，只缩小符号，不改变轨道、时间标尺和行间距。
        let available = bottom - layout.px(1.0).max(1) - top;
        if available <= 0 {
            continue;
        }
        let font = scaled_bitmap_font_height(16, layout.scale).min(available as u32);
        let sign_width = sign.map_or(0, |sign| i64::from(text_size(sign, font).0));
        let cross_size = i64::from(text_size("+", font).0);
        let gap = if wrong_colour && sign.is_some() {
            layout.px(3.0).max(1)
        } else {
            0
        };
        let width = sign_width + if wrong_colour { cross_size + gap } else { 0 };
        let left = pyround(x - width as f64 / 2.0);
        if wrong_colour {
            draw_wrong_colour_mark(
                image,
                left,
                top + (i64::from(font) - cross_size) / 2,
                cross_size,
            );
        }
        if let Some(sign) = sign {
            draw_text(
                image,
                left + if wrong_colour { cross_size + gap } else { 0 },
                top,
                sign,
                font,
                colors::hit_result(event.result),
            );
        }
    }
}

fn judgement_time(object: &PreparedObject, event: &GameplayEvent) -> Option<f64> {
    // sample_time 用于音效选样，滚奏超时事件也可能保存条头时间，不能用它定位静态节点。
    match &object.kind {
        // 大饼的第二次击打是独立奖励节点，漏判必须与主体判定分别显示。
        TaikoObjectKind::Hit => Some(object.source.start_time as f64),
        TaikoObjectKind::DrumRoll { ticks, .. } => {
            if event.maximum != HitResult::SmallBonus {
                return None;
            }
            event
                .node_id
                .and_then(|node| ticks.get(node))
                .map(|tick| tick.time)
        }
        TaikoObjectKind::Swell { .. } => match event.node_id {
            None => Some(object.source.end_time as f64),
            Some(_) => None,
        },
    }
}

fn draw_states(
    image: &mut Img,
    layout: &ReplayLayout<'_>,
    mapper: &ScrollPositionMapper,
    gameplay: &GameplayTimeline,
    end: f64,
    time_axis: TimeAxis,
) {
    let padding = layout.px(6.0).max(1);
    let font = scaled_bitmap_font_height(24, layout.scale);
    let start = layout.chart.chart_start_time as f64;
    let mut candidates = Vec::new();
    let mut previous = &gameplay.initial;
    for (time, state) in &gameplay.checkpoints {
        let rank_changed = state.rank != previous.rank || state.status != previous.status;
        let accuracy_changed =
            (state.accuracy.unwrap_or(1.0) - previous.accuracy.unwrap_or(1.0)).abs() >= 0.001;
        if rank_changed || accuracy_changed {
            previous = state;
            if *time >= start && *time <= end {
                candidates.push((if rank_changed { 1 } else { 2 }, *time));
            }
        }
    }
    for row in 0..layout.chart.row_count {
        let left = taiko_constants::png::sizing().PAGE_MARGIN_LEFT;
        let right = layout.row_right(mapper, row);
        let top = layout.strip_end(row) + row * layout.extra;
        image.fill_rect_size(
            left,
            top,
            right - left,
            layout.extra - layout.px(6.0),
            [15, 15, 15, 255],
        );
        let row_end = layout
            .chart
            .row_start_positions
            .get(row as usize + 1)
            .map_or(end, |position| mapper.time_at(*position) + start);
        let row_start = mapper.time_at(layout.chart.row_start_positions[row as usize]) + start;
        let mut points: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|(_, time)| *time >= row_start && *time < row_end)
            .collect();
        // 每行保留末尾快照，评级变化优先；只在有阅读空间的位置补充中间状态。
        points.push((0, row_end.min(end)));
        points.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut occupied: Vec<(i64, i64, f64)> = Vec::new();
        for (_, time) in points {
            let state = gameplay.state_at(time);
            let available = (right - left - padding * 2).max(1);
            let shrink = (available as f64 / badge_width(state, font, padding) as f64).min(1.0);
            let font = (font as f64 * shrink).floor().max(1.0) as u32;
            let padding = (padding as f64 * shrink).floor().max(1.0) as i64;
            let width = badge_width(state, font, padding);
            let position =
                mapper.position_at(time - start) - layout.chart.row_start_positions[row as usize];
            let center = png_row_chart_left(layout.chart, row) + position.round() as i64;
            let x = (center - width / 2).clamp(
                left + padding,
                (right - width - padding).max(left + padding),
            );
            if occupied.iter().any(|(from, to, at)| {
                (time - at).abs() < 5_000.0
                    || (x < *to + layout.px(24.0) && x + width > *from - layout.px(24.0))
            }) {
                continue;
            }
            draw_time_label(
                image,
                &TimingLine {
                    time: pyround(time) - layout.chart.chart_start_time,
                    position: mapper.position_at(time - start),
                    is_measure: false,
                    show_label: true,
                    is_kiai: false,
                    is_kiai_start: false,
                    bpm: None,
                },
                center,
                png_row_top(row) + row * layout.extra,
                layout.chart,
                time_axis,
            );
            draw_badge(image, x, top + padding, state, font, padding, layout.scale);
            occupied.push((x, x + width, time));
            if occupied.len() >= 4 {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rulesets::taiko::gameplay::DrumRollTick;

    fn fixture(objects: &str, frames: &[(i64, u32)]) -> (PreparedTaiko, GameplayTimeline) {
        let (_, mut document) =
            crate::replay::preparation::mania::tests::fixture("64,192,1000,1,0", frames);
        let replay = std::sync::Arc::get_mut(&mut document).unwrap();
        replay.header.ruleset_id = 1;
        replay.header.version = 20200101;
        for (frame, &(_, keys)) in replay.frames.iter_mut().zip(frames) {
            frame.keys = keys;
        }
        let map = crate::parse_beatmap_bytes(format!("osu file format v14\n[General]\nMode:1\n[Difficulty]\nOverallDifficulty:5\nHPDrainRate:5\nSliderMultiplier:1\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n{objects}\n").as_bytes()).unwrap();
        let prepared = crate::replay::preparation::taiko::prepare_replay(&map, &document).unwrap();
        let gameplay = crate::gameplay::engine::compile_taiko(&prepared, document).unwrap();
        (prepared, gameplay)
    }

    #[test]
    fn wrong_colour_uses_the_judging_press_and_keeps_timing_separate() {
        for (sound, key, offset, wrong) in [
            (0, 2, -35, true),
            (0, 2, -34, true),
            (0, 2, 0, true),
            (0, 2, 34, true),
            (0, 2, 35, true),
            (8, 1, -35, true),
            (8, 1, -34, true),
            (8, 1, 0, true),
            (8, 1, 34, true),
            (8, 1, 35, true),
            (0, 1, 80, false),
            (8, 2, 80, false),
        ] {
            let (prepared, gameplay) = fixture(
                &format!("0,0,1000,1,{sound}"),
                &[(0, 0), (1000 + offset, key), (1200, 0)],
            );
            let event = gameplay
                .events
                .iter()
                .find(|e| e.kind == EventKind::Judgement)
                .unwrap();
            assert_eq!(
                wrong_colour_hit(&prepared.objects[0], event, &gameplay.taiko_inputs),
                wrong
            );
            assert_eq!(
                timing_sign(
                    event,
                    crate::domain::rulesets::taiko::gameplay::hit_windows(prepared.od)[0]
                ),
                if offset.abs() <= 34 {
                    None
                } else if offset < 0 {
                    Some("+")
                } else {
                    Some("-")
                }
            );
        }
        // 同刻的后续错色输入不能把先前的普通超窗 Miss 误标为错色。
        for (frames, wrong) in [
            (
                vec![(0, 0), (1080, 1), (1080, 0), (1080, 2), (1200, 0)],
                false,
            ),
            (
                vec![(0, 0), (1080, 2), (1080, 0), (1080, 1), (1200, 0)],
                true,
            ),
            (vec![(0, 0), (1200, 0)], false),
        ] {
            let (prepared, gameplay) = fixture("0,0,1000,1,0", &frames);
            let event = gameplay
                .events
                .iter()
                .find(|e| e.kind == EventKind::Judgement)
                .unwrap();
            assert_eq!(
                wrong_colour_hit(&prepared.objects[0], event, &gameplay.taiko_inputs),
                wrong
            );
        }
    }

    #[test]
    fn timing_signs_stay_below_notes_and_inside_existing_rows() {
        let objects = "0,0,1000,1,0\n0,0,2000,1,4\n0,0,3000,1,0\n0,0,4000,1,0\n0,0,5000,1,0\n0,0,6000,2,0,L|100:0,1,100\n0,0,7000,1,0\n0,0,7500,1,0";
        let frames = [
            (0, 0),
            (965, 2),
            (966, 0),
            (2035, 2),
            (2036, 0),
            (3080, 1),
            (3081, 0),
            (5000, 2),
            (5001, 0),
            (6000, 1),
            (6001, 0),
            (7035, 1),
            (7036, 0),
            (7510, 1),
            (7511, 0),
            (8000, 0),
        ];
        let (prepared, gameplay) = fixture(objects, &frames);
        for (id, expected) in [
            (0, Some("+")),
            (1, Some("-")),
            (2, Some("-")),
            (3, None),
            (4, None),
            (7, None),
        ] {
            let event = gameplay
                .events
                .iter()
                .find(|e| {
                    e.kind == EventKind::Judgement && e.object_id == id && e.node_id.is_none()
                })
                .unwrap();
            assert_eq!(
                timing_sign(
                    event,
                    crate::domain::rulesets::taiko::gameplay::hit_windows(prepared.od)[0]
                ),
                expected
            );
        }
        for scale in [0.5, 0.75, 1.0, 1.25, 2.0] {
            let mut config = crate::config::CoreConfig::default();
            config.render.taiko.png.SCALE = scale;
            config.render.taiko.gif.SCALE = 3.0;
            config.render.taiko.mp4.SCALE = 4.0;
            crate::config::with_config(std::sync::Arc::new(config), || {
                let px = |value| scale_px(value, scale);
                let sizing = taiko_constants::png::sizing();
                let stride = sizing.ROW_HEIGHT + sizing.INFO_MARGIN_BOTTOM + sizing.ROW_GAP;
                let chart = TaikoPngLayout {
                    row_count: 2,
                    max_row_width: px(400.0),
                    content_width: px(466.0),
                    image_width: px(482.0),
                    image_height: stride * 2 + px(18.0),
                    normal_note_diameter: px(48.0),
                    big_note_diameter: px(74.0),
                    row_start_positions: vec![0.0, 400.0 * scale],
                    chart_start_time: 0,
                    overall_difficulty: 5.0,
                };
                let mapper = ScrollPositionMapper::new(vec![ScrollSegment {
                    start_time: 0,
                    end_time: 8000,
                    pixels_per_ms: 0.1 * scale,
                    start_position: 0.0,
                }]);
                let layout = ReplayLayout::new(&chart);
                let mut image = Img::new(
                    chart.image_width as u32,
                    (chart.image_height + chart.row_count * layout.extra) as u32,
                    [0; 4],
                );
                draw_timing_signs(
                    &mut image, &layout, &mapper, &prepared, &gameplay, 7500, 8000.0,
                );
                for (id, time) in [
                    (0, 1000.0),
                    (1, 2000.0),
                    (2, 3000.0),
                    (4, 5000.0),
                    (6, 7000.0),
                ] {
                    let (row, x) = layout.locate(&mapper, time);
                    let top =
                        png_row_center_y(row) + note_diameter(&chart, &prepared.objects[id]) / 2;
                    let bottom = png_row_top(row) + sizing.ROW_HEIGHT;
                    assert!(
                        (top..bottom).any(|y| {
                            (x as i64 - px(14.0)..=x as i64 + px(14.0))
                                .any(|x| image.get(x as u32, y as u32)[3] > 0)
                        }),
                        "missing sign for note {id} at scale {scale}"
                    );
                }
                assert!(image
                    .data
                    .chunks_exact(4)
                    .enumerate()
                    .filter(|(_, p)| p[3] > 0)
                    .all(|(i, _)| {
                        let x = (i % image.w as usize) as i64;
                        let y = (i / image.w as usize) as i64;
                        [0, 1, 2, 4, 6].into_iter().any(|id| {
                            let (row, center) = layout
                                .locate(&mapper, prepared.objects[id].source.start_time as f64);
                            (x - center.round() as i64).abs() <= px(14.0)
                                && y > png_row_center_y(row)
                                    + note_diameter(&chart, &prepared.objects[id]) / 2
                                && y < png_row_top(row) + sizing.ROW_HEIGHT
                        })
                    }));
                let mut clipped = image.clone();
                clipped.data.fill(0);
                draw_timing_signs(
                    &mut clipped,
                    &layout,
                    &mapper,
                    &prepared,
                    &gameplay,
                    900,
                    8000.0,
                );
                assert!(clipped.data.iter().all(|v| *v == 0));
                draw_timing_signs(
                    &mut clipped,
                    &layout,
                    &mapper,
                    &prepared,
                    &gameplay,
                    7500,
                    900.0,
                );
                assert!(clipped.data.iter().all(|v| *v == 0));
            });
        }
    }

    #[test]
    fn state_badges_include_display_time_without_chart_connector() {
        let (prepared, document) = crate::replay::preparation::mania::tests::fixture(
            "64,192,1200,1,0",
            &[(0, 0), (1200, 1), (2000, 0)],
        );
        let gameplay = crate::gameplay::engine::compile_mania(&prepared, document).unwrap();
        for scale in [0.75, 1.0, 1.25] {
            let mut config = crate::config::CoreConfig::default();
            config.render.taiko.png.SCALE = scale;
            crate::config::with_config(std::sync::Arc::new(config), || {
                let px = |value| scale_px(value, scale);
                let sizing = taiko_constants::png::sizing();
                let stride = sizing.ROW_HEIGHT + sizing.INFO_MARGIN_BOTTOM + sizing.ROW_GAP;
                let chart = TaikoPngLayout {
                    row_count: 1,
                    max_row_width: px(800.0),
                    content_width: px(866.0),
                    image_width: px(900.0),
                    image_height: stride + px(18.0),
                    normal_note_diameter: px(48.0),
                    big_note_diameter: px(74.0),
                    row_start_positions: vec![0.0],
                    chart_start_time: 1000,
                    overall_difficulty: 5.0,
                };
                let mapper = ScrollPositionMapper::new(vec![ScrollSegment {
                    start_time: 0,
                    end_time: 4000,
                    pixels_per_ms: 0.2 * scale,
                    start_position: 0.0,
                }]);
                let layout = ReplayLayout::new(&chart);
                let mut image = Img::new(
                    chart.image_width as u32,
                    (chart.image_height + layout.extra) as u32,
                    [0; 4],
                );
                draw_states(
                    &mut image,
                    &layout,
                    &mapper,
                    &gameplay,
                    2100.0,
                    TimeAxis::new(1000),
                );
                let chart_bottom = png_row_top(0) + sizing.ROW_HEIGHT;
                assert!((0..chart_bottom as u32)
                    .all(|y| { (0..image.w).all(|x| image.get(x, y) == [0; 4]) }));
                let label_width = i64::from(text_size("1.1s", sizing.TIME_LABEL_FONT_SIZE).0);
                let center = png_row_chart_left(&chart, 0) + px(220.0);
                let label_x = pyround(center as f64 - label_width as f64 / 2.0);
                let label_y = chart_bottom + sizing.TIME_LABEL_TOP_GAP;
                let mut expected = Img::new(image.w, image.h, [0; 4]);
                draw_text(
                    &mut expected,
                    label_x,
                    label_y,
                    "1.1s",
                    sizing.TIME_LABEL_FONT_SIZE,
                    taiko_constants::png::style::RULER_TEXT_COLOR,
                );
                for y in chart_bottom as u32..layout.strip_end(0) as u32 {
                    for x in 0..image.w {
                        assert_eq!(image.get(x, y), expected.get(x, y));
                    }
                }
                let font = scaled_bitmap_font_height(24, scale);
                let padding = px(6.0).max(1);
                let width = badge_width(gameplay.state_at(2100.0), font, padding);
                let badge_x = (center - width / 2).max(sizing.PAGE_MARGIN_LEFT + padding);
                assert_ne!(
                    image.get(badge_x as u32, layout.strip_end(0) as u32),
                    [0; 4]
                );
            });
        }
    }

    #[test]
    fn judgement_rings_keep_note_fill_and_great_keeps_original_sprite() {
        for (diameter, scale) in [(48, 1.0), (60, 1.25), (74, 1.0)] {
            for color in [CENTRE_NOTE_COLOR, RIM_NOTE_COLOR] {
                let edge = crate::render::geometry::scale_stroke_px(1.0, scale);
                let base =
                    super::super::super::notes::build_note_disc(color, diameter, edge, false);
                let center = diameter as f64 / 2.0;
                for (result, ring) in [
                    (HitResult::Miss, MISS),
                    (HitResult::Ok, OK),
                    (HitResult::Great, NOTE_RING_COLOR),
                ] {
                    let mut image = base.clone();
                    draw_note_judgement(
                        &mut image,
                        [center, center],
                        diameter as f64,
                        scale,
                        result,
                    );
                    let fill = image.get((center * 0.5) as u32, center as u32);
                    assert_eq!(fill, [color[0], color[1], color[2], 255]);
                    assert_eq!(
                        image.get(center as u32, center as u32),
                        base.get(center as u32, center as u32)
                    );
                    assert_eq!(image.get(edge as u32 + 1, center as u32), ring);
                    if result == HitResult::Great {
                        assert_eq!(image.data, base.data);
                    }
                }
            }
        }
    }

    #[test]
    fn inputs_stay_on_note_centers_across_scales_windows_and_row_boundaries() {
        for scale in [0.5, 1.0, 1.25, 2.0] {
            let mut config = crate::config::CoreConfig::default();
            config.render.taiko.png.SCALE = scale;
            config.render.taiko.gif.SCALE = 3.0;
            config.render.taiko.mp4.SCALE = 4.0;
            crate::config::with_config(std::sync::Arc::new(config), || {
                let px = |value| scale_px(value, scale);
                let sizing = taiko_constants::png::sizing();
                let stride = sizing.ROW_HEIGHT + sizing.INFO_MARGIN_BOTTOM + sizing.ROW_GAP;
                let chart = TaikoPngLayout {
                    row_count: 2,
                    max_row_width: px(400.0),
                    content_width: px(466.0),
                    image_width: px(482.0),
                    image_height: stride * 2 + px(18.0),
                    normal_note_diameter: px(48.0),
                    big_note_diameter: px(74.0),
                    row_start_positions: vec![0.0, 400.0 * scale],
                    chart_start_time: 10_000,
                    overall_difficulty: 5.0,
                };
                let mapper = ScrollPositionMapper::new(vec![ScrollSegment {
                    start_time: 0,
                    end_time: 4_000,
                    pixels_per_ms: 0.2 * scale,
                    start_position: 0.0,
                }]);
                let layout = ReplayLayout::new(&chart);
                assert_eq!(layout.strip_end(1), chart.image_height);
                let mut image =
                    Img::new(chart.image_width as u32, chart.image_height as u32, [0; 4]);
                let presses = [(10_100.0, 1), (10_200.0, 8), (12_000.0, 4), (13_001.0, 2)].map(
                    |(time, key)| TaikoPress {
                        time,
                        key,
                        object_id: Some(0),
                        node_id: None,
                    },
                );
                draw_inputs(&mut image, &layout, &mapper, &presses, 13_000);
                let diameter = (chart.normal_note_diameter as f64 * 0.32).round().max(3.0) as i64;
                for (time, key, expected_row) in
                    [(10_100.0, 1_u32, 0), (10_200.0, 8, 0), (12_000.0, 4, 1)]
                {
                    let (row, x) = layout.locate(&mapper, time);
                    assert_eq!(row, expected_row);
                    let marker = input_marker(key, diameter);
                    let offset = if key & 12 != 0 {
                        marker.w * 3 / 4
                    } else {
                        marker.w / 4
                    };
                    let x = pyround(x - diameter as f64 / 2.0) + i64::from(offset);
                    let y = png_row_center_y(row);
                    assert_eq!(
                        image.get(x as u32, y as u32),
                        INPUT_COLORS[usize::from(key & 10 != 0)]
                    );
                }
                let (row, x) = layout.locate(&mapper, 13_001.0);
                assert_eq!(
                    image.get(x.round() as u32, png_row_center_y(row) as u32),
                    [0; 4]
                );
            });
        }
    }

    #[test]
    fn input_markers_split_hands_and_keep_red_blue_identity() {
        for diameter in [8, 15, 20] {
            for key in [1_u32, 2, 4, 8] {
                let marker = input_marker(key, diameter);
                let right = key & 12 != 0;
                let mid = marker.h / 2;
                let colored_x = if right {
                    marker.w * 3 / 4
                } else {
                    marker.w / 4
                };
                assert_eq!(
                    marker.get(colored_x, mid),
                    INPUT_COLORS[usize::from(key & 10 != 0)]
                );
                assert_eq!(marker.get(marker.w - 1 - colored_x, mid), [0; 4]);
            }
        }
    }
    #[test]
    fn bonus_markers_use_node_times_and_keep_strong_hit_bonus_separate() {
        let mut object = PreparedObject {
            source: TaikoHitObject {
                start_time: 1_000,
                end_time: 2_000,
                hit_type: 2,
                hitsound: 4,
                samples: Vec::new(),
            },
            kind: TaikoObjectKind::DrumRoll {
                ticks: vec![
                    DrumRollTick { time: 1_000.0 },
                    DrumRollTick { time: 1_125.0 },
                ],
                window: 62.5,
            },
        };
        let mut event = GameplayEvent {
            id: 0,
            time: 1_188.0,
            object_id: 0,
            node_id: Some(1),
            result: HitResult::IgnoreMiss,
            maximum: HitResult::SmallBonus,
            kind: EventKind::Judgement,
            error: None,
            position: None,
            sample_time: 1_000.0,
        };
        assert_eq!(judgement_time(&object, &event), Some(1_125.0));
        event.maximum = HitResult::LargeBonus;
        assert_eq!(judgement_time(&object, &event), None);
        object.kind = TaikoObjectKind::Swell { required: 2 };
        assert_eq!(judgement_time(&object, &event), None);
        event.result = HitResult::IgnoreHit;
        assert_eq!(judgement_time(&object, &event), None);
        event.node_id = None;
        assert_eq!(judgement_time(&object, &event), Some(2_000.0));
        object.kind = TaikoObjectKind::Hit;
        assert_eq!(judgement_time(&object, &event), Some(1_000.0));
        event.node_id = Some(0);
        assert_eq!(judgement_time(&object, &event), Some(1_000.0));
        event.result = HitResult::IgnoreMiss;
        let mut marker = Img::new(24, 24, [0; 4]);
        draw_note_judgement(&mut marker, [12.0, 12.0], 12.0, 1.0, event.result);
        assert!(marker.data.iter().all(|&value| value == 0));
    }
}
