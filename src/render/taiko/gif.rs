//! osu!taiko GIF 渲染器：多段预览或单画面片段。

use crate::common::time_selection::{GifRenderOptions, PreviewSegmentTiming, PreviewTimeSelector};
use crate::core::errors::{PreviewError, Result};
use crate::core::models::{Beatmap, TaikoHitObject, TimingPoint};
use crate::core::mods::ModSettings;
use crate::core::timeout::RequestDeadline;
use crate::parser::round_half_even;
use crate::render::canvas::Img;
use crate::render::composer;
use crate::render::text::{draw_text, text_size};
use std::cell::RefCell;
use std::path::Path;

use super::animation::{
    drum_roll_tick_transform, generate_drum_roll_ticks, generate_measure_lines, measure_line_alpha,
};
use super::constants::*;
use super::notes::{
    cached_drum_roll_tick, cached_note_disc, cached_roll_tail, draw_drum_panel, draw_note_disc,
    draw_track_background, paste_clipped, RenderCache,
};
use super::timing::*;

#[inline]
pub(crate) fn pyround(v: f64) -> i64 {
    round_half_even(v)
}

// ─── GIF 辅助函数 ───

fn gif_judgement_line_offset(row_height: i64) -> i64 {
    pyround(
        crate::render::taiko::constants::REFERENCE_JUDGEMENT_X * row_height as f64
            / crate::render::taiko::constants::TAIKO_BASE_HEIGHT,
    )
}

// ─── 倍率与预处理对象 ───

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
    drum_roll_ticks: Vec<PreparedAnimationPoint>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PreparedAnimationPoint {
    time: f64,
    multiplier: f64,
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
    playfield_left: i64,
    first_row_top: i64,
    row_stride: i64,
}

// ─── 公共 API ───

pub(crate) fn render_taiko_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    options: GifRenderOptions,
    output_path: &Path,
    deadline: &RequestDeadline,
) -> Result<()> {
    deadline.check()?;
    match options {
        GifRenderOptions::Segments {
            times_ms,
            duration_seconds,
            time_axis,
        } => render_taiko_segment_gif(
            beatmap,
            mods,
            times_ms,
            duration_seconds,
            time_axis,
            output_path,
            deadline,
        ),
    }
}

