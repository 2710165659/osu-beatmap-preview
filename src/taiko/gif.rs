//! osu!taiko GIF renderer: 4-segment animated preview with Overlapping scroll.

use crate::canvas::Img;
use crate::composer;
use crate::errors::{PreviewError, Result};
use crate::models::{Beatmap, TaikoHitObject, TimingPoint};
use crate::mods::ModSettings;
use crate::parser::round_half_even;
use crate::text::{draw_text, text_size};
use crate::common::time_selection::{PreviewSegmentTiming, PreviewTimeSelector};
use std::cell::RefCell;
use std::path::Path;

use super::constants::*;
use super::notes::{
    cached_note_disc, cached_roll_tail, draw_drum_panel, draw_note_disc, draw_track_background,
    paste_clipped, RenderCache, DRUM_PANEL_WIDTH_RATIO,
};
use super::timing::*;

#[inline]
pub(crate) fn pyround(v: f64) -> i64 {
    round_half_even(v)
}

// ─── GIF helpers ───

fn gif_judgement_line_offset() -> i64 {
    pyround(GIF_REFERENCE_JUDGEMENT_X * GIF_ROW_HEIGHT as f64 / GIF_TAIKO_BASE_HEIGHT)
}

fn gif_scroll_length_px() -> i64 {
    pyround(GIF_REFERENCE_SCROLL_LENGTH * GIF_ROW_HEIGHT as f64 / GIF_TAIKO_BASE_HEIGHT)
}

// ─── multiplier / prepared objects ───

#[derive(Debug, Clone, Copy)]
pub(crate) struct MultiplierPoint {
    time: f64,
    multiplier: f64,
}

pub(crate) struct MultiplierLookup {
    pub(crate) points: Vec<MultiplierPoint>,
}

