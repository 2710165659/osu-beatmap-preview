//! osu!mania GIF renderer: 4-segment animated falling-note preview.
//! Port of beatmap_preview/mania/gif_renderer.py.

use crate::canvas::{Img, Rgba};
use crate::composer::save_animated_gif_streamed;
use crate::errors::{PreviewError, Result};
use crate::models::{Beatmap, ManiaHitObject, TimingPoint};
use crate::mods::ModSettings;
use crate::parser::round_half_even;
use crate::text::{draw_text, render_text_sprite, text_size};
use crate::common::time_selection::PreviewTimeSelector;
use std::path::Path;

use super::{
    apply_hold_off_mod, apply_inverse_mod, build_sv_changes, darken, format_sv_label,
    is_native_mania, mania_objects, resolve_key_count, GIF_DEFAULT_HIT_POSITION,
    GIF_DURATION_MS, GIF_FPS, GIF_FRAME_HEIGHT, GIF_GRID_GAP, GIF_JUDGEMENT_LINE,
    GIF_MAX_TIME_RANGE, GIF_PREVIEW_TIME_LABEL_COLOR, GIF_SCROLL_SPEED, GIF_SEGMENT_COUNT,
    GIF_SEPARATOR_BACKGROUND, GIF_SEPARATOR_WIDTH, GIF_STAGE_TOP_PADDING, GIF_TIME_LABEL_COLOR,
    GIF_TIME_LABEL_FONT_SIZE, GIF_TIME_LABEL_HEIGHT, GIF_TIME_LABEL_NOTE_COLOR,
    GIF_TIME_LABEL_NOTE_FONT_SIZE, GIF_TIME_LABEL_TOP_GAP, IMAGE_BACKGROUND, LANE_BACKGROUND,
    LANE_WIDTH, LEFT_PANEL_BACKGROUND, LEFT_PANEL_WIDTH, NOTE_HEAD_HEIGHT, NOTE_SIDE_PADDING,
    PAGE_MARGIN_X, PAGE_MARGIN_Y, SV_TEXT_COLOR, SV_TEXT_FONT_SIZE,
};

use super::skin::load_mania_skin_config;

pub(crate) struct GifLayout {
    pub(crate) segment_count: i64,
    pub(crate) segment_width: i64,
    pub(crate) playfield_height: i64,
    pub(crate) lane_area_width: i64,
    pub(crate) image_width: i64,
    pub(crate) image_height: i64,
    pub(crate) hit_position_y: i64,
    pub(crate) scroll_length: i64,
    pub(crate) note_head_height: i64,
    pub(crate) column_left_offsets: Vec<i64>,
    pub(crate) column_widths: Vec<i64>,
    pub(crate) column_colours: Vec<Rgba>,
}

/// Maps beatmap time to a sequential scroll distance, handling BPM and SV changes.
pub(crate) struct ScrollMap {
    starts: Vec<f64>,
    positions: Vec<f64>,
    multipliers: Vec<f64>,
}

impl ScrollMap {
    pub(crate) fn position_at(&self, time: f64) -> f64 {
        let index = self
            .starts
            .partition_point(|s| *s <= time)
            .saturating_sub(1);
        self.positions[index] + (time - self.starts[index]) * self.multipliers[index]
    }
}

