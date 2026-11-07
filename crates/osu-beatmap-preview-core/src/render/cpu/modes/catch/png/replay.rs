//! Catch 静态回放：在谱面时间轴上叠加真实接盘轨迹、漏接边界及稀疏成绩快照。

use super::*;
use crate::{
    gameplay::{
        events::{EventKind, GameplayEvent, HitResult},
        state::GameplayState,
        timeline::GameplayTimeline,
    },
    render::{
        geometry::{output_scale, GameMode, OutputFormat},
        shared::replay::colors,
    },
    replay::preparation::catch::PreparedCatch,
};

const PATH_COLOR: [u8; 4] = [252, 97, 143, 230];
const DROPLET_COLOR: [u8; 4] = colors::BLUE;
const MISS_BAND_COLOR: [u8; 4] = colors::with_alpha(MISS_COLOR, 72);
const DROPLET_BAND_COLOR: [u8; 4] = colors::with_alpha(DROPLET_COLOR, 72);
const STATE_FONT_SIZE: u32 = 24;
const STATE_PADDING: f64 = 6.0;
const MISS_COLOR: [u8; 4] = colors::hit_result(HitResult::Miss);
const BANANA_HIT_COLOR: [u8; 4] = colors::hit_result(HitResult::LargeBonus);
const MARKER_BACKGROUND: [u8; 4] = [20, 8, 14, 255];
pub fn render_scene(
    layout: &CatchPngLayout,
    render_objects: &[RenderObject],
    timing_lines: &[TimingLine],
    time_axis: TimeAxis,
    prepared: &PreparedCatch,
    gameplay: &GameplayTimeline,
    chart_end_time: i64,
) -> Img {
    let mut image = render_background(layout, render_objects, timing_lines, time_axis);
    draw_miss_bands(
        &mut image,
        layout,
        prepared,
        gameplay,
        chart_end_time as f64,
    );
    draw_objects(&mut image, render_objects, layout);
    let mut seed = 1337_u32.to_le_bytes();
    let _ = getrandom::getrandom(&mut seed);
    let labels = select_states(
        layout,
        gameplay,
        chart_end_time as f64,
        u32::from_le_bytes(seed),
    );
    draw_states(&mut image, layout, &labels);
    // 判定边框叠在原物件之上，真实接盘轨迹与范围线最后绘制。
    draw_judgements(
        &mut image,
        layout,
        prepared,
        gameplay,
        chart_end_time as f64,
    );
    draw_path(&mut image, layout, gameplay, chart_end_time as f64);
    draw_miss_plates(
        &mut image,
        layout,
        prepared,
        gameplay,
        chart_end_time as f64,
    );
    image
}

fn scale() -> f64 {
    output_scale(GameMode::Catch, OutputFormat::Png)
}

fn draw_path(image: &mut Img, layout: &CatchPngLayout, gameplay: &GameplayTimeline, end: f64) {
    let start = layout.chart_start_time as f64;
    let end = end.min(gameplay.settlement_time);
    if end <= start {
        return;
    }
    let frames = &gameplay.input.frames;
    let first = frames
        .partition_point(|frame| (frame.time_ms as f64) < start)
        .saturating_sub(1);
    let last = frames.partition_point(|frame| (frame.time_ms as f64) <= end);
    for pair in frames[first..last.saturating_add(1).min(frames.len())].windows(2) {
        let a = pair[0].time_ms as f64;
        let b = pair[1].time_ms as f64;
        let from = a.max(start);
        let to = b.min(end);
        if to <= from {
            continue;
        }
        let x0 = f64::from(pair[0].cursor[0]);
        let x1 = f64::from(pair[1].cursor[0]);
        // 先插值再限制盘子位置；越界输入在场地边缘保留平台段。
        let mut times = [from, to, 0.0, 0.0];
        let mut time_count = 2;
        if x1 != x0 {
            for edge in [0.0, 512.0] {
                let crossing = a + (edge - x0) / (x1 - x0) * (b - a);
                if crossing > from && crossing < to {
                    times[time_count] = crossing;
                    time_count += 1;
                }
            }
        }
        let times = &mut times[..time_count];
        times.sort_by(f64::total_cmp);
        for span in times.windows(2) {
            let x_at = |time: f64| (x0 + (x1 - x0) * (time - a) / (b - a)).clamp(0.0, 512.0);
            for (p, q) in time_axis_segments(
                span[0] - start,
                x_at(span[0]),
                span[1] - start,
                x_at(span[1]),
                layout,
            ) {
                image.draw_line(p.0, p.1, q.0, q.1, (1.5 * scale()).max(1.0), PATH_COLOR);
            }
        }
    }
}