fn render_taiko_segment_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    times_ms: Option<Vec<i64>>,
    duration_seconds: Option<f64>,
    time_axis: crate::common::time_selection::TimeAxis,
    output_path: &Path,
    deadline: &RequestDeadline,
) -> Result<()> {
    let hit_objects = apply_taiko_object_mods(taiko_hit_objects(beatmap), mods);
    if hit_objects.is_empty() {
        return Err(PreviewError::render("taiko beatmap has no hit objects"));
    }

    let speed_multiplier = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let segment_duration_ms = duration_seconds
        .map(|seconds| seconds * 1000.0)
        .unwrap_or(crate::config::current().layout.taiko.gif.DURATION_MS);
    let gameplay_segment_duration = pyround(segment_duration_ms * speed_multiplier);

    let spans: Vec<(i64, i64)> = hit_objects
        .iter()
        .map(|h| (h.start_time, h.end_time))
        .collect();
    let segment_timings: Vec<PreviewSegmentTiming> = PreviewTimeSelector::new(
        beatmap,
        spans,
        crate::config::current().layout.taiko.gif.ROW_COUNT as usize,
        gameplay_segment_duration,
        times_ms,
    )?
    .choose()?;

    let slider_multiplier = effective_slider_multiplier(beatmap, mods)?;
    let timing_points = effective_timing_points(beatmap, mods);

    let multiplier_lookup = MultiplierLookup {
        points: build_multiplier_points(&timing_points, slider_multiplier),
    };
    let slider_tick_rate = beatmap.difficulty.get_f64_or("SliderTickRate", 1.0);
    let prepared_hit_objects = prepare_hit_objects(
        &hit_objects,
        &multiplier_lookup,
        &timing_points,
        slider_tick_rate,
    );
    let prepared_measure_lines = prepare_measure_lines(
        &hit_objects,
        &timing_points,
        &multiplier_lookup,
        crate::config::current().layout.taiko.gif.SHOW_MEASURE_LINES,
    );
    let time_range = compute_time_range() / speed_multiplier;

    let layout = build_gif_layout(time_range);
    let frame_count = pyround(
        segment_duration_ms * crate::config::current().layout.taiko.gif.FPS / 1000.0,
    )
    .max(1) as usize;
    let frame_duration_ms =
        pyround(1000.0 / crate::config::current().layout.taiko.gif.FPS).max(1) as u32;

    let segment_snapshot_times: Vec<Vec<i64>> = segment_timings
        .iter()
        .map(|timing| {
            (0..frame_count)
                .map(|frame_index| {
                    timing.start_time
                        + pyround(
                            frame_index as f64 * 1000.0 * speed_multiplier
                                / crate::config::current().layout.taiko.gif.FPS,
                        )
                })
                .collect()
        })
        .collect();

    // 每线程独立渲染缓存，避免并行渲染调用在同一 Mutex 后串行化。
    // 否则 rayon 分块渲染会在锁上排队。每个线程拥有自己的缓存；前几帧构建纹理，
    // 之后主要命中缓存。
    thread_local! {
        static TAIKO_GIF_CACHE: RefCell<RenderCache> = RefCell::new(RenderCache::default());
    }

    // 预渲染静态行背景（鼓面板、轨道和判定线）一次，再逐帧克隆，
    // 避免在 150 帧中重复绘制 600 次。
    let static_bg = {
        let mut bg = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            crate::config::current().layout.taiko.gif.IMAGE_BACKGROUND,
        );
        for segment_index in 0..segment_timings.len() {
            draw_row_background(&mut bg, &layout, segment_index as i64);
        }
        bg
    };

    let render = move |frame_index: usize| -> Img {
        let mut canvas = static_bg.clone();

        debug_assert_eq!(segment_timings.len(), segment_snapshot_times.len());
        for (segment_index, snapshot_times) in segment_snapshot_times
            .iter()
            .enumerate()
            .take(segment_timings.len())
        {
            let snapshot_time = snapshot_times[frame_index];
            TAIKO_GIF_CACHE.with(|cache| {
                draw_hit_objects(
                    &mut canvas,
                    &prepared_hit_objects,
                    &prepared_measure_lines,
                    &layout,
                    segment_index as i64,
                    snapshot_time,
                    &mut cache.borrow_mut(),
                )
            });
        }

        for (segment_index, segment_timing) in segment_timings.iter().enumerate() {
            if crate::config::current().layout.taiko.gif.SHOW_TIME_LABEL {
                draw_time_label(
                    &mut canvas,
                    segment_timing.start_time,
                    gameplay_segment_duration,
                    segment_index as i64,
                    &layout,
                    segment_timing.is_preview,
                    time_axis,
                );
            }
        }

        canvas
    };

    composer::save_animated_gif_streamed(
        frame_count,
        render,
        output_path,
        frame_duration_ms,
        deadline,
    )
}

// ─── 时间范围与倍率 ───

pub(crate) fn compute_time_range() -> f64 {
    let in_length = crate::render::taiko::constants::ASPECT_RATIO
        * crate::render::taiko::constants::STABLE_GAMEFIELD_HEIGHT
        - crate::render::taiko::constants::STABLE_HIT_LOCATION;
    in_length / 100.0 * 1000.0 / crate::render::taiko::constants::VELOCITY_MULTIPLIER
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
    timing_points: &[TimingPoint],
    slider_tick_rate: f64,
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
                drum_roll_ticks: generate_drum_roll_ticks(
                    hit_object,
                    timing_points,
                    slider_tick_rate,
                )
                .into_iter()
                .map(|tick| PreparedAnimationPoint {
                    time: tick.time,
                    multiplier: multiplier_lookup.at(tick.time),
                })
                .collect(),
            }
        })
        .collect()
}