pub(crate) fn render_mania_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    times_ms: Option<Vec<i64>>,
    output_path: &Path,
) -> Result<()> {
    let key_count = resolve_key_count(beatmap)?;
    let palette = super::lane_palette(key_count);
    let original_objects = mania_objects(beatmap);
    let mut hit_objects = original_objects.clone();
    if mods.is_some_and(|m| m.inverse) {
        hit_objects = apply_inverse_mod(&hit_objects, &beatmap.timing_points);
    }
    if mods.is_some_and(|m| m.hold_off) {
        hit_objects = apply_hold_off_mod(&hit_objects);
    }
    let cs_mode = mods.is_some_and(|m| m.cs_override);
    if hit_objects.is_empty() {
        return Err(PreviewError::render("mania beatmap has no hit objects"));
    }

    // DT/HT only changes how fast chart time advances; the GIF still plays 10s/segment.
    let speed_multiplier = mods.map_or(1.0, |m| m.speed_multiplier);
    let gameplay_segment_duration = round_half_even(GIF_DURATION_MS as f64 * speed_multiplier);
    let spans: Vec<(i64, i64)> = hit_objects
        .iter()
        .map(|ho| (ho.start_time, ho.end_time))
        .collect();
    let segment_timings = PreviewTimeSelector::new(
        beatmap,
        spans,
        GIF_SEGMENT_COUNT as usize,
        gameplay_segment_duration,
        times_ms,
    )?
    .choose()?;

    let skin_config = load_mania_skin_config(key_count);
    let layout = build_gif_layout(&skin_config);
    let native_mania = is_native_mania(beatmap);
    // CS is Constant Scroll: keep the 33-speed time window but skip SV multipliers.
    let scroll_map = build_scroll_map(beatmap, &original_objects, cs_mode, native_mania);
    // time_range is the chart time span visible from judgement line to top at 33 speed.
    let time_range = compute_time_range(speed_multiplier, skin_config.hit_position);
    let pixels_per_scroll_unit = layout.scroll_length as f64 / time_range;
    let frame_count = round_half_even((GIF_DURATION_MS * GIF_FPS) as f64 / 1000.0).max(1);
    let frame_duration_ms = round_half_even(1000.0 / GIF_FPS as f64).max(1);
    let max_segment_end = segment_timings
        .iter()
        .map(|t| t.start_time + gameplay_segment_duration)
        .max()
        .unwrap_or(0);
    let sv_changes = if cs_mode || !native_mania {
        Vec::new()
    } else {
        build_sv_changes(
            &beatmap.timing_points,
            max_segment_end + round_half_even(time_range),
        )
    };

    let segment_snapshot_times: Vec<Vec<i64>> = segment_timings
        .iter()
        .map(|timing| {
            (0..frame_count)
                .map(|frame_index| {
                    timing.start_time
                        + round_half_even(
                            frame_index as f64 * 1000.0 * speed_multiplier / GIF_FPS as f64,
                        )
                })
                .collect()
        })
        .collect();

    let hold_colors: Vec<Rgba> = palette.iter().map(|&c| darken(c, 0.5)).collect();

    // Precompute each object's scroll-distance position (position_at(start_time))
    // for sorted binary-search culling.  position_at is strictly monotonic in
    // time (every scroll multiplier is > 0) and hit_objects is sorted by
    // start_time, so `pos_start` is ascending.
    //
    // Culling by scroll DISTANCE — not chart time — is required under variable
    // SV: time↔position is non-linear, so during slow SV many chart-seconds
    // fold into a few on-screen pixels.  A time-based window would drop notes
    // that are visibly on screen.  Distance maps to y via the constant
    // `pixels_per_scroll_unit`, so a distance window stays exact under any SV.
    let pos_start: Vec<f64> = hit_objects
        .iter()
        .map(|ho| scroll_map.position_at(ho.start_time as f64))
        .collect();
    let pos_end: Vec<f64> = hit_objects
        .iter()
        .map(|ho| scroll_map.position_at(ho.end_time as f64))
        .collect();
    // Widest LN body in scroll-distance space; subtracted from the lower bound
    // so a hold note whose head is far in the past but whose body is still on
    // screen is never skipped.  Taps contribute 0.
    let max_hold_position: f64 = pos_start
        .iter()
        .zip(&pos_end)
        .map(|(&start, &end)| (end - start).max(0.0))
        .fold(0.0_f64, f64::max);

    // Pre-render static background (segment separators + column/lane backdrops +
    // judgement lines) once and clone per frame, avoiding ~600 redraws of the
    // same pixels across 150 frames.
    let static_bg = {
        let mut bg = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            IMAGE_BACKGROUND,
        );
        draw_segment_separators(&mut bg, &layout);
        for segment_index in 0..layout.segment_count {
            draw_segment_background(&mut bg, segment_left(segment_index, &layout), &layout);
        }
        bg
    };

    // Pre-render each segment's time label (and "Preview Time" note) into a
    // sprite once.  Label text is constant within a segment, so this avoids
    // 150 × format! + text_size + draw_text calls per segment — each frame
    // just alpha_composites the pre-built sprite.
    let label_y = PAGE_MARGIN_Y + layout.playfield_height + GIF_TIME_LABEL_TOP_GAP;
    let pre_labels: Vec<PreLabel> = segment_timings
        .iter()
        .enumerate()
        .map(|(si, st)| {
            let seg_left = segment_left(si as i64, &layout);
            build_pre_label(st, gameplay_segment_duration, &layout, seg_left, label_y)
        })
        .collect();

    // Pre-render SV label sprites: format_sv_label is expensive (String alloc)
    // and the label text never changes, only its y position scrolls per frame.
    let sv_sprites: Vec<(f64, Img)> = sv_changes
        .iter()
        .map(|&(time, sv)| {
            (
                scroll_map.position_at(time as f64),
                render_text_sprite(&format_sv_label(sv), SV_TEXT_FONT_SIZE, SV_TEXT_COLOR),
            )
        })
        .collect();

    let render_frame = |frame_index: usize| -> Img {
        let mut canvas = static_bg.clone();

        for (segment_index, _segment_timing) in segment_timings.iter().enumerate() {
            let seg_left = segment_left(segment_index as i64, &layout);
            let snapshot_time = segment_snapshot_times[segment_index][frame_index];
            let snapshot_pos = scroll_map.position_at(snapshot_time as f64);
            draw_gif_sv_indicators_fast(
                &mut canvas,
                &sv_sprites,
                seg_left,
                snapshot_pos,
                &layout,
                pixels_per_scroll_unit,
            );
            // Binary-search the precomputed scroll-distance positions instead
            // of scanning every object.  Culling by distance (not chart time)
            // stays correct under variable SV; the inner y-cull in
            // draw_gif_hit_object still runs for pixel-exact clipping.
            let (lo_pos, hi_pos) = visible_pos_window(
                snapshot_pos,
                &layout,
                pixels_per_scroll_unit,
                max_hold_position,
            );
            let start_idx = pos_start.partition_point(|&p| p < lo_pos);
            for idx in start_idx..hit_objects.len() {
                if pos_start[idx] > hi_pos {
                    break;
                }
                draw_gif_hit_object(
                    &mut canvas,
                    &hit_objects[idx],
                    &palette,
                    &hold_colors,
                    seg_left,
                    pos_start[idx],
                    pos_end[idx],
                    snapshot_pos,
                    &layout,
                    pixels_per_scroll_unit,
                );
            }
            // Blit pre-rendered time label sprite (no format!/text_size per frame).
            let pl = &pre_labels[segment_index];
            canvas.alpha_composite(&pl.sprite, pl.x, pl.y);
            if let Some(ref note) = pl.note {
                canvas.alpha_composite(&note.sprite, note.x, note.y);
            }
        }
        canvas
    };

    save_animated_gif_streamed(
        frame_count as usize,
        render_frame,
        output_path,
        frame_duration_ms as u32,
    )
}