fn catcher_x_at(gameplay: &GameplayTimeline, time: f64) -> Option<f64> {
    let frames = &gameplay.input.frames;
    let next = frames.partition_point(|frame| frame.time_ms as f64 <= time);
    let previous = next.checked_sub(1).and_then(|index| frames.get(index));
    let x = match (previous, frames.get(next)) {
        (Some(a), Some(b)) if b.time_ms > a.time_ms => {
            let progress = (time - a.time_ms as f64) / (b.time_ms - a.time_ms) as f64;
            f64::from(a.cursor[0]) + f64::from(b.cursor[0] - a.cursor[0]) * progress
        }
        (Some(frame), _) | (_, Some(frame)) => f64::from(frame.cursor[0]),
        _ => return None,
    };
    x.is_finite().then(|| x.clamp(0.0, 512.0))
}

fn miss_radius(object: &RenderObject, layout: &CatchPngLayout) -> f64 {
    (super::super::drawing::object_diameter(
        layout.object_scale,
        layout.playfield_scale,
        object.scale_factor,
    ) / 2.0
        + 4.0 * scale())
    .max(8.0 * scale())
}

fn visible_events(gameplay: &GameplayTimeline, start: i64, end: f64) -> &[GameplayEvent] {
    let first = gameplay
        .events
        .partition_point(|event| event.time < start as f64);
    let last = gameplay.events.partition_point(|event| event.time <= end);
    &gameplay.events[first..last.max(first)]
}

fn draw_miss_bands(
    image: &mut Img,
    layout: &CatchPngLayout,
    prepared: &PreparedCatch,
    gameplay: &GameplayTimeline,
    end: f64,
) {
    let sizing = catch_constants::png::sizing();
    let top = sizing.PAGE_MARGIN_TOP + sizing.INFO_MARGIN_TOP;
    let height = layout.total_column_height as usize;
    // 每行、每种漏接只混合一次，避免密集小水滴反复叠色后遮掉红色；内存仅随时间轴高度增长。
    let mut coverage = vec![0_u8; height * layout.column_count as usize];
    for event in visible_events(gameplay, layout.chart_start_time, end) {
        if event.kind != EventKind::Judgement || event.result.is_hit() {
            continue;
        }
        let object = &prepared.objects[event.object_id];
        if object.object_type == ObjType::Banana {
            continue;
        }
        let (column, y) = locate_time(rhe(event.time - layout.chart_start_time as f64), layout);
        let radius = rhe(miss_radius(object, layout));
        let from = (y - radius - top).clamp(0, height as i64) as usize;
        let to = (y + radius - top).clamp(0, height as i64) as usize;
        let flag = if object.object_type == ObjType::TinyDroplet {
            2
        } else {
            1
        };
        let offset = column as usize * height;
        for row in &mut coverage[offset + from..offset + to] {
            *row |= flag;
        }
    }
    for column in 0..layout.column_count {
        let left = playfield_left(column, layout).max(0);
        let right = (left + layout.visible_playfield_width).min(i64::from(image.w));
        let offset = column as usize * height;
        for (row, &flag) in coverage[offset..offset + height].iter().enumerate() {
            if flag == 0 {
                continue;
            }
            let y = top + row as i64;
            for x in left..right {
                if flag & 1 != 0 {
                    image.blend_px(x, y, MISS_BAND_COLOR);
                }
                if flag & 2 != 0 {
                    image.blend_px(x, y, DROPLET_BAND_COLOR);
                }
            }
            let panel_left = column_left(column, layout);
            let panel_width = sizing.LEFT_PANEL_WIDTH.max(1);
            if flag & 1 != 0 {
                image.fill_rect_size(panel_left, y, panel_width, 1, MISS_COLOR);
            }
            if flag & 2 != 0 {
                let split = if flag & 1 != 0 { panel_width / 2 } else { 0 };
                image.fill_rect_size(panel_left + split, y, panel_width - split, 1, DROPLET_COLOR);
            }
        }
    }
}