pub(crate) fn prepare_measure_lines(
    hit_objects: &[TaikoHitObject],
    timing_points: &[TimingPoint],
    multiplier_lookup: &MultiplierLookup,
    enabled: bool,
) -> Vec<PreparedAnimationPoint> {
    if !enabled {
        return Vec::new();
    }
    let Some(first_hit_time) = hit_objects.iter().map(|object| object.start_time).min() else {
        return Vec::new();
    };
    let last_hit_time = hit_objects
        .iter()
        .map(|object| object.end_time)
        .max()
        .unwrap_or(first_hit_time);

    generate_measure_lines(timing_points, first_hit_time, last_hit_time)
        .into_iter()
        .map(|line| PreparedAnimationPoint {
            time: line.time,
            multiplier: multiplier_lookup.at(line.time),
        })
        .collect()
}

// ─── 布局 ───

pub(crate) fn build_gif_layout(time_range: f64) -> GifLayout {
    build_gif_layout_with_segments_and_format(
        time_range,
        crate::config::current().layout.taiko.gif.ROW_COUNT as usize,
        crate::render::geometry::OutputFormat::Gif,
    )
}

pub(crate) fn build_gif_layout_with_segments_and_format(
    time_range: f64,
    segment_count: usize,
    output_format: crate::render::geometry::OutputFormat,
) -> GifLayout {
    let geometry = crate::render::geometry::taiko_geometry(output_format);
    let render_scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Taiko,
        output_format,
    );
    let row_height = geometry.playfield.height;
    let left_panel_width =
        pyround(row_height as f64 * crate::render::taiko::constants::DRUM_PANEL_WIDTH_RATIO);
    let right_panel_width = geometry.playfield.width - left_panel_width;
    let inner_padding = crate::render::geometry::scale_px(
        crate::render::taiko::constants::ROW_INNER_PADDING_X as f64,
        render_scale,
    );
    let segment_width = (right_panel_width - inner_padding * 2).max(1);

    let (image_width, image_height, playfield_left, first_row_top, row_stride) =
        if output_format == crate::render::geometry::OutputFormat::Gif {
            let config = &crate::config::current().layout.taiko.gif;
            let unit_width =
                config.INFO_MARGIN_LEFT + geometry.content.width + config.INFO_MARGIN_RIGHT;
            // 时间标签关闭时，底部信息区不参与行高度和行间步进。
            let info_bottom = if config.SHOW_TIME_LABEL {
                config.INFO_MARGIN_BOTTOM
            } else {
                0
            };
            let unit_height = config.INFO_MARGIN_TOP + row_height + info_bottom;
            (
                config.PAGE_MARGIN_LEFT + unit_width + config.PAGE_MARGIN_RIGHT,
                config.PAGE_MARGIN_TOP
                    + segment_count as i64 * unit_height
                    + (segment_count as i64 - 1) * config.ROW_GAP
                    + config.PAGE_MARGIN_BOTTOM,
                config.PAGE_MARGIN_LEFT + config.INFO_MARGIN_LEFT,
                config.PAGE_MARGIN_TOP + config.INFO_MARGIN_TOP,
                unit_height + config.ROW_GAP,
            )
        } else {
            (geometry.content.width, row_height, 0, 0, row_height)
        };

    let normal_note_diameter =
        pyround(row_height as f64 * crate::render::taiko::constants::NORMAL_NOTE_SIZE_RATIO);
    let big_note_diameter =
        pyround(normal_note_diameter as f64 * crate::render::taiko::constants::BIG_NOTE_SCALE);

    GifLayout {
        segment_width,
        row_height,
        left_panel_width,
        right_panel_width,
        image_width,
        image_height,
        normal_note_diameter,
        big_note_diameter,
        time_range,
        playfield_left,
        first_row_top,
        row_stride,
    }
}