fn build_gif_layout(skin_config: &super::skin::ManiaSkinConfig) -> GifLayout {
    let column_left_offsets =
        build_column_left_offsets(&skin_config.column_widths, &skin_config.column_line_widths);
    let lane_area_width: i64 = skin_config.column_widths.iter().sum::<i64>()
        + skin_config.column_line_widths.iter().sum::<i64>();
    let segment_width = LEFT_PANEL_WIDTH * 2 + lane_area_width;
    let playfield_height = GIF_FRAME_HEIGHT;
    let hit_position_y = round_half_even(playfield_height as f64 - skin_config.hit_position);
    let scroll_length = (hit_position_y - GIF_STAGE_TOP_PADDING).max(1);
    let average_column_width = skin_config.column_widths.iter().sum::<i64>() as f64
        / skin_config.column_widths.len() as f64;
    // PNG uses a 38px lane with 15px notes; scale the GIF note height with the skin
    // column width so wide columns don't get squashed-looking notes.
    let note_head_height =
        round_half_even(NOTE_HEAD_HEIGHT as f64 * average_column_width / LANE_WIDTH as f64).max(1);
    let image_width = PAGE_MARGIN_X * 2
        + GIF_SEGMENT_COUNT * segment_width
        + (GIF_SEGMENT_COUNT - 1) * GIF_GRID_GAP;
    let image_height =
        PAGE_MARGIN_Y * 2 + playfield_height + GIF_TIME_LABEL_TOP_GAP + GIF_TIME_LABEL_HEIGHT;
    GifLayout {
        segment_count: GIF_SEGMENT_COUNT,
        segment_width,
        playfield_height,
        lane_area_width,
        image_width,
        image_height,
        hit_position_y,
        scroll_length,
        note_head_height,
        column_left_offsets,
        column_widths: skin_config.column_widths.clone(),
        column_colours: skin_config.column_colours.clone(),
    }
}

