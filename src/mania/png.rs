//! osu!mania PNG grid renderer.
//! Port of beatmap_preview/mania/renderer.py.

use crate::canvas::{Img, Rgba};
use crate::composer::save_png;
use crate::errors::Result;
use crate::models::{Beatmap, ManiaHitObject, TimingPoint};
use crate::mods::ModSettings;
use crate::parser::round_half_even;
use crate::text::{draw_text, text_size};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{
    apply_hold_off_mod, apply_inverse_mod, build_sv_changes, darken, is_native_mania,
    mania_objects, resolve_key_count, COLUMN_GAP, IMAGE_BACKGROUND, LANE_BACKGROUND,
    LANE_WIDTH, LEFT_PANEL_BACKGROUND, LEFT_PANEL_WIDTH,
    NOTE_HEAD_HEIGHT, NOTE_SIDE_PADDING, PAGE_MARGIN_X, PAGE_MARGIN_Y, PIXELS_PER_MS,
    RULER_TEXT, SV_TEXT_COLOR, SV_TEXT_FONT_SIZE, TOP_BUFFER,
};

const LANE_GAP: i64 = 0;
const LANE_SEPARATOR: Rgba = [32, 32, 32, 255];
const TIME_LABEL_FONT_SIZE: u32 = 20;
const TIME_LABEL_MIN_INTERVAL_MS: i64 = 2000;

const MAX_AREA_HEIGHT_0_TO_1_MIN: i64 = 4000;
const MAX_AREA_HEIGHT_1_TO_2_MIN: i64 = 5500;
const MAX_AREA_HEIGHT_2_TO_3_MIN: i64 = 7000;
const MAX_AREA_HEIGHT_3_TO_4_MIN: i64 = 8500;
const MAX_AREA_HEIGHT_4_TO_5_MIN: i64 = 10000;
const MAX_AREA_HEIGHT_5_TO_6_MIN: i64 = 11500;
const FIXED_COLUMN_COUNT_6_TO_10_MIN: i64 = 30;
const MAX_SUPPORTED_DURATION_MS: i64 = 10 * 60 * 1000;
const BOTTOM_PADDING_MS: i64 = 2000;

// ── colors (PNG-specific) ──
const MEASURE_LINE: Rgba = [83, 83, 83, 255];
const BEAT_LINE: Rgba = [56, 56, 56, 255];
const SUBDIVISION_LINE: Rgba = [34, 34, 34, 255];

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
    image_width: i64,
    image_height: i64,
    chart_start_time: i64,
}