fn gif_row_top(row_index: i64, layout: &GifLayout) -> i64 {
    layout.first_row_top + row_index * layout.row_stride
}

fn gif_row_center_y(row_index: i64, layout: &GifLayout) -> i64 {
    gif_row_top(row_index, layout) + layout.row_height / 2
}

fn judgement_line_x(layout: &GifLayout) -> i64 {
    layout.playfield_left + layout.left_panel_width + gif_judgement_line_offset(layout.row_height)
}

// ─── 绘制 ───

fn draw_judgement_line(image: &mut Img, layout: &GifLayout, row_index: i64) {
    let line_x = judgement_line_x(layout);
    let row_top = gif_row_top(row_index, layout);
    image.set_rect(
        line_x - 1,
        row_top,
        line_x + 1,
        row_top + layout.row_height,
        crate::config::current()
            .layout
            .taiko
            .gif
            .JUDGEMENT_LINE_COLOR,
    );
}

/// 绘制单段背景：鼓面板 + 轨道 + 判定线（程序化，无图片）。
pub(crate) fn draw_row_background(image: &mut Img, layout: &GifLayout, row_index: i64) {
    let row_top = gif_row_top(row_index, layout);

    draw_drum_panel(
        image,
        layout.playfield_left,
        row_top,
        layout.left_panel_width,
        layout.row_height,
    );
    draw_track_background(
        image,
        layout.playfield_left + layout.left_panel_width,
        row_top,
        layout.right_panel_width,
        layout.row_height,
    );

    draw_judgement_line(image, layout, row_index);
}