pub(crate) fn build_column_left_offsets(column_widths: &[i64], column_line_widths: &[i64]) -> Vec<i64> {
    // ColumnLineWidth has keys + 1 entries: leftmost, between-columns, rightmost.
    let mut offsets = Vec::with_capacity(column_widths.len());
    let mut cursor = column_line_widths.first().copied().unwrap_or(0);
    for (index, width) in column_widths.iter().enumerate() {
        offsets.push(cursor);
        cursor += width;
        if index + 1 < column_line_widths.len() {
            cursor += column_line_widths[index + 1];
        }
    }
    offsets
}

/// Mirrors DrawableManiaRuleset.updateTimeRange(): base 33-speed window adjusted by HitPosition.
pub(crate) fn compute_time_range(speed_multiplier: f64, hit_position: f64) -> f64 {
    let hit_position_scale =
        (GIF_FRAME_HEIGHT as f64 - hit_position) / (GIF_FRAME_HEIGHT as f64 - GIF_DEFAULT_HIT_POSITION);
    (GIF_MAX_TIME_RANGE / GIF_SCROLL_SPEED * hit_position_scale * speed_multiplier).max(1.0)
}

pub(crate) fn build_scroll_map(
    beatmap: &Beatmap,
    hit_objects: &[ManiaHitObject],
    constant: bool,
    allow_sv: bool,
) -> ScrollMap {
    if constant {
        return ScrollMap {
            starts: vec![0.0],
            positions: vec![0.0],
            multipliers: vec![1.0],
        };
    }

    let timing_points = &beatmap.timing_points;
    let mut starts: Vec<f64> = Vec::new();
    let mut multipliers: Vec<f64> = Vec::new();
    let base_beat_length = most_common_beat_length(timing_points, hit_objects);
    let mut current_beat_length = base_beat_length;
    let mut current_scroll_speed;

    for point in timing_points {
        if point.uninherited {
            current_beat_length = point.beat_length;
            current_scroll_speed = 1.0;
        } else if allow_sv && point.beat_length < 0.0 {
            // Green-line beat_length is negative; osu! encodes SV as -100 / beat_length.
            current_scroll_speed = -100.0 / point.beat_length;
        } else {
            continue;
        }
        starts.push(point.time);
        multipliers.push(current_scroll_speed * base_beat_length / current_beat_length);
    }

    if starts.is_empty() {
        starts.push(0.0);
        multipliers.push(1.0);
    } else if starts[0] > 0.0 {
        starts.insert(0, 0.0);
        multipliers.insert(0, multipliers[0]);
    }

    let mut positions = vec![0.0];
    for index in 1..starts.len() {
        positions.push(
            positions[index - 1] + (starts[index] - starts[index - 1]) * multipliers[index - 1],
        );
    }
    ScrollMap {
        starts,
        positions,
        multipliers,
    }
}

fn most_common_beat_length(timing_points: &[TimingPoint], hit_objects: &[ManiaHitObject]) -> f64 {
    let red_lines: Vec<&TimingPoint> = timing_points
        .iter()
        .filter(|p| p.uninherited && p.beat_length > 0.0)
        .collect();
    if red_lines.is_empty() {
        return 500.0;
    }

    let last_time = if hit_objects.is_empty() {
        red_lines.last().unwrap().time
    } else {
        hit_objects.iter().map(|ho| ho.end_time).max().unwrap() as f64
    };

    let mut buckets: Vec<(i64, f64)> = Vec::new();
    for (index, point) in red_lines.iter().enumerate() {
        let duration = if point.time > last_time {
            0.0
        } else {
            let current_time = if index == 0 { 0.0 } else { point.time };
            let next_time = if index == red_lines.len() - 1 {
                last_time
            } else {
                red_lines[index + 1].time
            };
            (next_time - current_time).max(0.0)
        };

        let key = round_half_even(point.beat_length * 1000.0);
        match buckets.iter_mut().find(|(k, _)| *k == key) {
            Some((_, total)) => *total += duration,
            None => buckets.push((key, duration)),
        }
    }

    let mut most_common = buckets[0];
    for &bucket in &buckets[1..] {
        if bucket.1 > most_common.1 {
            most_common = bucket;
        }
    }
    let most_common = most_common.0 as f64 / 1000.0;
    let min_beat_length = red_lines.iter().map(|p| p.beat_length).fold(f64::MAX, f64::min);
    let max_beat_length = red_lines.iter().map(|p| p.beat_length).fold(f64::MIN, f64::max);
    most_common.min(max_beat_length).max(min_beat_length)
}