impl MultiplierLookup {
    fn at(&self, time: f64) -> f64 {
        let idx = self.points.partition_point(|p| p.time <= time);
        self.points[idx.saturating_sub(1)].multiplier
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedTaikoHitObject {
    hit_object: TaikoHitObject,
    start_multiplier: f64,
    end_multiplier: f64,
    min_multiplier: f64,
    max_multiplier: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct GifLayout {
    pub(crate) segment_width: i64,
    pub(crate) row_height: i64,
    pub(crate) left_panel_width: i64,
    pub(crate) right_panel_width: i64,
    pub(crate) image_width: i64,
    pub(crate) image_height: i64,
    pub(crate) normal_note_diameter: i64,
    pub(crate) big_note_diameter: i64,
    pub(crate) time_range: f64,
}

// ─── public API ───

pub(crate) fn render_taiko_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    times_ms: Option<Vec<i64>>,
    output_path: &Path,
) -> Result<()> {
    let hit_objects = apply_taiko_object_mods(taiko_hit_objects(beatmap), mods);
    if hit_objects.is_empty() {
        return Err(PreviewError::render("taiko beatmap has no hit objects"));
    }

    let speed_multiplier = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let gameplay_segment_duration = pyround(GIF_DURATION_MS * speed_multiplier);

    let spans: Vec<(i64, i64)> = hit_objects
        .iter()
        .map(|h| (h.start_time, h.end_time))
        .collect();
    let segment_timings: Vec<PreviewSegmentTiming> = PreviewTimeSelector::new(
        beatmap,
        spans,
        GIF_SEGMENT_COUNT,
        gameplay_segment_duration,
        times_ms,
    )?
    .choose()?;

    let slider_multiplier = effective_slider_multiplier(beatmap, mods)?;
    let timing_points = effective_timing_points(beatmap, mods);

    let multiplier_lookup = MultiplierLookup {
        points: build_multiplier_points(&timing_points, slider_multiplier),
    };
    let prepared_hit_objects = prepare_hit_objects(&hit_objects, &multiplier_lookup);
    let time_range = compute_time_range() / speed_multiplier;

    let layout = build_gif_layout(time_range);
    let frame_count = pyround(GIF_DURATION_MS * GIF_FPS / 1000.0).max(1) as usize;
    let frame_duration_ms = pyround(1000.0 / GIF_FPS).max(1) as u32;

    let segment_snapshot_times: Vec<Vec<i64>> = segment_timings
        .iter()
        .map(|timing| {
            (0..frame_count)
                .map(|frame_index| {
                    timing.start_time
                        + pyround(frame_index as f64 * 1000.0 * speed_multiplier / GIF_FPS)
                })
                .collect()
        })
        .collect();

    // Per-thread render cache avoids serialising parallel render calls behind a
    // single Mutex — rayon's chunk-parallel render would otherwise queue on the
    // lock.  Each thread gets its own cache; first few frames rebuild textures,
    // then cache hits dominate.
    thread_local! {
        static TAIKO_GIF_CACHE: RefCell<RenderCache> = RefCell::new(RenderCache::default());
    }

    // Pre-render static row backgrounds (drum panels + tracks + judgement lines)
    // once, then clone per frame instead of redrawing 600 times across 150 frames.
    let static_bg = {
        let mut bg = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            IMAGE_BACKGROUND,
        );
        for segment_index in 0..segment_timings.len() {
            draw_row_background(&mut bg, &layout, segment_index as i64);
        }
        bg
    };

    let render = move |frame_index: usize| -> Img {
        let mut canvas = static_bg.clone();

        for segment_index in 0..segment_timings.len() {
            let snapshot_time = segment_snapshot_times[segment_index][frame_index];
            TAIKO_GIF_CACHE.with(|cache| {
                draw_hit_objects(
                    &mut canvas,
                    &prepared_hit_objects,
                    &layout,
                    segment_index as i64,
                    snapshot_time,
                    &mut *cache.borrow_mut(),
                )
            });
        }

        for (segment_index, segment_timing) in segment_timings.iter().enumerate() {
            draw_time_label(
                &mut canvas,
                segment_timing.start_time,
                gameplay_segment_duration,
                segment_index as i64,
                &layout,
                segment_timing.is_preview,
            );
        }

        canvas
    };

    composer::save_animated_gif_streamed(frame_count, render, output_path, frame_duration_ms)
}

// ─── time range / multiplier ───

pub(crate) fn compute_time_range() -> f64 {
    let in_length = GIF_ASPECT * GIF_STABLE_GAMEFIELD_HEIGHT - GIF_STABLE_HIT_LOCATION;
    in_length / 100.0 * 1000.0 / GIF_VELOCITY_MULTIPLIER
}

pub(crate) fn build_multiplier_points(
    timing_points: &[TimingPoint],
    slider_multiplier: f64,
) -> Vec<MultiplierPoint> {
    let base_beat_length = MULTIPLIER_BASE_BEAT_LENGTH;
    let mut points: Vec<MultiplierPoint> = Vec::new();
    let mut current_beat_length = base_beat_length;
    let mut current_scroll_speed = 1.0f64;

    for tp in timing_points {
        if tp.uninherited {
            if tp.beat_length.is_finite() && tp.beat_length.abs() > 1e-9 {
                current_beat_length = tp.beat_length;
            }
            current_scroll_speed = 1.0;
        } else if tp.beat_length < -0.001 {
            current_scroll_speed = -100.0 / tp.beat_length;
        } else if !tp.beat_length.is_nan() {
            current_scroll_speed = 1.0;
        }

        let multiplier =
            slider_multiplier * current_scroll_speed * base_beat_length / current_beat_length;
        points.push(MultiplierPoint {
            time: tp.time,
            multiplier,
        });
    }

    if points.is_empty() {
        points.push(MultiplierPoint {
            time: 0.0,
            multiplier: slider_multiplier,
        });
    } else if points[0].time > 0.0 {
        let first_multiplier = points[0].multiplier;
        points.insert(
            0,
            MultiplierPoint {
                time: 0.0,
                multiplier: first_multiplier,
            },
        );
    }

    points
}

pub(crate) fn prepare_hit_objects(
    hit_objects: &[TaikoHitObject],
    multiplier_lookup: &MultiplierLookup,
) -> Vec<PreparedTaikoHitObject> {
    hit_objects
        .iter()
        .map(|hit_object| {
            let start_multiplier = multiplier_lookup.at(hit_object.start_time as f64);
            let end_multiplier = multiplier_lookup.at(hit_object.end_time as f64);
            PreparedTaikoHitObject {
                hit_object: *hit_object,
                start_multiplier,
                end_multiplier,
                min_multiplier: start_multiplier.min(end_multiplier),
                max_multiplier: start_multiplier.max(end_multiplier),
            }
        })
        .collect()
}

// ─── layout ───

pub(crate) fn build_gif_layout(time_range: f64) -> GifLayout {
    let segment_width = gif_scroll_length_px();
    let left_panel_width = pyround(GIF_ROW_HEIGHT as f64 * DRUM_PANEL_WIDTH_RATIO);
    let right_panel_width = ROW_INNER_PADDING_X * 2 + segment_width;

    let image_width = PAGE_MARGIN_X * 2 + left_panel_width + right_panel_width;
    let image_height = PAGE_MARGIN_Y * 2
        + GIF_SEGMENT_COUNT as i64 * GIF_ROW_HEIGHT
        + (GIF_SEGMENT_COUNT as i64 - 1) * GIF_ROW_GAP
        + 50;

    let normal_note_diameter = pyround(GIF_ROW_HEIGHT as f64 * NORMAL_NOTE_SIZE_RATIO);
    let big_note_diameter = pyround(normal_note_diameter as f64 * BIG_NOTE_SCALE);

    GifLayout {
        segment_width,
        row_height: GIF_ROW_HEIGHT,
        left_panel_width,
        right_panel_width,
        image_width,
        image_height,
        normal_note_diameter,
        big_note_diameter,
        time_range,
    }
}

fn gif_row_top(row_index: i64, layout: &GifLayout) -> i64 {
    PAGE_MARGIN_Y + row_index * (layout.row_height + GIF_ROW_GAP)
}

fn gif_row_center_y(row_index: i64, layout: &GifLayout) -> i64 {
    gif_row_top(row_index, layout) + layout.row_height / 2
}

fn judgement_line_x(layout: &GifLayout) -> i64 {
    PAGE_MARGIN_X + layout.left_panel_width + gif_judgement_line_offset()
}

// ─── drawing ───

fn draw_judgement_line(image: &mut Img, layout: &GifLayout, row_index: i64) {
    let line_x = judgement_line_x(layout);
    let row_top = gif_row_top(row_index, layout);
    image.set_rect(
        line_x - 1,
        row_top,
        line_x + 1,
        row_top + layout.row_height,
        GIF_JUDGEMENT_LINE_COLOR,
    );
}

/// 绘制单段背景：鼓面板 + 轨道 + 判定线（程序化，无图片）。
pub(crate) fn draw_row_background(image: &mut Img, layout: &GifLayout, row_index: i64) {
    let row_top = gif_row_top(row_index, layout);

    draw_drum_panel(
        image,
        PAGE_MARGIN_X,
        row_top,
        layout.left_panel_width,
        layout.row_height,
    );
    draw_track_background(
        image,
        PAGE_MARGIN_X + layout.left_panel_width,
        row_top,
        layout.right_panel_width,
        layout.row_height,
    );

    draw_judgement_line(image, layout, row_index);
}

pub(crate) fn draw_hit_objects(
    image: &mut Img,
    hit_objects: &[PreparedTaikoHitObject],
    layout: &GifLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
) {
    let left_bound = judgement_line_x(layout);
    let right_bound = PAGE_MARGIN_X + layout.left_panel_width + layout.right_panel_width;

    for hit_object in hit_objects.iter().rev() {
        if can_skip(hit_object, snapshot_time, layout, left_bound, right_bound) {
            continue;
        }
        draw_hit_object(image, hit_object, layout, row_index, snapshot_time, cache);
    }
}

/// Overlapping PositionAt: x = judgement_x + (t - now) / timeRange * multiplier * scrollLength
fn object_x(note_time: f64, snapshot_time: f64, multiplier: f64, layout: &GifLayout) -> i64 {
    let judgement_x = judgement_line_x(layout);
    let offset =
        (note_time - snapshot_time) / layout.time_range * multiplier * layout.segment_width as f64;
    pyround(judgement_x as f64 + offset)
}

fn can_skip(
    hit_object: &PreparedTaikoHitObject,
    snapshot_time: i64,
    layout: &GifLayout,
    left_bound: i64,
    right_bound: i64,
) -> bool {
    let base = &hit_object.hit_object;
    let mut earliest_x = object_x(
        base.start_time as f64,
        snapshot_time as f64,
        hit_object.min_multiplier,
        layout,
    );
    let mut latest_x = object_x(
        base.end_time as f64,
        snapshot_time as f64,
        hit_object.max_multiplier,
        layout,
    );
    if earliest_x > latest_x {
        std::mem::swap(&mut earliest_x, &mut latest_x);
    }
    latest_x < left_bound || earliest_x > right_bound
}

fn draw_hit_object(
    image: &mut Img,
    hit_object: &PreparedTaikoHitObject,
    layout: &GifLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
) {
    let base = &hit_object.hit_object;
    if base.hit_type & SWELL_FLAG != 0 {
        draw_span_object(
            image,
            hit_object,
            layout,
            row_index,
            snapshot_time,
            cache,
            true,
            SWELL_COLOR,
            true,
        );
        return;
    }
    if base.hit_type & DRUMROLL_FLAG != 0 {
        let is_big_roll = base.hitsound & HIT_SOUNDS_STRONG != 0;
        draw_span_object(
            image,
            hit_object,
            layout,
            row_index,
            snapshot_time,
            cache,
            is_big_roll,
            ROLL_COLOR,
            false,
        );
        return;
    }
    draw_circle_object(image, hit_object, layout, row_index, snapshot_time, cache);
}

fn draw_circle_object(
    image: &mut Img,
    hit_object: &PreparedTaikoHitObject,
    layout: &GifLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
) {
    let base = &hit_object.hit_object;
    let center_x = object_x(
        base.start_time as f64,
        snapshot_time as f64,
        hit_object.start_multiplier,
        layout,
    );
    let center_y = gif_row_center_y(row_index, layout);

    let judgement_x = judgement_line_x(layout);
    let right_bound = PAGE_MARGIN_X + layout.left_panel_width + layout.right_panel_width;
    if center_x < judgement_x || center_x > right_bound {
        return;
    }

    let is_strong = base.hitsound & HIT_SOUNDS_STRONG != 0;
    let is_rim = base.hitsound & HIT_SOUNDS_RIM != 0;
    let diameter = if is_strong {
        layout.big_note_diameter
    } else {
        layout.normal_note_diameter
    };
    let color = if is_rim { RIM_NOTE_COLOR } else { CENTRE_NOTE_COLOR };

    draw_note_disc(image, cache, color, diameter, center_x, center_y, false);
}

#[allow(clippy::too_many_arguments)]
fn draw_span_object(
    image: &mut Img,
    hit_object: &PreparedTaikoHitObject,
    layout: &GifLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
    is_swell: bool,
    span_color: [u8; 3],
    draw_swell_marker: bool,
) {
    let base = &hit_object.hit_object;
    let start_x = object_x(
        base.start_time as f64,
        snapshot_time as f64,
        hit_object.start_multiplier,
        layout,
    );
    let end_x = object_x(
        base.end_time as f64,
        snapshot_time as f64,
        hit_object.end_multiplier,
        layout,
    );
    let center_y = gif_row_center_y(row_index, layout);
    let clip_left = judgement_line_x(layout);
    let clip_right = PAGE_MARGIN_X + layout.left_panel_width + layout.right_panel_width;

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

    draw_roll_body(
        image,
        span_color,
        start_x,
        end_x,
        center_y,
        body_height,
        clip_left,
        clip_right,
    );
    draw_span_head(
        image,
        span_color,
        start_x,
        center_y,
        head_diameter,
        cache,
        draw_swell_marker,
        clip_left,
        clip_right,
    );
    draw_span_tail(
        image,
        span_color,
        end_x,
        center_y,
        body_height,
        cache,
        clip_left,
        clip_right,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_roll_body(
    image: &mut Img,
    color: [u8; 3],
    start_x: i64,
    end_x: i64,
    center_y: i64,
    height: i64,
    clip_left: i64,
    clip_right: i64,
) {
    if end_x <= start_x {
        return;
    }
    let visible_left = start_x.max(clip_left);
    let visible_right = end_x.min(clip_right);
    if visible_right <= visible_left {
        return;
    }
    let y0 = pyround(center_y as f64 - height as f64 / 2.0);
    image.fill_rect(
        visible_left,
        y0,
        visible_right - 1,
        y0 + height - 1,
        [color[0], color[1], color[2], 255],
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_span_head(
    image: &mut Img,
    color: [u8; 3],
    center_x: i64,
    center_y: i64,
    diameter: i64,
    cache: &mut RenderCache,
    draw_swell_marker: bool,
    clip_left: i64,
    clip_right: i64,
) {
    let sprite_x = pyround(center_x as f64 - diameter as f64 / 2.0);
    let sprite_y = pyround(center_y as f64 - diameter as f64 / 2.0);
    let disc = cached_note_disc(cache, color, diameter, draw_swell_marker);
    paste_clipped(image, disc, sprite_x, sprite_y, clip_left, clip_right);
}

#[allow(clippy::too_many_arguments)]
fn draw_span_tail(
    image: &mut Img,
    color: [u8; 3],
    join_x: i64,
    center_y: i64,
    height: i64,
    cache: &mut RenderCache,
    clip_left: i64,
    clip_right: i64,
) {
    let y = pyround(center_y as f64 - height as f64 / 2.0);
    let tail = cached_roll_tail(cache, color, height);
    paste_clipped(image, tail, join_x, y, clip_left, clip_right);
}

fn draw_time_label(
    image: &mut Img,
    start_time: i64,
    duration_ms: i64,
    row_index: i64,
    layout: &GifLayout,
    is_preview: bool,
) {
    let y = gif_row_top(row_index, layout) + layout.row_height + 5;
    let label = format!(
        "{} - {}",
        format_gif_time(start_time),
        format_gif_time(start_time + duration_ms)
    );
    let color = if is_preview {
        GIF_PREVIEW_TIME_LABEL_COLOR
    } else {
        GIF_TIME_LABEL_COLOR
    };
    let note_color = if is_preview {
        GIF_PREVIEW_TIME_LABEL_COLOR
    } else {
        GIF_TIME_LABEL_NOTE_COLOR
    };
    let (label_w, label_h) = text_size(&label, GIF_TIME_LABEL_FONT_SIZE);
    let x = (PAGE_MARGIN_X as f64
        + (layout.image_width - PAGE_MARGIN_X * 2 - label_w as i64) as f64 / 2.0)
        .floor() as i64;
    draw_text(image, x, y, &label, GIF_TIME_LABEL_FONT_SIZE, color);

    if is_preview {
        let note = "Preview Time";
        let (note_w, _) = text_size(note, GIF_TIME_LABEL_NOTE_FONT_SIZE);
        let note_x = (PAGE_MARGIN_X as f64
            + (layout.image_width - PAGE_MARGIN_X * 2 - note_w as i64) as f64 / 2.0)
            .floor() as i64;
        draw_text(
            image,
            note_x,
            y + label_h as i64 + 4,
            note,
            GIF_TIME_LABEL_NOTE_FONT_SIZE,
            note_color,
        );
    }
}

fn format_gif_time(ms: i64) -> String {
    let total_seconds = ms.max(0) / 1000;
    format!("{}:{:02}", total_seconds / 60, total_seconds % 60)
}
