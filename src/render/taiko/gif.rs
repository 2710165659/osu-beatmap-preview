//! osu!taiko GIF renderer: multi-segment preview or single-screen clip.

use crate::common::time_selection::{
    GifClipRange, GifRenderOptions, PreviewSegmentTiming, PreviewTimeSelector,
};
use crate::core::errors::{PreviewError, Result};
use crate::core::models::{Beatmap, TaikoHitObject};
use crate::core::mods::ModSettings;
use crate::parser::round_half_even;
use crate::render::canvas::Img;
use crate::render::composer;
use crate::render::text::{draw_text, text_size};
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
    options: GifRenderOptions,
    output_path: &Path,
) -> Result<()> {
    match options {
        GifRenderOptions::Segments {
            times_ms,
            time_axis,
        } => render_taiko_segment_gif(beatmap, mods, times_ms, time_axis, output_path),
        GifRenderOptions::Clip {
            range,
            show_time_label,
        } => render_taiko_clip_gif(beatmap, mods, range, show_time_label, output_path),
    }
}

fn render_taiko_segment_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    times_ms: Option<Vec<i64>>,
    time_axis: crate::common::time_selection::TimeAxis,
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
    let chart_end_time = hit_objects
        .iter()
        .map(|object| object.end_time)
        .max()
        .unwrap();
    let scroll_mapper = build_scroll_mapper(&timing_points, chart_end_time, slider_multiplier, 0.0);
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
    let segment_bpms: Vec<Option<f64>> = segment_timings
        .iter()
        .map(|timing| crate::render::timing::bpm_at(&timing_points, timing.start_time))
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

        for (segment_index, snapshot_times) in segment_snapshot_times
            .iter()
            .enumerate()
            .take(segment_timings.len())
        {
            let snapshot_time = snapshot_times[frame_index];
            TAIKO_GIF_CACHE.with(|cache| {
                draw_hit_objects(
                    &mut canvas,
                    &hit_objects,
                    &scroll_mapper,
                    &layout,
                    segment_index as i64,
                    snapshot_time,
                    &mut cache.borrow_mut(),
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
                time_axis,
                segment_bpms[segment_index],
            );
        }

        canvas
    };

    composer::save_animated_gif_streamed(frame_count, render, output_path, frame_duration_ms)
}

fn render_taiko_clip_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    range: GifClipRange,
    show_time_label: bool,
    output_path: &Path,
) -> Result<()> {
    let hit_objects = apply_taiko_object_mods(taiko_hit_objects(beatmap), mods);
    if hit_objects.is_empty() {
        return Err(PreviewError::render("taiko beatmap has no hit objects"));
    }

    let speed_multiplier = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let slider_multiplier = effective_slider_multiplier(beatmap, mods)?;
    let timing_points = effective_timing_points(beatmap, mods);
    let bpm = crate::render::timing::bpm_at(&timing_points, range.start);
    let chart_end_time = hit_objects
        .iter()
        .map(|object| object.end_time)
        .max()
        .unwrap();
    let scroll_mapper = build_scroll_mapper(&timing_points, chart_end_time, slider_multiplier, 0.0);
    let time_range = compute_time_range() / speed_multiplier;
    let mut layout = build_gif_layout_with_segments(time_range, 1);
    if !show_time_label {
        layout.image_height = PAGE_MARGIN_Y * 2 + GIF_ROW_HEIGHT;
    }
    let frame_count =
        pyround((range.end - range.start) as f64 * GIF_FPS / (1000.0 * speed_multiplier)).max(1)
            as usize;
    let frame_duration_ms = pyround(1000.0 / GIF_FPS).max(1) as u32;

    thread_local! {
        static TAIKO_GIF_CLIP_CACHE: RefCell<RenderCache> = RefCell::new(RenderCache::default());
    }

    let static_bg = {
        let mut bg = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            IMAGE_BACKGROUND,
        );
        draw_row_background(&mut bg, &layout, 0);
        bg
    };

    let render = move |frame_index: usize| -> Img {
        let snapshot_time =
            range.start + pyround(frame_index as f64 * 1000.0 * speed_multiplier / GIF_FPS);
        let mut canvas = static_bg.clone();
        TAIKO_GIF_CLIP_CACHE.with(|cache| {
            draw_hit_objects(
                &mut canvas,
                &hit_objects,
                &scroll_mapper,
                &layout,
                0,
                snapshot_time,
                &mut cache.borrow_mut(),
            )
        });
        if show_time_label {
            draw_time_label_range(
                &mut canvas,
                range.start,
                range.end,
                0,
                &layout,
                range.is_preview,
                range.time_axis,
                bpm,
            );
        }
        canvas
    };

    composer::save_animated_gif_streamed(frame_count, render, output_path, frame_duration_ms)
}