pub(crate) fn segment_left(segment_index: i64, layout: &GifLayout) -> i64 {
    PAGE_MARGIN_X + segment_index * (layout.segment_width + GIF_GRID_GAP)
}

fn draw_segment_separators(canvas: &mut Img, layout: &GifLayout) {
    let playfield_top = PAGE_MARGIN_Y;
    let playfield_bottom = playfield_top + layout.playfield_height;
    for segment_index in 0..layout.segment_count - 1 {
        let left_segment_right = segment_left(segment_index, layout) + layout.segment_width;
        let separator_left = left_segment_right + (GIF_GRID_GAP - GIF_SEPARATOR_WIDTH) / 2;
        canvas.set_rect(
            separator_left,
            playfield_top,
            separator_left + GIF_SEPARATOR_WIDTH,
            playfield_bottom,
            GIF_SEPARATOR_BACKGROUND,
        );
    }
}

pub(crate) fn draw_segment_background(canvas: &mut Img, seg_left: i64, layout: &GifLayout) {
    // GIF skips bar/beat/lane-separator lines; only grey side panels + judgement line.
    let playfield_top = PAGE_MARGIN_Y;
    let playfield_bottom = playfield_top + layout.playfield_height;
    let lane_area_left = seg_left + LEFT_PANEL_WIDTH;
    let lane_area_right = lane_area_left + layout.lane_area_width;

    canvas.set_rect(
        seg_left,
        playfield_top,
        seg_left + layout.segment_width,
        playfield_bottom,
        LANE_BACKGROUND,
    );
    canvas.set_rect(
        seg_left,
        playfield_top,
        lane_area_left,
        playfield_bottom,
        LEFT_PANEL_BACKGROUND,
    );
    canvas.set_rect(
        lane_area_right,
        playfield_top,
        seg_left + layout.segment_width,
        playfield_bottom,
        LEFT_PANEL_BACKGROUND,
    );

    for (lane_index, &lane_width) in layout.column_widths.iter().enumerate() {
        let lane_left = lane_area_left + layout.column_left_offsets[lane_index];
        canvas.set_rect(
            lane_left,
            playfield_top,
            lane_left + lane_width,
            playfield_bottom,
            layout.column_colours[lane_index],
        );
    }

    let judgement_y = playfield_top + layout.hit_position_y;
    canvas.draw_line(
        seg_left as f64,
        judgement_y as f64,
        (seg_left + layout.segment_width) as f64,
        judgement_y as f64,
        2.0,
        GIF_JUDGEMENT_LINE,
    );
}