pub(crate) fn render_mania_grid(
    beatmap: &Beatmap,
    output_path: &Path,
    mods: Option<&ModSettings>,
) -> Result<PathBuf> {
    // key count comes straight from the beatmap CS (mods don't change native mania lanes)
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

    // Trim leading silence: if first note is >= 5s in, start 1s before it.
    let first_note_time = hit_objects.iter().map(|ho| ho.start_time).min().unwrap_or(0);
    let chart_start_time = if first_note_time >= 5000 { (first_note_time - 1000).max(0) } else { 0 };

    if chart_start_time > 0 {
        for ho in &mut hit_objects {
            ho.start_time = (ho.start_time - chart_start_time).max(0);
            ho.end_time = (ho.end_time - chart_start_time).max(ho.start_time);
        }
    }

    let beatmap_duration = hit_objects.iter().map(|ho| ho.end_time).max().unwrap_or(0);
    let chart_end_time = beatmap_duration + BOTTOM_PADDING_MS;
    let timing_points_for_render: Vec<TimingPoint> = if chart_start_time > 0 {
        beatmap.timing_points.iter().map(|tp| {
            let mut tp = *tp;
            tp.time = (tp.time - chart_start_time as f64).max(0.0);
            tp
        }).collect()
    } else {
        beatmap.timing_points.clone()
    };
    let timing_lines = build_timing_lines(
        &timing_points_for_render,
        chart_end_time,
        beatmap.beat_divisor,
        hit_objects.iter().map(|ho| ho.start_time).min().unwrap_or(0),
    );
    let sv_changes = if cs_mode || !native_mania {
        Vec::new()
    } else {
        build_sv_changes(&timing_points_for_render, chart_end_time)
    };
    let layout = build_png_layout(key_count, beatmap_duration, chart_end_time, chart_start_time)?;

    let mut image = Img::new(
        layout.image_width as u32,
        layout.image_height as u32,
        IMAGE_BACKGROUND,
    );

    for column_index in 0..layout.column_count {
        draw_column_background(&mut image, key_count, column_index, &layout);
    }
    let mut last_label_time: Option<i64> = None;
    for timing_line in &timing_lines {
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
    for sv_change in &sv_changes {
        draw_sv_indicator(&mut image, *sv_change, &layout);
    }
    for hit_object in &hit_objects {
        draw_png_hit_object(&mut image, hit_object, &palette, &layout);
    }

    save_png(&image, output_path)?;
    Ok(output_path.to_path_buf())
}

fn build_png_layout(
    key_count: i32,
    beatmap_duration: i64,
    chart_end_time: i64,
    chart_start_time: i64,
) -> Result<RenderLayout> {
    let total_chart_height = ((chart_end_time as f64 * PIXELS_PER_MS).ceil() as i64).max(1);
    let column_count = calculate_column_count(beatmap_duration, total_chart_height)?;
    let time_per_column = ceil_div(chart_end_time, column_count);
    let column_height = (time_per_column as f64 * PIXELS_PER_MS).ceil() as i64;
    let total_column_height = TOP_BUFFER + column_height;
    let lane_area_width = key_count as i64 * LANE_WIDTH + (key_count as i64 - 1) * LANE_GAP;
    let column_width = LEFT_PANEL_WIDTH + lane_area_width;
    let image_width = PAGE_MARGIN_X * 2 + column_count * column_width + column_count * COLUMN_GAP;
    let image_height = PAGE_MARGIN_Y * 2 + total_column_height;
    Ok(RenderLayout {
        column_count,
        time_per_column,
        column_height,
        total_column_height,
        lane_area_width,
        column_width,
        image_width,
        image_height,
        chart_start_time,
    })
}

fn ceil_div(a: i64, b: i64) -> i64 {
    (a + b - 1).div_euclid(b)
}

fn calculate_column_count(beatmap_duration: i64, total_chart_height: i64) -> Result<i64> {
    if beatmap_duration >= MAX_SUPPORTED_DURATION_MS {
        return Err(crate::errors::PreviewError::new(
            "songs longer than 10 minutes are not supported",
        ));
    }
    if beatmap_duration >= 6 * 60 * 1000 {
        return Ok(FIXED_COLUMN_COUNT_6_TO_10_MIN);
    }
    let max_area_height = resolve_max_area_height(beatmap_duration);
    Ok(ceil_div(total_chart_height, max_area_height).max(1))
}

fn resolve_max_area_height(beatmap_duration: i64) -> i64 {
    if beatmap_duration < 60 * 1000 {
        MAX_AREA_HEIGHT_0_TO_1_MIN
    } else if beatmap_duration < 2 * 60 * 1000 {
        MAX_AREA_HEIGHT_1_TO_2_MIN
    } else if beatmap_duration < 3 * 60 * 1000 {
        MAX_AREA_HEIGHT_2_TO_3_MIN
    } else if beatmap_duration < 4 * 60 * 1000 {
        MAX_AREA_HEIGHT_3_TO_4_MIN
    } else if beatmap_duration < 5 * 60 * 1000 {
        MAX_AREA_HEIGHT_4_TO_5_MIN
    } else {
        MAX_AREA_HEIGHT_5_TO_6_MIN
    }
}

fn draw_column_background(
    image: &mut Img,
    key_count: i32,
    column_index: i64,
    layout: &RenderLayout,
) {
    let column_left = PAGE_MARGIN_X + column_index * (layout.column_width + COLUMN_GAP);
    let chart_top = PAGE_MARGIN_Y;
    let lane_area_left = column_left + LEFT_PANEL_WIDTH;

    image.set_rect(
        column_left,
        chart_top,
        lane_area_left,
        chart_top + layout.total_column_height,
        LEFT_PANEL_BACKGROUND,
    );

    for lane_index in 0..key_count as i64 {
        let lane_left = lane_area_left + lane_index * (LANE_WIDTH + LANE_GAP);
        let lane_right = lane_left + LANE_WIDTH;
        image.set_rect(
            lane_left,
            chart_top,
            lane_right,
            chart_top + layout.total_column_height,
            LANE_BACKGROUND,
        );
        if lane_index > 0 {
            image.set_rect(
                lane_left,
                chart_top,
                lane_left,
                chart_top + layout.total_column_height,
                LANE_SEPARATOR,
            );
        }
    }
}

fn draw_timing_line(image: &mut Img, timing_line: &TimingLine, layout: &RenderLayout) {
    let column_index =
        (timing_line.time.div_euclid(layout.time_per_column)).min(layout.column_count - 1);
    let local_time = timing_line.time - column_index * layout.time_per_column;
    let column_left = PAGE_MARGIN_X + column_index * (layout.column_width + COLUMN_GAP);
    let lane_area_left = column_left + LEFT_PANEL_WIDTH;
    let chart_top = PAGE_MARGIN_Y + TOP_BUFFER;
    let y = chart_top + layout.column_height - round_half_even(local_time as f64 * PIXELS_PER_MS);

    image.set_rect(
        lane_area_left,
        y,
        lane_area_left + layout.lane_area_width - 1,
        y,
        timing_line.color,
    );

    if timing_line.show_label {
        let label = format!("{:.1}s", (timing_line.time + layout.chart_start_time) as f64 / 1000.0);
        let (label_width, label_height) = text_size(&label, TIME_LABEL_FONT_SIZE);
        let label_width = label_width as i64;
        let text_mid_y = label_height as f64 / 2.0;
        let mut label_x = column_left + layout.column_width + 4;
        if column_index < layout.column_count - 1 {
            let next_column_left = column_left + layout.column_width + COLUMN_GAP;
            label_x = label_x.min(next_column_left - label_width - 4);
        } else {
            label_x = label_x.min(layout.image_width - PAGE_MARGIN_X - label_width);
        }
        let label_y = (chart_top as f64).max(y as f64 - text_mid_y).floor() as i64;
        draw_text(
            image,
            label_x,
            label_y,
            &label,
            TIME_LABEL_FONT_SIZE,
            RULER_TEXT,
        );

        if let Some(ref bpm_label) = timing_line.bpm_label {
            let (bpm_w, bpm_h) = text_size(bpm_label, TIME_LABEL_FONT_SIZE);
            let bpm_w = bpm_w as i64;
            let mut bpm_x = column_left + layout.column_width + 4;
            if column_index < layout.column_count - 1 {
                let next_column_left = column_left + layout.column_width + COLUMN_GAP;
                bpm_x = bpm_x.min(next_column_left - bpm_w - 4);
            } else {
                bpm_x = bpm_x.min(layout.image_width - PAGE_MARGIN_X - bpm_w);
            }
            let bpm_y = (label_y + label_height as i64 + 3).min(PAGE_MARGIN_Y + layout.total_column_height - bpm_h as i64);
            draw_text(image, bpm_x, bpm_y, bpm_label, TIME_LABEL_FONT_SIZE, RULER_TEXT);
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
        let column_left = PAGE_MARGIN_X + column_index * (layout.column_width + COLUMN_GAP);
        let lane_area_left = column_left + LEFT_PANEL_WIDTH;
        let chart_top = PAGE_MARGIN_Y;
        let chart_axis_top = chart_top + TOP_BUFFER;
        let chart_bottom = chart_axis_top + layout.column_height;
        let lane_left = lane_area_left + lane as i64 * (LANE_WIDTH + LANE_GAP) + NOTE_SIDE_PADDING;
        let lane_right = lane_left + LANE_WIDTH - NOTE_SIDE_PADDING * 2;
        let segment_start = hit_object
            .start_time
            .max(column_index * layout.time_per_column);
        let segment_end = hit_object
            .end_time
            .min((column_index + 1) * layout.time_per_column);
        let y_start = chart_axis_top + layout.column_height
            - round_half_even(
                (segment_start - column_index * layout.time_per_column) as f64 * PIXELS_PER_MS,
            );
        let y_end = chart_axis_top + layout.column_height
            - round_half_even(
                (segment_end - column_index * layout.time_per_column) as f64 * PIXELS_PER_MS,
            );

        if hit_object.is_long_note {
            let body_top = chart_top.max(y_end.min(y_start - NOTE_HEAD_HEIGHT));
            let body_bottom = chart_bottom.min(y_start);
            if body_top < body_bottom {
                image.set_rect(lane_left, body_top, lane_right, body_bottom, hold_color);
            }
            if column_index == start_column {
                let head_top = chart_top.max(y_start - NOTE_HEAD_HEIGHT);
                let head_bottom = chart_bottom.min(y_start);
                if head_top < head_bottom {
                    image.set_rect(lane_left, head_top, lane_right, head_bottom, lane_color);
                }
            }
        } else {
            let head_top = chart_top.max(y_start - NOTE_HEAD_HEIGHT);
            let head_bottom = chart_bottom.min(y_start);
            if head_top < head_bottom {
                image.set_rect(lane_left, head_top, lane_right, head_bottom, lane_color);
            }
        }
    }
}

fn build_timing_lines(timing_points: &[TimingPoint], chart_end_time: i64, beat_divisor: i32, first_note_time: i64) -> Vec<TimingLine> {
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

        let beat_pixels = point.beat_length * PIXELS_PER_MS;
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
        // NaN steps fall through (single line emitted, like Python); zero/negative
        // steps would loop forever, so skip them.
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
                            MEASURE_LINE
                        } else if is_beat {
                            BEAT_LINE
                        } else {
                            SUBDIVISION_LINE
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
    // Attach BPM labels: at each red line's first bar line when BPM changes,
    // and at the bar line nearest the first note.
    if !ordered_unique.is_empty() {
        let mut last_bpm: Option<f64> = None;
        for point in &base_points {
            let bpm = 60_000.0 / point.beat_length;
            let bpm_changed = last_bpm.map_or(true, |prev| (bpm - prev).abs() > 0.01);
            last_bpm = Some(bpm);

            if bpm_changed {
                // Find the first bar line at or after this red line's time.
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

        // First note BPM: use the current BPM at first_note_time.
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
                    // Only attach if not already labelled by a BPM change at the same time.
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
    let column_left = PAGE_MARGIN_X + column_index * (layout.column_width + COLUMN_GAP);
    let chart_top = PAGE_MARGIN_Y + TOP_BUFFER;
    let y = chart_top + layout.column_height - round_half_even(local_time as f64 * PIXELS_PER_MS);

    let label = super::format_sv_label(sv);
    let (label_width, label_height) = text_size(&label, SV_TEXT_FONT_SIZE);
    let text_mid_y = label_height as f64 / 2.0;
    let label_x = (column_left - 1 - label_width as i64).max(0);
    let label_y = (chart_top as f64).max(y as f64 - text_mid_y).floor() as i64;
    draw_text(
        image,
        label_x,
        label_y,
        &label,
        SV_TEXT_FONT_SIZE,
        SV_TEXT_COLOR,
    );
}