fn draw_banana_hit(
    image: &mut Img,
    object: &RenderObject,
    layout: &CatchPngLayout,
    x: f64,
    y: f64,
) {
    let radius = super::super::drawing::object_diameter(
        layout.object_scale,
        layout.playfield_scale,
        object.scale_factor,
    ) / 2.0;
    // 仅填充空心香蕉的中心，保留原有外圈。
    image.fill_circle_aa(x, y, radius * 0.72, BANANA_HIT_COLOR);
}

fn draw_note_judgement(image: &mut Img, x: f64, y: f64, radius: f64, color: [u8; 4]) {
    // 覆盖原物件的白色内边框，保留圆形主体与填充颜色。
    image.stroke_circle_aa(x, y, radius, radius * 0.2, color);
}

fn draw_judgements(
    image: &mut Img,
    layout: &CatchPngLayout,
    prepared: &PreparedCatch,
    gameplay: &GameplayTimeline,
    end: f64,
) {
    let events = visible_events(gameplay, layout.chart_start_time, end)
        .iter()
        .filter(|event| {
            event.kind == EventKind::Judgement
                && (!event.result.is_hit() || event.result == HitResult::LargeBonus)
        });
    // 次级标记先画，断连 Miss 最后画；密集水果串中小水滴不能覆盖真正的 Miss。
    let primary =
        |event: &&GameplayEvent| matches!(event.result, HitResult::Miss | HitResult::LargeTickMiss);
    for event in events
        .clone()
        .filter(|event| !primary(event))
        .chain(events.filter(primary))
    {
        let object = &prepared.objects[event.object_id];
        let (column, y) = locate_time(rhe(event.time - layout.chart_start_time as f64), layout);
        let left = playfield_left(column, layout) as f64;
        let x = left + object.x * layout.playfield_scale;
        if object.object_type == ObjType::Banana {
            if event.result.is_hit() {
                draw_banana_hit(image, object, layout, x, y as f64);
            }
            continue;
        }
        if event.result.is_hit() {
            continue;
        }
        let radius = super::super::drawing::object_diameter(
            layout.object_scale,
            layout.playfield_scale,
            object.scale_factor,
        ) / 2.0;
        let color = if object.object_type == ObjType::TinyDroplet {
            DROPLET_COLOR
        } else {
            colors::hit_result(event.result)
        };
        draw_note_judgement(image, x, y as f64, radius, color);
    }
}

fn draw_miss_plates(
    image: &mut Img,
    layout: &CatchPngLayout,
    prepared: &PreparedCatch,
    gameplay: &GameplayTimeline,
    end: f64,
) {
    // 盘子统一在最上层绘制，远距离 Miss 也保留；相邻判定不能覆盖前一个盘子。
    for event in visible_events(gameplay, layout.chart_start_time, end) {
        if event.kind != EventKind::Judgement
            || !matches!(event.result, HitResult::Miss | HitResult::LargeTickMiss)
        {
            continue;
        }
        let (column, y) = locate_time(rhe(event.time - layout.chart_start_time as f64), layout);
        let left = playfield_left(column, layout) as f64;
        let catcher_x = event
            .position
            .map(|position| f64::from(position[0]))
            .filter(|x| x.is_finite())
            .or_else(|| catcher_x_at(gameplay, event.time));
        let plate_range = catcher_x.map(|x| {
            let x = x.clamp(0.0, 512.0);
            (
                (x - prepared.half_catch_width).max(0.0),
                (x + prepared.half_catch_width).min(512.0),
            )
        });
        if let Some((x0, x1)) = plate_range {
            // 接盘范围线最后画，压在音符和判定边框上，便于比较实际差距。
            for (width, color) in [(7.0, MARKER_BACKGROUND), (4.0, PATH_COLOR)] {
                image.draw_line(
                    left + x0 * layout.playfield_scale,
                    y as f64,
                    left + x1 * layout.playfield_scale,
                    y as f64,
                    (width * scale()).max(1.0),
                    color,
                );
            }
        }
    }
}