// ─── time range ───

pub(crate) fn compute_time_range() -> f64 {
    let in_length = GIF_ASPECT * GIF_STABLE_GAMEFIELD_HEIGHT - GIF_STABLE_HIT_LOCATION;
    in_length / 100.0 * 1000.0 / GIF_VELOCITY_MULTIPLIER
}

// ─── layout ───

pub(crate) fn build_gif_layout(time_range: f64) -> GifLayout {
    build_gif_layout_with_segments(time_range, GIF_SEGMENT_COUNT)
}

pub(crate) fn build_gif_layout_with_segments(time_range: f64, segment_count: usize) -> GifLayout {
    let segment_width = gif_scroll_length_px();
    let left_panel_width = pyround(GIF_ROW_HEIGHT as f64 * DRUM_PANEL_WIDTH_RATIO);
    let right_panel_width = ROW_INNER_PADDING_X * 2 + segment_width;

    let image_width = PAGE_MARGIN_X * 2 + left_panel_width + right_panel_width;
    let image_height = PAGE_MARGIN_Y * 2
        + segment_count as i64 * GIF_ROW_HEIGHT
        + (segment_count as i64 - 1) * GIF_ROW_GAP
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
    hit_objects: &[TaikoHitObject],
    scroll_mapper: &ScrollPositionMapper,
    layout: &GifLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
) {
    let left_bound = judgement_line_x(layout);
    let right_bound = PAGE_MARGIN_X + layout.left_panel_width + layout.right_panel_width;

    for hit_object in hit_objects.iter().rev() {
        if can_skip(
            hit_object,
            scroll_mapper,
            snapshot_time,
            layout,
            left_bound,
            right_bound,
        ) {
            continue;
        }
        draw_hit_object(
            image,
            hit_object,
            scroll_mapper,
            layout,
            row_index,
            snapshot_time,
            cache,
        );
    }
}

/// Map the integrated scroll distance between `snapshot_time` and `note_time`
/// into the visible lane. This correctly crosses any number of SV changes.
fn object_x(
    note_time: f64,
    snapshot_time: f64,
    scroll_mapper: &ScrollPositionMapper,
    layout: &GifLayout,
) -> i64 {
    let judgement_x = judgement_line_x(layout);
    let base_scroll_per_ms = PIXELS_PER_SCROLL_MULTIPLIER_MS * SCROLL_LENGTH_RATIO;
    let offset = scroll_mapper.distance_between(snapshot_time, note_time)
        / (layout.time_range * base_scroll_per_ms)
        * layout.segment_width as f64;
    pyround(judgement_x as f64 + offset)
}

fn can_skip(
    hit_object: &TaikoHitObject,
    scroll_mapper: &ScrollPositionMapper,
    snapshot_time: i64,
    layout: &GifLayout,
    left_bound: i64,
    right_bound: i64,
) -> bool {
    let mut earliest_x = object_x(
        hit_object.start_time as f64,
        snapshot_time as f64,
        scroll_mapper,
        layout,
    );
    let mut latest_x = object_x(
        hit_object.end_time as f64,
        snapshot_time as f64,
        scroll_mapper,
        layout,
    );
    if earliest_x > latest_x {
        std::mem::swap(&mut earliest_x, &mut latest_x);
    }
    latest_x < left_bound || earliest_x > right_bound
}