pub(crate) fn draw_hit_objects(
    image: &mut Img,
    hit_objects: &[PreparedTaikoHitObject],
    measure_lines: &[PreparedAnimationPoint],
    layout: &GifLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
) {
    let left_bound = judgement_line_x(layout);
    let right_bound = layout.playfield_left + layout.left_panel_width + layout.right_panel_width;

    draw_measure_lines(
        image,
        measure_lines,
        layout,
        row_index,
        snapshot_time,
        left_bound,
        right_bound,
    );

    for hit_object in hit_objects.iter().rev() {
        if can_skip(hit_object, snapshot_time, layout, left_bound, right_bound) {
            continue;
        }
        draw_hit_object(image, hit_object, layout, row_index, snapshot_time, cache);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_measure_lines(
    image: &mut Img,
    measure_lines: &[PreparedAnimationPoint],
    layout: &GifLayout,
    row_index: i64,
    snapshot_time: i64,
    left_bound: i64,
    right_bound: i64,
) {
    let height = pyround(layout.row_height as f64 * MEASURE_LINE_HEIGHT_RATIO).max(1);
    let center_y = gif_row_center_y(row_index, layout);
    let top = center_y - height / 2;
    let base_color = ANIMATION_MEASURE_LINE_COLOR;

    for line in measure_lines {
        let alpha = measure_line_alpha(line.time, snapshot_time);
        if alpha <= 0.0 {
            continue;
        }
        let center_x = object_x(line.time, snapshot_time as f64, line.multiplier, layout);
        let left = center_x - MEASURE_LINE_WIDTH / 2;
        let right = left + MEASURE_LINE_WIDTH - 1;
        if right < left_bound || left > right_bound {
            continue;
        }
        let mut color = base_color;
        color[3] = pyround(color[3] as f64 * alpha).clamp(0, 255) as u8;
        image.fill_rect(
            left.max(left_bound),
            top,
            right.min(right_bound),
            top + height - 1,
            color,
        );
    }
}

/// Overlapping PositionAt：x = judgement_x + (t - now) / timeRange * multiplier * scrollLength。
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
            crate::render::taiko::constants::SWELL_COLOR,
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
            crate::render::taiko::constants::ROLL_COLOR,
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
    let right_bound = layout.playfield_left + layout.left_panel_width + layout.right_panel_width;
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
    let color = if is_rim {
        crate::render::taiko::constants::RIM_NOTE_COLOR
    } else {
        crate::render::taiko::constants::CENTRE_NOTE_COLOR
    };

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
    use_large_geometry: bool,
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
    let clip_right = layout.playfield_left + layout.left_panel_width + layout.right_panel_width;

    let head_diameter = if use_large_geometry {
        layout.big_note_diameter
    } else {
        layout.normal_note_diameter
    };
    let body_ratio = if use_large_geometry {
        crate::render::taiko::constants::SWELL_BODY_HEIGHT_RATIO
    } else {
        crate::render::taiko::constants::SPAN_BODY_HEIGHT_RATIO
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
    if base.hit_type & DRUMROLL_FLAG != 0 {
        // 尾端 tick 必须位于尾部半圆上方，否则交界处会被覆盖一半；
        // 头部最后绘制，仍保持 osu! 中头部遮挡首个 tick 的层级。
        draw_drum_roll_ticks(
            image,
            &hit_object.drum_roll_ticks,
            layout,
            snapshot_time,
            center_y,
            cache,
            clip_left,
            clip_right,
        );
    }
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
}

#[allow(clippy::too_many_arguments)]
fn draw_drum_roll_ticks(
    image: &mut Img,
    ticks: &[PreparedAnimationPoint],
    layout: &GifLayout,
    snapshot_time: i64,
    center_y: i64,
    cache: &mut RenderCache,
    clip_left: i64,
    clip_right: i64,
) {
    let base_diameter =
        pyround(layout.normal_note_diameter as f64 * DRUM_ROLL_TICK_DIAMETER_RATIO).max(1);

    for tick in ticks {
        let (alpha, scale) = drum_roll_tick_transform(tick.time, snapshot_time);
        if alpha <= 0.0 {
            continue;
        }
        let diameter = pyround(base_diameter as f64 * scale).max(1);
        let center_x = object_x(tick.time, snapshot_time as f64, tick.multiplier, layout);
        // 奇数尺寸按整数半径定位，避免银行家舍入让极小菱形偏离逻辑中心 1px。
        let x = center_x - diameter / 2;
        if x + diameter < clip_left || x > clip_right {
            continue;
        }
        let y = center_y - diameter / 2;
        let sprite = cached_drum_roll_tick(cache, diameter, alpha);
        paste_clipped(image, sprite, x, y, clip_left, clip_right);
    }
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
) {
    let text_gap = crate::render::geometry::scale_px(
        5.0,
        crate::render::geometry::output_scale(
            crate::render::geometry::GameMode::Taiko,
            crate::render::geometry::OutputFormat::Gif,
        ),
    );
    let y = gif_row_top(row_index, layout) + layout.row_height + text_gap;
    let label = format!(
        "{} - {}",
        crate::render::text::format_mmss_floor(time_axis.to_display(start_time)),
        crate::render::text::format_mmss_floor(time_axis.to_display(start_time + duration_ms))
    );
    let color = if is_preview {
        crate::config::current()
            .layout
            .taiko
            .gif
            .PREVIEW_TIME_LABEL_COLOR
    } else {
        crate::config::current().layout.taiko.gif.TIME_LABEL_COLOR
    };
    let note_color = if is_preview {
        crate::config::current()
            .layout
            .taiko
            .gif
            .PREVIEW_TIME_LABEL_COLOR
    } else {
        crate::config::current()
            .layout
            .taiko
            .gif
            .TIME_LABEL_NOTE_COLOR
    };
    let (label_w, label_h) = text_size(
        &label,
        crate::config::current()
            .layout
            .taiko
            .gif
            .TIME_LABEL_FONT_SIZE,
    );
    let x = (layout.playfield_left as f64
        + (layout.image_width - layout.playfield_left * 2 - label_w as i64) as f64 / 2.0)
        .floor() as i64;
    draw_text(
        image,
        x,
        y,
        &label,
        crate::config::current()
            .layout
            .taiko
            .gif
            .TIME_LABEL_FONT_SIZE,
        color,
    );

    if is_preview {
        let note = "Preview Time";
        let (note_w, _) = text_size(
            note,
            crate::config::current()
                .layout
                .taiko
                .gif
                .TIME_LABEL_NOTE_FONT_SIZE,
        );
        let note_x = (layout.playfield_left as f64
            + (layout.image_width - layout.playfield_left * 2 - note_w as i64) as f64 / 2.0)
            .floor() as i64;
        draw_text(
            image,
            note_x,
            y + label_h as i64
                + crate::render::geometry::scale_px(
                    4.0,
                    crate::render::geometry::output_scale(
                        crate::render::geometry::GameMode::Taiko,
                        crate::render::geometry::OutputFormat::Gif,
                    ),
                ),
            note,
            crate::config::current()
                .layout
                .taiko
                .gif
                .TIME_LABEL_NOTE_FONT_SIZE,
            note_color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drum_roll_end_tick_is_drawn_above_tail() {
        let layout = GifLayout {
            segment_width: 100,
            row_height: 80,
            left_panel_width: 0,
            right_panel_width: 200,
            image_width: 216,
            image_height: 96,
            normal_note_diameter: 38,
            big_note_diameter: 58,
            time_range: 1000.0,
            playfield_left: 0,
            first_row_top: 0,
            row_stride: 80,
        };
        let hit_object = PreparedTaikoHitObject {
            hit_object: TaikoHitObject {
                start_time: 0,
                end_time: 1000,
                hit_type: DRUMROLL_FLAG,
                hitsound: 0,
            },
            start_multiplier: 1.0,
            end_multiplier: 1.0,
            min_multiplier: 1.0,
            max_multiplier: 1.0,
            drum_roll_ticks: vec![PreparedAnimationPoint {
                time: 1000.0,
                multiplier: 1.0,
            }],
        };
        let mut image = Img::new(216, 96, [0, 0, 0, 255]);
        let mut cache = RenderCache::default();

        draw_span_object(
            &mut image,
            &hit_object,
            &layout,
            0,
            0,
            &mut cache,
            false,
            ROLL_COLOR,
            false,
        );

        let end_x = object_x(1000.0, 0.0, 1.0, &layout);
        let center_y = gif_row_center_y(0, &layout);
        let right_half_contains_tick = (end_x..=end_x + 4).any(|x| {
            (center_y - 4..=center_y + 4).any(|y| {
                let pixel = image.get(x as u32, y as u32);
                pixel[2] > 100
            })
        });
        assert!(right_half_contains_tick);
    }

    #[test]
    fn strong_drum_roll_still_draws_ticks() {
        let layout = GifLayout {
            segment_width: 100,
            row_height: 80,
            left_panel_width: 0,
            right_panel_width: 200,
            image_width: 216,
            image_height: 96,
            normal_note_diameter: 38,
            big_note_diameter: 58,
            time_range: 1000.0,
            playfield_left: 0,
            first_row_top: 0,
            row_stride: 80,
        };
        let hit_object = PreparedTaikoHitObject {
            hit_object: TaikoHitObject {
                start_time: 0,
                end_time: 1000,
                hit_type: DRUMROLL_FLAG,
                hitsound: HIT_SOUNDS_STRONG,
            },
            start_multiplier: 1.0,
            end_multiplier: 1.0,
            min_multiplier: 1.0,
            max_multiplier: 1.0,
            drum_roll_ticks: vec![PreparedAnimationPoint {
                time: 500.0,
                multiplier: 1.0,
            }],
        };
        let mut image = Img::new(216, 96, [0, 0, 0, 255]);
        let mut cache = RenderCache::default();

        draw_hit_object(&mut image, &hit_object, &layout, 0, 0, &mut cache);

        let tick_x = object_x(500.0, 0.0, 1.0, &layout);
        let tick_y = gif_row_center_y(0, &layout);
        assert_eq!(
            image.get(tick_x as u32, tick_y as u32),
            [255, 255, 255, 255]
        );
    }
}