fn select_states<'a>(
    layout: &CatchPngLayout,
    gameplay: &'a GameplayTimeline,
    end: f64,
    seed: u32,
) -> Vec<(f64, &'a GameplayState)> {
    crate::render::shared::replay::static_state::select_states(
        gameplay,
        layout.chart_start_time as f64,
        end,
        seed,
        scale(),
        |time| locate_time(rhe(time - layout.chart_start_time as f64), layout),
        Default::default(),
    )
}

fn state_font() -> u32 {
    crate::render::text::scaled_bitmap_font_height(STATE_FONT_SIZE, scale())
}

pub fn state_panel_width() -> i64 {
    let font = state_font();
    let padding = crate::render::geometry::scale_px(STATE_PADDING, scale()).max(1);
    i64::from(text_size("100.00%", font).0 + text_size("SS", font).0) + padding * 7
}

fn draw_states(image: &mut Img, layout: &CatchPngLayout, states: &[(f64, &GameplayState)]) {
    let sizing = catch_constants::png::sizing();
    let font = state_font();
    let padding = crate::render::geometry::scale_px(STATE_PADDING, scale()).max(1);
    let top = sizing.PAGE_MARGIN_TOP + sizing.INFO_MARGIN_TOP;
    for (time, state) in states {
        let (column, y) = locate_time(rhe(*time - layout.chart_start_time as f64), layout);
        let x = column_left(column, layout) + sizing.COLUMN_WIDTH + sizing.INFO_MARGIN_RIGHT;
        let label_y = (y - i64::from(font) / 2).clamp(
            top + padding,
            (top + layout.total_column_height - i64::from(font) - padding).max(top + padding),
        );
        crate::render::shared::replay::static_state::draw_badge(
            image,
            x,
            label_y,
            state,
            font,
            padding,
            scale(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::shared::replay::static_state::rank_color;
    use crate::{
        gameplay::{events::HitResult, state::ReplayStatus},
        replay::{ReplayFrame, ReplayTimeline},
    };
    use std::sync::Arc;

    fn fixture() -> (PreparedCatch, GameplayTimeline) {
        let map = crate::parse_beatmap_bytes(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../osu-beatmap-preview-web/test/fixtures/replay-catch.osu"
        )))
        .unwrap();
        let document = Arc::new(
            crate::replay::parse_osr(include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../osu-beatmap-preview-web/test/fixtures/replay-catch.osr"
            )))
            .unwrap(),
        );
        let prepared = crate::replay::preparation::catch::prepare_replay(&map, &document).unwrap();
        let gameplay = crate::gameplay::engine::compile_catch(&prepared, document).unwrap();
        (prepared, gameplay)
    }

    fn layout() -> CatchPngLayout {
        let scale = scale();
        CatchPngLayout {
            column_count: 3,
            total_column_height: rhe(200.0 * scale),
            visible_playfield_width: rhe(260.0 * scale),
            image_width: rhe(1400.0 * scale),
            image_height: rhe(240.0 * scale),
            playfield_scale: 260.0 * scale / 512.0,
            object_scale: 0.5,
            pixels_per_ms: 0.1 * scale,
            chart_start_time: 1000,
            state_panel_width: 0,
        }
    }

    fn blank(layout: &CatchPngLayout) -> Img {
        Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            [0; 4],
        )
    }

    #[test]
    fn path_clips_window_splits_columns_and_stops_at_failure() {
        let (_, mut gameplay) = fixture();
        gameplay.input = ReplayTimeline::new(
            [(900, -100.0), (3100, 600.0), (5000, 256.0)]
                .into_iter()
                .enumerate()
                .map(|(sequence, (time_ms, x))| ReplayFrame {
                    time_ms,
                    cursor: [x, 0.0],
                    sequence,
                    delta_ms: 0,
                    keys: 0,
                })
                .collect(),
        );
        gameplay.settlement_time = 3500.0;
        gameplay.failure_time = Some(3500.0);
        for png_scale in [0.75, 1.25] {
            let mut config = crate::config::CoreConfig::default();
            config.render.catch.png.SCALE = png_scale;
            config.render.catch.gif.SCALE = 2.0;
            config.render.catch.mp4.SCALE = 0.5;
            crate::config::with_config(Arc::new(config), || {
                let layout = layout();
                let mut image = blank(&layout);
                draw_path(&mut image, &layout, &gameplay, 7000.0);
                for (time, x) in [(1100, 0.0_f64), (2900, 512.0), (3200, 581.894736842)] {
                    let (column, y) = locate_time(time - layout.chart_start_time, &layout);
                    let x = playfield_left(column, &layout) as f64
                        + x.clamp(0.0, 512.0) * layout.playfield_scale;
                    assert!(image.get(rhe(x) as u32, y as u32)[3] > 0);
                }
                let gutter = playfield_left(0, &layout)
                    + layout.visible_playfield_width
                    + rhe(20.0 * png_scale);
                assert!((0..image.h).all(|y| image.get(gutter as u32, y) == [0; 4]));
                let (_, after_failure_y) = locate_time(2800, &layout);
                assert!((playfield_left(1, &layout)
                    ..playfield_left(1, &layout) + layout.visible_playfield_width)
                    .all(|x| image.get(x as u32, after_failure_y as u32) == [0; 4]));
                let mut after_failure = layout;
                after_failure.chart_start_time = 4000;
                let mut image = blank(&after_failure);
                draw_path(&mut image, &after_failure, &gameplay, 7000.0);
                assert!(image.data.iter().all(|v| *v == 0));
            });
        }
    }

    #[test]
    fn judgement_borders_preserve_centers_banana_fill_and_miss_plates() {
        let (mut prepared, mut gameplay) = fixture();
        let layout = layout();
        let mut event = gameplay.events[0].clone();
        event.time = 2100.0;
        event.kind = EventKind::Judgement;
        for frame in Arc::make_mut(&mut gameplay.input.frames) {
            frame.cursor[0] = 0.0;
        }

        for (kind, miss) in [
            (ObjType::Fruit, HitResult::Miss),
            (ObjType::Droplet, HitResult::LargeTickMiss),
            (ObjType::TinyDroplet, HitResult::SmallTickMiss),
            (ObjType::Banana, HitResult::IgnoreMiss),
        ] {
            let id = prepared
                .objects
                .iter()
                .position(|o| o.object_type == kind)
                .unwrap();
            prepared.objects[id].x = 256.0;
            let near = 256.0 + prepared.half_catch_width + 20.0;
            event.object_id = id;
            event.result = miss;
            event.position = Some([near as f32, 0.0]);
            gameplay.events = vec![event.clone()];
            let mut image = blank(&layout);
            draw_judgements(&mut image, &layout, &prepared, &gameplay, 7000.0);
            draw_miss_plates(&mut image, &layout, &prepared, &gameplay, 7000.0);
            let (column, y) = locate_time(1100, &layout);
            let plate_x =
                rhe(playfield_left(column, &layout) as f64 + near * layout.playfield_scale);
            assert_eq!(
                image.get(plate_x as u32, y as u32),
                if matches!(kind, ObjType::Banana | ObjType::TinyDroplet) {
                    [0; 4]
                } else {
                    PATH_COLOR
                }
            );
            if kind == ObjType::Banana {
                gameplay.events[0].result = HitResult::LargeBonus;
                let mut image = blank(&layout);
                draw_judgements(&mut image, &layout, &prepared, &gameplay, 7000.0);
                draw_miss_plates(&mut image, &layout, &prepared, &gameplay, 7000.0);
                let banana_x =
                    rhe(playfield_left(column, &layout) as f64 + 256.0 * layout.playfield_scale);
                assert!(image
                    .data
                    .chunks_exact(4)
                    .any(|pixel| pixel == BANANA_HIT_COLOR));
                assert!(!image
                    .data
                    .chunks_exact(4)
                    .any(|pixel| pixel == MARKER_BACKGROUND));
                assert!(!image.data.chunks_exact(4).any(|pixel| pixel == [255; 4]));
                assert_ne!(image.get(banana_x as u32, y as u32), [0; 4]);
                assert_eq!(image.get(plate_x as u32, y as u32), [0; 4]);
            } else if kind == ObjType::TinyDroplet {
                let mut image = blank(&layout);
                draw_miss_bands(&mut image, &layout, &prepared, &gameplay, 7000.0);
                assert_eq!(
                    image.get(column_left(column, &layout) as u32, y as u32),
                    DROPLET_COLOR
                );
                assert_eq!(image.get(plate_x as u32, y as u32), DROPLET_BAND_COLOR);
                draw_judgements(&mut image, &layout, &prepared, &gameplay, 7000.0);
                draw_miss_plates(&mut image, &layout, &prepared, &gameplay, 7000.0);
                let object_x =
                    rhe(playfield_left(column, &layout) as f64 + 256.0 * layout.playfield_scale);
                assert_eq!(image.get(object_x as u32, y as u32), DROPLET_BAND_COLOR);
                for scale in [0.75, 1.25] {
                    let mut note = Img::new(80, 80, [123, 45, 67, 255]);
                    let radius = 16.0 * scale;
                    draw_note_judgement(&mut note, 40.0, 40.0, radius, DROPLET_COLOR);
                    assert_eq!(note.get(40, 40), [123, 45, 67, 255]);
                    assert_eq!(note.get((40.0 + radius * 0.9) as u32, 40), DROPLET_COLOR);
                    assert_eq!(note.get((40.0 + radius) as u32 + 2, 40), [123, 45, 67, 255]);
                }
            } else {
                // 远距离漏接仍画盘子；缺少判定位置时从回放帧补出位置。
                for position in [Some([0.0, 0.0]), None] {
                    gameplay.events[0].position = position;
                    let mut image = blank(&layout);
                    draw_miss_bands(&mut image, &layout, &prepared, &gameplay, 7000.0);
                    assert_eq!(
                        image.get(column_left(column, &layout) as u32, y as u32),
                        MISS_COLOR
                    );
                    draw_judgements(&mut image, &layout, &prepared, &gameplay, 7000.0);
                    draw_miss_plates(&mut image, &layout, &prepared, &gameplay, 7000.0);
                    let object_x = rhe(
                        playfield_left(column, &layout) as f64 + 256.0 * layout.playfield_scale
                    );
                    assert_eq!(image.get(object_x as u32, y as u32), MISS_BAND_COLOR);
                    assert_eq!(
                        image.get((playfield_left(column, &layout) + 5) as u32, y as u32),
                        PATH_COLOR
                    );
                }
            }
        }
        let mut primary = event.clone();
        primary.object_id = prepared
            .objects
            .iter()
            .position(|o| o.object_type == ObjType::Fruit)
            .unwrap();
        primary.result = HitResult::Miss;
        primary.position = None;
        let mut secondary = primary.clone();
        secondary.object_id = prepared
            .objects
            .iter()
            .position(|o| o.object_type == ObjType::TinyDroplet)
            .unwrap();
        secondary.result = HitResult::SmallTickMiss;
        gameplay.events = vec![primary, secondary];
        let mut image = blank(&layout);
        draw_judgements(&mut image, &layout, &prepared, &gameplay, 7000.0);
        draw_miss_plates(&mut image, &layout, &prepared, &gameplay, 7000.0);
        let (column, y) = locate_time(1100, &layout);
        let object_x = rhe(playfield_left(column, &layout) as f64 + 256.0 * layout.playfield_scale);
        assert_eq!(image.get(object_x as u32, y as u32), [0; 4]);
        let mut bands = blank(&layout);
        draw_miss_bands(&mut bands, &layout, &prepared, &gameplay, 7000.0);
        let x = playfield_left(column, &layout) as u32;
        let mut expected = Img::new(1, 1, [0; 4]);
        expected.blend_px(0, 0, MISS_BAND_COLOR);
        expected.blend_px(0, 0, DROPLET_BAND_COLOR);
        assert_eq!(bands.get(x, y as u32), expected.get(0, 0));
        let left = column_left(column, &layout) as u32;
        assert_eq!(bands.get(left, y as u32), MISS_COLOR);
        assert_eq!(
            bands.get(
                left + catch_constants::png::sizing().LEFT_PANEL_WIDTH as u32 - 1,
                y as u32
            ),
            DROPLET_COLOR
        );
        gameplay
            .events
            .push(gameplay.events.last().unwrap().clone());
        let mut repeated = blank(&layout);
        draw_miss_bands(&mut repeated, &layout, &prepared, &gameplay, 7000.0);
        assert_eq!(bands.data, repeated.data);
    }

    #[test]
    fn state_samples_are_sparse_random_and_keep_final_failure() {
        let (_, mut gameplay) = fixture();
        let mut layout = layout();
        layout.pixels_per_ms = 0.01;
        gameplay.checkpoints = (0..60)
            .map(|i| {
                (
                    1000.0 + i as f64 * 1000.0,
                    GameplayState {
                        accuracy: Some(0.99 - i as f64 * 0.001),
                        rank: Some("A".into()),
                        status: if i == 59 {
                            ReplayStatus::Failed
                        } else {
                            ReplayStatus::Playing
                        },
                        ..Default::default()
                    },
                )
            })
            .collect();
        let a = select_states(&layout, &gameplay, 60000.0, 1);
        let b = select_states(&layout, &gameplay, 60000.0, 2);
        assert_eq!(a.last().unwrap().1.status, ReplayStatus::Failed);
        assert_ne!(
            a.iter().map(|p| p.0).collect::<Vec<_>>(),
            b.iter().map(|p| p.0).collect::<Vec<_>>()
        );
        assert!(a.windows(2).all(|pair| pair[1].0 - pair[0].0 >= 5000.0));
        for png_scale in [0.75, 1.0, 1.25] {
            let mut config = crate::config::CoreConfig::default();
            config.render.catch.png.SCALE = png_scale;
            crate::config::with_config(Arc::new(config), || {
                let mut panel_layout = self::layout();
                panel_layout.state_panel_width = state_panel_width();
                panel_layout.image_width +=
                    panel_layout.column_count * panel_layout.state_panel_width;
                let sizing = catch_constants::png::sizing();
                let panel_x =
                    column_left(0, &panel_layout) + sizing.COLUMN_WIDTH + sizing.INFO_MARGIN_RIGHT;
                let mut image = blank(&panel_layout);
                draw_states(&mut image, &panel_layout, &[(2100.0, a[0].1)]);
                assert!(state_font() > sizing.TIME_LABEL_FONT_SIZE);
                let text_left = panel_x
                    + crate::render::geometry::scale_px(STATE_PADDING, png_scale).max(1) * 2;
                let text_right = text_left + i64::from(text_size("99.00%", state_font()).0);
                assert!(image.data.chunks_exact(4).enumerate().any(|(i, pixel)| {
                    let x = (i % image.w as usize) as i64;
                    x >= text_left && x < text_right && pixel == rank_color("A")
                }));
                assert!(image
                    .data
                    .chunks_exact(4)
                    .any(|pixel| pixel == rank_color("A")));
                assert!(image
                    .data
                    .chunks_exact(4)
                    .enumerate()
                    .filter(|(_, pixel)| *pixel != [0; 4])
                    .all(|(i, _)| {
                        let x = (i % image.w as usize) as i64;
                        x >= panel_x && x < panel_x + panel_layout.state_panel_width
                    }));
            });
        }
        for column in 0..layout.column_count {
            let labels = a
                .iter()
                .filter(|p| locate_time(rhe(p.0 - 1000.0), &layout).0 == column)
                .collect::<Vec<_>>();
            assert!(labels.len() <= 4);
            assert!(labels.windows(2).all(|pair| {
                let y0 = locate_time(rhe(pair[0].0 - 1000.0), &layout).1;
                let y1 = locate_time(rhe(pair[1].0 - 1000.0), &layout).1;
                (y0 - y1).abs() as f64 >= 56.0 * scale()
            }));
        }
    }
}