fn draw_hit_object(
    image: &mut Img,
    hit_object: &TaikoHitObject,
    scroll_mapper: &ScrollPositionMapper,
    layout: &GifLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
) {
    if hit_object.hit_type & SWELL_FLAG != 0 {
        draw_span_object(
            image,
            hit_object,
            scroll_mapper,
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
    if hit_object.hit_type & DRUMROLL_FLAG != 0 {
        let is_big_roll = hit_object.hitsound & HIT_SOUNDS_STRONG != 0;
        draw_span_object(
            image,
            hit_object,
            scroll_mapper,
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
    draw_circle_object(
        image,
        hit_object,
        scroll_mapper,
        layout,
        row_index,
        snapshot_time,
        cache,
    );
}

fn draw_circle_object(
    image: &mut Img,
    hit_object: &TaikoHitObject,
    scroll_mapper: &ScrollPositionMapper,
    layout: &GifLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
) {
    let center_x = object_x(
        hit_object.start_time as f64,
        snapshot_time as f64,
        scroll_mapper,
        layout,
    );
    let center_y = gif_row_center_y(row_index, layout);

    let judgement_x = judgement_line_x(layout);
    let right_bound = PAGE_MARGIN_X + layout.left_panel_width + layout.right_panel_width;
    if center_x < judgement_x || center_x > right_bound {
        return;
    }

    let is_strong = hit_object.hitsound & HIT_SOUNDS_STRONG != 0;
    let is_rim = hit_object.hitsound & HIT_SOUNDS_RIM != 0;
    let diameter = if is_strong {
        layout.big_note_diameter
    } else {
        layout.normal_note_diameter
    };
    let color = if is_rim {
        RIM_NOTE_COLOR
    } else {
        CENTRE_NOTE_COLOR
    };

    draw_note_disc(image, cache, color, diameter, center_x, center_y, false);
}

#[allow(clippy::too_many_arguments)]
fn draw_span_object(
    image: &mut Img,
    hit_object: &TaikoHitObject,
    scroll_mapper: &ScrollPositionMapper,
    layout: &GifLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
    is_swell: bool,
    span_color: [u8; 3],
    draw_swell_marker: bool,
) {
    let start_x = object_x(
        hit_object.start_time as f64,
        snapshot_time as f64,
        scroll_mapper,
        layout,
    );
    let end_x = object_x(
        hit_object.end_time as f64,
        snapshot_time as f64,
        scroll_mapper,
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
    time_axis: crate::common::time_selection::TimeAxis,
    bpm: Option<f64>,
) {
    let y = gif_row_top(row_index, layout) + layout.row_height + 5;
    let label = format!(
        "{} - {}",
        crate::render::text::format_mmss_floor(time_axis.to_display(start_time)),
        crate::render::text::format_mmss_floor(time_axis.to_display(start_time + duration_ms))
    );
    let color = if is_preview {
        GIF_PREVIEW_TIME_LABEL_COLOR
    } else {
        GIF_TIME_LABEL_COLOR
    };
    let note_color = if bpm.is_some() {
        crate::render::timing::BPM_LABEL_COLOR
    } else if is_preview {
        GIF_PREVIEW_TIME_LABEL_COLOR
    } else {
        GIF_TIME_LABEL_NOTE_COLOR
    };
    let (label_w, label_h) = text_size(&label, GIF_TIME_LABEL_FONT_SIZE);
    let x = (PAGE_MARGIN_X as f64
        + (layout.image_width - PAGE_MARGIN_X * 2 - label_w as i64) as f64 / 2.0)
        .floor() as i64;
    draw_text(image, x, y, &label, GIF_TIME_LABEL_FONT_SIZE, color);

    if is_preview || bpm.is_some() {
        let bpm_label = bpm.map(crate::render::timing::format_bpm);
        let note = bpm_label.unwrap_or_else(|| "Preview Time".to_owned());
        let (note_w, _) = text_size(&note, GIF_TIME_LABEL_NOTE_FONT_SIZE);
        let note_x = (PAGE_MARGIN_X as f64
            + (layout.image_width - PAGE_MARGIN_X * 2 - note_w as i64) as f64 / 2.0)
            .floor() as i64;
        draw_text(
            image,
            note_x,
            y + label_h as i64 + 4,
            &note,
            GIF_TIME_LABEL_NOTE_FONT_SIZE,
            note_color,
        );
    }
}

fn draw_time_label_range(
    image: &mut Img,
    start_time: i64,
    end_time: i64,
    row_index: i64,
    layout: &GifLayout,
    is_preview: bool,
    time_axis: crate::common::time_selection::TimeAxis,
    bpm: Option<f64>,
) {
    let y = gif_row_top(row_index, layout) + layout.row_height + 5;
    let label = format!(
        "{} - {}",
        crate::render::text::format_mmss_floor(time_axis.to_display(start_time)),
        crate::render::text::format_mmss_floor(time_axis.to_display(end_time))
    );
    let color = if is_preview {
        GIF_PREVIEW_TIME_LABEL_COLOR
    } else {
        GIF_TIME_LABEL_COLOR
    };
    let note_color = if bpm.is_some() {
        crate::render::timing::BPM_LABEL_COLOR
    } else if is_preview {
        GIF_PREVIEW_TIME_LABEL_COLOR
    } else {
        GIF_TIME_LABEL_NOTE_COLOR
    };
    let (label_w, label_h) = text_size(&label, GIF_TIME_LABEL_FONT_SIZE);
    let x = (PAGE_MARGIN_X as f64
        + (layout.image_width - PAGE_MARGIN_X * 2 - label_w as i64) as f64 / 2.0)
        .floor() as i64;
    draw_text(image, x, y, &label, GIF_TIME_LABEL_FONT_SIZE, color);

    if is_preview || bpm.is_some() {
        let bpm_label = bpm.map(crate::render::timing::format_bpm);
        let note = bpm_label.unwrap_or_else(|| "Preview Time".to_owned());
        let (note_w, _) = text_size(&note, GIF_TIME_LABEL_NOTE_FONT_SIZE);
        let note_x = (PAGE_MARGIN_X as f64
            + (layout.image_width - PAGE_MARGIN_X * 2 - note_w as i64) as f64 / 2.0)
            .floor() as i64;
        draw_text(
            image,
            note_x,
            y + label_h as i64 + 4,
            &note,
            GIF_TIME_LABEL_NOTE_FONT_SIZE,
            note_color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::TimingPoint;

    #[test]
    fn object_position_integrates_every_sv_segment() {
        let timing_points = [
            TimingPoint {
                time: 0.0,
                beat_length: 500.0,
                meter: 4,
                uninherited: true,
                kiai_mode: false,
            },
            TimingPoint {
                time: 500.0,
                beat_length: -50.0,
                meter: 4,
                uninherited: false,
                kiai_mode: false,
            },
        ];
        let mapper = build_scroll_mapper(&timing_points, 1000, 1.0, 0.0);
        let layout = build_gif_layout_with_segments(compute_time_range(), 1);

        // 0..500ms scrolls at 2x BPM multiplier; 500..1000ms also has 2x SV,
        // so the integrated multiplier is 500*2 + 500*4 = 3000.
        let expected_offset = pyround(3000.0 / layout.time_range * layout.segment_width as f64);
        let actual_offset = object_x(1000.0, 0.0, &mapper, &layout) - judgement_line_x(&layout);

        assert_eq!(actual_offset, expected_offset);
    }
}