pub(crate) fn draw_gif_sv_indicators(
    canvas: &mut Img,
    sv_changes: &[(i64, f64)],
    sv_positions: &[f64],
    seg_left: i64,
    snapshot_pos: f64,
    layout: &GifLayout,
    pixels_per_scroll_unit: f64,
) {
    // SV text sits near the left grey panel; only marks the change point, no line.
    debug_assert_eq!(sv_changes.len(), sv_positions.len());
    let (lo_pos, hi_pos) = visible_pos_window(snapshot_pos, layout, pixels_per_scroll_unit, 0.0);
    let start = sv_positions.partition_point(|&pos| pos < lo_pos);
    for index in start..sv_changes.len() {
        let position = sv_positions[index];
        if position > hi_pos {
            break;
        }
        let (_, sv) = sv_changes[index];
        let y = y_at_position(position, snapshot_pos, layout, pixels_per_scroll_unit);
        if y < PAGE_MARGIN_Y || y > PAGE_MARGIN_Y + layout.playfield_height {
            continue;
        }
        let label = format_sv_label(sv);
        let (label_w, label_h) = text_size(&label, SV_TEXT_FONT_SIZE);
        let x = (seg_left + LEFT_PANEL_WIDTH - label_w as i64 - 3).max(0);
        let label_y = (y as f64 - label_h as f64 / 2.0).floor() as i64;
        draw_text(canvas, x, label_y, &label, SV_TEXT_FONT_SIZE, SV_TEXT_COLOR);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_gif_hit_object(
    canvas: &mut Img,
    hit_object: &ManiaHitObject,
    palette: &[Rgba],
    hold_colors: &[Rgba],
    seg_left: i64,
    start_pos: f64,
    end_pos: f64,
    snapshot_pos: f64,
    layout: &GifLayout,
    pixels_per_scroll_unit: f64,
) {
    let y_start = y_at_position(start_pos, snapshot_pos, layout, pixels_per_scroll_unit);
    let y_end = y_at_position(end_pos, snapshot_pos, layout, pixels_per_scroll_unit);
    let playfield_top = PAGE_MARGIN_Y;
    let playfield_bottom = playfield_top + layout.playfield_height;
    if y_start.max(y_end) < playfield_top - layout.note_head_height
        || y_start.min(y_end) > playfield_bottom + layout.note_head_height
    {
        return;
    }

    let lane = (hit_object.lane.max(0) as usize).min(layout.column_widths.len() - 1);
    let lane_color = palette[lane.min(palette.len() - 1)];
    // LN body keeps the PNG look (darkened lane color), not skin.ini ColourHold.
    let hold_color = hold_colors[lane.min(hold_colors.len() - 1)];
    let lane_left =
        seg_left + LEFT_PANEL_WIDTH + layout.column_left_offsets[lane] + NOTE_SIDE_PADDING;
    let lane_right = lane_left + layout.column_widths[lane] - NOTE_SIDE_PADDING * 2;

    if hit_object.is_long_note {
        let body_top = playfield_top.max(y_end.min(y_start - layout.note_head_height));
        let body_bottom = playfield_bottom.min(y_start);
        if body_top < body_bottom {
            canvas.set_rect(lane_left, body_top, lane_right, body_bottom, hold_color);
        }
    }

    let head_top = playfield_top.max(y_start - layout.note_head_height);
    let head_bottom = playfield_bottom.min(y_start);
    if head_top < head_bottom {
        canvas.set_rect(lane_left, head_top, lane_right, head_bottom, lane_color);
    }
}

/// In falling-note mode, future objects sit above the judgement line, past ones below.
#[cfg(test)]
fn y_at_time(
    time: f64,
    snapshot_time: i64,
    layout: &GifLayout,
    scroll_map: &ScrollMap,
    pixels_per_scroll_unit: f64,
) -> i64 {
    let distance = scroll_map.position_at(time) - scroll_map.position_at(snapshot_time as f64);
    PAGE_MARGIN_Y + layout.hit_position_y - round_half_even(distance * pixels_per_scroll_unit)
}

/// Same as `y_at_time` but takes a pre-computed `snapshot_pos` to avoid the
/// `position_at(snapshot_time)` binary search on every call.  For a frame with
/// thousands of objects this eliminates thousands of redundant searches.
#[inline]
fn y_at_position(
    object_pos: f64,
    snapshot_pos: f64,
    layout: &GifLayout,
    pixels_per_scroll_unit: f64,
) -> i64 {
    let distance = object_pos - snapshot_pos;
    PAGE_MARGIN_Y + layout.hit_position_y - round_half_even(distance * pixels_per_scroll_unit)
}

/// Scroll-distance window `[lo, hi]` outside which no hit object can be
/// visible, used to binary-search the precomputed `pos_start` array instead
/// of scanning every object per frame.
///
/// The window is in scroll-DISTANCE units (position_at), not chart time,
/// because variable SV makes time↔position non-linear: during slow SV many
/// chart-seconds fold into a few on-screen pixels, so a time-based window
/// would drop notes that are visibly on screen.  Distance maps to screen y
/// via the constant `pixels_per_scroll_unit`, so this window stays exact
/// under any SV.  `lo`/`hi` span the playfield plus a `note_head_height`
/// margin so heads/bodies don't pop at the edges.
///
/// `max_hold_position` (widest LN body in distance space) is subtracted from
/// the lower bound so long-note bodies are never clipped: a LN whose
/// `end_time` is still on screen may have `start_time` far earlier in
/// distance.  Since `pos_start >= pos_end - max_hold_position` and
/// `pos_end >= snapshot_pos - past_dist`, `pos_start >= lo` holds and
/// partition_point keeps the note.  The inner y-cull in `draw_gif_hit_object`
/// still runs for pixel-exact clipping.
#[inline]
pub(crate) fn visible_pos_window(
    snapshot_pos: f64,
    layout: &GifLayout,
    pixels_per_scroll_unit: f64,
    max_hold_position: f64,
) -> (f64, f64) {
    // Furthest visible future note head sits at y = playfield_top - note_head_height,
    // i.e. distance = (hit_position_y + note_head_height) above the judgement line.
    let future_dist =
        (layout.hit_position_y + layout.note_head_height) as f64 / pixels_per_scroll_unit;
    // Furthest visible past note sits at y = playfield_bottom + note_head_height,
    // i.e. distance = -(playfield_height - hit_position_y + note_head_height).
    let past_dist = (layout.playfield_height - layout.hit_position_y + layout.note_head_height)
        as f64
        / pixels_per_scroll_unit;
    (
        snapshot_pos - past_dist - max_hold_position,
        snapshot_pos + future_dist,
    )
}

/// Pre-rendered time label sprite + blit position, built once per segment.
struct PreLabel {
    sprite: Img,
    x: i64,
    y: i64,
    note: Option<Box<PreLabel>>,
}

fn build_pre_label(
    timing: &crate::common::time_selection::PreviewSegmentTiming,
    duration_ms: i64,
    layout: &GifLayout,
    seg_left: i64,
    y: i64,
) -> PreLabel {
    let label = format!(
        "{} - {}",
        format_gif_time(timing.start_time),
        format_gif_time(timing.start_time + duration_ms)
    );
    let color = if timing.is_preview {
        GIF_PREVIEW_TIME_LABEL_COLOR
    } else {
        GIF_TIME_LABEL_COLOR
    };
    let note_color = if timing.is_preview {
        GIF_PREVIEW_TIME_LABEL_COLOR
    } else {
        GIF_TIME_LABEL_NOTE_COLOR
    };
    let (label_w, label_h) = text_size(&label, GIF_TIME_LABEL_FONT_SIZE);
    let sprite = render_text_sprite(&label, GIF_TIME_LABEL_FONT_SIZE, color);
    let x = seg_left + (layout.segment_width - label_w as i64).div_euclid(2);

    let note = if timing.is_preview {
        let note_text = "Preview Time";
        let (note_w, _) = text_size(note_text, GIF_TIME_LABEL_NOTE_FONT_SIZE);
        let note_sprite = render_text_sprite(note_text, GIF_TIME_LABEL_NOTE_FONT_SIZE, note_color);
        let note_x = seg_left + (layout.segment_width - note_w as i64).div_euclid(2);
        Some(Box::new(PreLabel {
            sprite: note_sprite,
            x: note_x,
            y: y + label_h as i64 + 4,
            note: None,
        }))
    } else {
        None
    };

    PreLabel { sprite, x, y, note }
}

/// SV indicator drawing using pre-rendered label sprites.
/// `sv_sprites` is `(scroll_position, sprite)` pairs built once; only the y
/// position is computed per frame.
fn draw_gif_sv_indicators_fast(
    canvas: &mut Img,
    sv_sprites: &[(f64, Img)],
    seg_left: i64,
    snapshot_pos: f64,
    layout: &GifLayout,
    pixels_per_scroll_unit: f64,
) {
    // SV positions are monotonic because timing points are sorted and every
    // valid SV multiplier is positive. Include both boundaries to preserve
    // the old pixel-space visibility behavior at the playfield edges.
    let (lo_pos, hi_pos) = visible_pos_window(snapshot_pos, layout, pixels_per_scroll_unit, 0.0);
    let start = sv_sprites.partition_point(|(pos, _)| *pos < lo_pos);
    for &(position, ref sprite) in &sv_sprites[start..] {
        if position > hi_pos {
            break;
        }
        let y = y_at_position(position, snapshot_pos, layout, pixels_per_scroll_unit);
        if y < PAGE_MARGIN_Y || y > PAGE_MARGIN_Y + layout.playfield_height {
            continue;
        }
        let label_h = sprite.h as i64;
        let x = (seg_left + LEFT_PANEL_WIDTH - sprite.w as i64 - 3).max(0);
        let label_y = (y as f64 - label_h as f64 / 2.0).floor() as i64;
        canvas.alpha_composite(sprite, x, label_y);
    }
}

fn format_gif_time(ms: i64) -> String {
    let total_seconds = ms.max(0) / 1000;
    format!("{}:{:02}", total_seconds / 60, total_seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_layout() -> GifLayout {
        GifLayout {
            segment_count: 1,
            segment_width: 100,
            playfield_height: 768,
            lane_area_width: 80,
            image_width: 140,
            image_height: 808,
            hit_position_y: 640,
            scroll_length: 624,
            note_head_height: 15,
            column_left_offsets: vec![0],
            column_widths: vec![80],
            column_colours: vec![[0, 0, 0, 255]],
        }
    }

    fn variable_sv_map() -> ScrollMap {
        ScrollMap {
            starts: vec![0.0, 1_000.0, 2_000.0, 4_000.0],
            positions: vec![0.0, 1_000.0, 1_250.0, 5_250.0],
            multipliers: vec![1.0, 0.25, 2.0, 0.5],
        }
    }

    #[test]
    fn precomputed_positions_match_time_based_y_across_sv_changes() {
        let layout = test_layout();
        let scroll_map = variable_sv_map();
        let pixels_per_scroll_unit = 0.8;

        for snapshot in [-500, 0, 999, 1_000, 1_500, 2_000, 3_999, 4_000, 6_000] {
            let snapshot_pos = scroll_map.position_at(snapshot as f64);
            for time in [-250, 0, 500, 1_000, 1_750, 2_000, 3_000, 4_000, 5_000] {
                let old_y = y_at_time(
                    time as f64,
                    snapshot,
                    &layout,
                    &scroll_map,
                    pixels_per_scroll_unit,
                );
                let new_y = y_at_position(
                    scroll_map.position_at(time as f64),
                    snapshot_pos,
                    &layout,
                    pixels_per_scroll_unit,
                );
                assert_eq!(old_y, new_y, "snapshot={snapshot}, time={time}");
            }
        }
    }

    #[test]
    fn sv_position_window_matches_full_pixel_visibility_scan() {
        let layout = test_layout();
        let scroll_map = variable_sv_map();
        let pixels_per_scroll_unit = 0.8;
        let sv_times = [0, 500, 1_000, 1_500, 2_000, 2_000, 3_000, 4_000, 5_000];
        let sv_positions: Vec<f64> = sv_times
            .iter()
            .map(|&time| scroll_map.position_at(time as f64))
            .collect();

        for snapshot in [-500, 0, 750, 1_000, 1_750, 2_000, 3_500, 4_000, 6_000] {
            let snapshot_pos = scroll_map.position_at(snapshot as f64);
            let expected: Vec<usize> = sv_times
                .iter()
                .enumerate()
                .filter_map(|(index, &time)| {
                    let y = y_at_time(
                        time as f64,
                        snapshot,
                        &layout,
                        &scroll_map,
                        pixels_per_scroll_unit,
                    );
                    (y >= PAGE_MARGIN_Y && y <= PAGE_MARGIN_Y + layout.playfield_height)
                        .then_some(index)
                })
                .collect();

            let (lo_pos, hi_pos) =
                visible_pos_window(snapshot_pos, &layout, pixels_per_scroll_unit, 0.0);
            let start = sv_positions.partition_point(|&pos| pos < lo_pos);
            let actual: Vec<usize> = (start..sv_positions.len())
                .take_while(|&index| sv_positions[index] <= hi_pos)
                .filter(|&index| {
                    let y = y_at_position(
                        sv_positions[index],
                        snapshot_pos,
                        &layout,
                        pixels_per_scroll_unit,
                    );
                    y >= PAGE_MARGIN_Y && y <= PAGE_MARGIN_Y + layout.playfield_height
                })
                .collect();

            assert_eq!(expected, actual, "snapshot={snapshot}");
        }
    }
}
