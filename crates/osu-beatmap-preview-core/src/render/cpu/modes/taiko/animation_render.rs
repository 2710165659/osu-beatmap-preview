//! osu!taiko GIF 宫格预览与 GIF、视频共用的单画面动画绘制。

use crate::render::cpu::modes::taiko::constants as taiko_constants;

use crate::domain::errors::{PreviewError, Result};
use crate::domain::models::{Beatmap, TaikoHitObject, TimingPoint};
use crate::domain::mods::ModSettings;
use crate::domain::parser::round_half_even;
use crate::domain::shared::time_selection::{GifRenderOptions, PreviewSegmentTiming};
use crate::domain::timeout::RequestDeadline;
use crate::render::canvas::Img;
use crate::render::cpu::AnimationFrames;
use crate::render::shared::draw::OverlayCanvas;
use crate::render::text::{draw_text, text_size};
use crate::render::visibility::{Flashlight, FlashlightKind, VisibilityTimeline};
use std::cell::RefCell;

use super::animation::{
    drum_roll_tick_transform, generate_drum_roll_ticks, generate_measure_lines, measure_line_alpha,
    required_swell_hits, swell_hit_progress, SWELL_BODY_FADE_MS, SWELL_CLEAR_MS,
    SWELL_TRANSITION_MS,
};
use super::constants::*;
use super::notes::{
    cached_drum_roll_tick_arc, cached_note_disc_arc, cached_roll_tail_arc, draw_drum_panel,
    draw_track_background, RenderCache,
};
use super::timing::*;

pub(crate) trait TaikoReplayHitResult: Copy {
    fn is_hit(self) -> bool;
}

impl TaikoReplayHitResult for bool {
    fn is_hit(self) -> bool {
        self
    }
}

#[cfg(feature = "replay")]
impl TaikoReplayHitResult for crate::gameplay::events::HitResult {
    fn is_hit(self) -> bool {
        crate::gameplay::events::HitResult::is_hit(self)
    }
}

pub(crate) struct TaikoReplayDrawState<'a, T> {
    pub head: Option<(f64, T)>,
    pub end: Option<(f64, T)>,
    pub nodes: &'a [Option<(f64, T)>],
}

#[inline]
pub fn pyround(v: f64) -> i64 {
    round_half_even(v)
}

// ─── GIF 辅助函数 ───

fn gif_judgement_line_offset(row_height: i64) -> i64 {
    pyround(
        crate::render::cpu::modes::taiko::constants::REFERENCE_JUDGEMENT_X * row_height as f64
            / crate::render::cpu::modes::taiko::constants::TAIKO_BASE_HEIGHT,
    )
}

// ─── 倍率与预处理对象 ───

#[derive(Debug, Clone, Copy)]
pub struct MultiplierPoint {
    pub time: f64,
    pub multiplier: f64,
}

pub struct MultiplierLookup {
    pub points: Vec<MultiplierPoint>,
}

impl MultiplierLookup {
    fn at(&self, time: f64) -> f64 {
        let idx = self.points.partition_point(|p| p.time <= time);
        self.points[idx.saturating_sub(1)].multiplier
    }
}

#[derive(Debug, Clone)]
pub struct PreparedTaikoHitObject {
    pub hit_object: TaikoHitObject,
    pub start_multiplier: f64,
    pub end_multiplier: f64,
    pub min_multiplier: f64,
    pub max_multiplier: f64,
    pub drum_roll_ticks: Vec<PreparedAnimationPoint>,
    pub hidden: bool,
    pub swell_required_hits: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct PreparedAnimationPoint {
    pub time: f64,
    pub multiplier: f64,
}

#[derive(Debug, Clone)]
pub struct AnimationLayout {
    pub segment_width: i64,
    pub row_height: i64,
    pub left_panel_width: i64,
    pub right_panel_width: i64,
    pub image_width: i64,
    pub image_height: i64,
    pub normal_note_diameter: i64,
    pub big_note_diameter: i64,
    pub time_range: f64,
    pub playfield_left: i64,
    pub first_row_top: i64,
    pub row_stride: i64,
    pub render_scale: f64,
    /// 轨道内容框；GIF 每格和 MP4 都使用独立的完整画布。
    /// 视频物件层的帧就是最终画布，底色只填这块区域，物件允许溢出到画布边缘。
    pub content: crate::render::geometry::PixelRect,
    pub track_background_color: [u8; 4],
    pub track_edge_color: [u8; 4],
    pub track_accent_color: [u8; 4],
    pub judgement_line_color: [u8; 4],
    /// 转盘按实际输出格式读取配置，避免 GIF 与 MP4 混用样式或字号。
    pub output_format: crate::render::geometry::OutputFormat,
}

// ─── 公共 API ───

pub fn prepare_taiko_gif_frames(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    options: GifRenderOptions,
    fps: Option<u32>,
    deadline: &RequestDeadline,
) -> Result<AnimationFrames> {
    deadline.check()?;
    let (times_ms, duration_seconds, time_axis) = options.into_parts();
    prepare_taiko_segment_gif_frames(
        beatmap,
        mods,
        times_ms,
        duration_seconds,
        time_axis,
        fps,
        deadline,
    )
}

fn prepare_taiko_segment_gif_frames(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    times_ms: crate::domain::shared::time_selection::PreviewTimeSelection,
    duration_seconds: Option<f64>,
    time_axis: crate::domain::shared::time_selection::TimeAxis,
    fps: Option<u32>,
    deadline: &RequestDeadline,
) -> Result<AnimationFrames> {
    deadline.check()?;
    let hit_objects = apply_taiko_object_mods(taiko_hit_objects(beatmap), mods);
    if hit_objects.is_empty() {
        return Err(PreviewError::render("taiko beatmap has no hit objects"));
    }

    let speed_multiplier = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let segment_duration_ms = duration_seconds
        .map(|seconds| seconds * 1000.0)
        .unwrap_or(crate::config::current().render.taiko.gif.style.DURATION_MS);
    let gameplay_segment_duration = pyround(segment_duration_ms * speed_multiplier);
    let fps = fps
        .map(f64::from)
        .unwrap_or(crate::config::current().render.taiko.gif.style.FPS);

    let spans: Vec<(i64, i64)> = hit_objects
        .iter()
        .map(|h| (h.start_time, h.end_time))
        .collect();
    let config = crate::config::current();
    let structure = &config.render.taiko.gif.structure;
    let segment_count = structure
        .ROW_COUNT
        .checked_mul(structure.IMAGES_PER_ROW)
        .filter(|&count| {
            count > 0
                && structure.ROW_COUNT <= u16::MAX as usize
                && structure.IMAGES_PER_ROW <= u16::MAX as usize
        })
        .ok_or_else(|| PreviewError::render("太鼓 GIF 网格行列数必须为正数且不能溢出"))?;
    let segment_timings: Vec<PreviewSegmentTiming> =
        times_ms.choose(beatmap, spans, segment_count, gameplay_segment_duration)?;

    let slider_multiplier = effective_slider_multiplier(beatmap, mods)?;
    let timing_points = effective_timing_points(beatmap, mods);

    let multiplier_lookup = MultiplierLookup {
        points: build_multiplier_points(&timing_points, slider_multiplier),
    };
    let slider_tick_rate = beatmap.difficulty.get_f64_or("SliderTickRate", 1.0);
    let prepared_hit_objects = prepare_hit_objects_with_mods(
        &hit_objects,
        &multiplier_lookup,
        &timing_points,
        slider_tick_rate,
        effective_overall_difficulty(beatmap, mods),
        mods,
    );
    let flashlight = mods
        .is_some_and(|mods| mods.flashlight)
        .then(|| taiko_visibility_timeline(&hit_objects, &beatmap.break_periods));
    let prepared_measure_lines = prepare_measure_lines(
        &hit_objects,
        &timing_points,
        &multiplier_lookup,
        crate::config::current()
            .render
            .taiko
            .gif
            .style
            .SHOW_MEASURE_LINES,
    );
    // DT/HT 已作用于谱面时钟；滚动窗口保持谱面时间单位，避免倍率重复生效。
    let time_range = compute_time_range();

    let layout =
        build_video_animation_layout(time_range, crate::render::geometry::OutputFormat::Gif);
    let (canvas_width, canvas_height) = gif_canvas_size(&layout);
    if canvas_width <= 0
        || canvas_height <= 0
        || canvas_width > u16::MAX as i64
        || canvas_height > u16::MAX as i64
    {
        return Err(PreviewError::render("太鼓 GIF 画布尺寸超出格式范围"));
    }
    let origins: Vec<_> = (0..segment_timings.len())
        .map(|index| gif_frame_origin(index, &layout))
        .collect();
    let autoplay = mods
        .is_some_and(|mods| mods.autoplay)
        .then(|| super::autoplay::Autoplay::new(&prepared_hit_objects, &layout));
    let frame_count = pyround(segment_duration_ms * fps / 1000.0).max(1) as usize;

    let segment_snapshot_times: Vec<Vec<i64>> = segment_timings
        .iter()
        .map(|timing| {
            (0..frame_count)
                .map(|frame_index| {
                    timing.start_time
                        + pyround(frame_index as f64 * 1000.0 * speed_multiplier / fps)
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

    // 每格先在完整画布内绘制，再拼入宫格，转盘和 FL 不会影响相邻片段或时间标签。
    let mut static_bg = Img::new(
        layout.image_width as u32,
        layout.image_height as u32,
        taiko_constants::gif::style::IMAGE_BACKGROUND,
    );
    draw_row_background(&mut static_bg, &layout, 0);

    let render = move |frame_index: usize| -> Img {
        let mut canvas = Img::new(
            canvas_width as u32,
            canvas_height as u32,
            taiko_constants::gif::style::IMAGE_BACKGROUND,
        );
        for (segment_index, (timing, snapshot_times)) in segment_timings
            .iter()
            .zip(&segment_snapshot_times)
            .enumerate()
        {
            let snapshot_time = snapshot_times[frame_index];
            let mut frame = static_bg.clone();
            TAIKO_GIF_CACHE.with(|cache| {
                draw_hit_objects(
                    &mut frame,
                    &prepared_hit_objects,
                    &prepared_measure_lines,
                    &layout,
                    0,
                    snapshot_time,
                    &mut cache.borrow_mut(),
                );
                if let Some(autoplay) = &autoplay {
                    autoplay.draw(
                        &mut frame,
                        &layout,
                        0,
                        snapshot_time,
                        &mut cache.borrow_mut(),
                    );
                }
            });
            if let Some(timeline) = &flashlight {
                taiko_flashlight(timeline, snapshot_time, &layout, 0).apply(
                    &mut frame,
                    crate::render::geometry::PixelRect {
                        x: 0,
                        y: 0,
                        width: layout.image_width,
                        height: layout.image_height,
                    },
                );
            }
            let (x, y) = origins[segment_index];
            canvas.alpha_composite(&frame, x, y);
            if taiko_constants::gif::style::SHOW_TIME_LABEL {
                draw_time_label(
                    &mut canvas,
                    timing.start_time,
                    gameplay_segment_duration,
                    (x, y),
                    &layout,
                    timing.is_preview,
                    time_axis,
                );
            }
        }
        canvas
    };

    Ok(AnimationFrames::with_frame_rate(frame_count, fps, render))
}

// ─── 时间范围与倍率 ───

pub fn compute_time_range() -> f64 {
    let in_length = crate::render::cpu::modes::taiko::constants::ASPECT_RATIO
        * crate::render::cpu::modes::taiko::constants::STABLE_GAMEFIELD_HEIGHT
        - crate::render::cpu::modes::taiko::constants::STABLE_HIT_LOCATION;
    in_length / 100.0 * 1000.0 / crate::render::cpu::modes::taiko::constants::VELOCITY_MULTIPLIER
}

pub fn build_multiplier_points(
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

pub fn prepare_hit_objects(
    hit_objects: &[TaikoHitObject],
    multiplier_lookup: &MultiplierLookup,
    timing_points: &[TimingPoint],
    slider_tick_rate: f64,
) -> Vec<PreparedTaikoHitObject> {
    prepare_hit_objects_with_mods(
        hit_objects,
        multiplier_lookup,
        timing_points,
        slider_tick_rate,
        5.0,
        None,
    )
}

pub fn prepare_hit_objects_with_mods(
    hit_objects: &[TaikoHitObject],
    multiplier_lookup: &MultiplierLookup,
    timing_points: &[TimingPoint],
    slider_tick_rate: f64,
    overall_difficulty: f64,
    mods: Option<&ModSettings>,
) -> Vec<PreparedTaikoHitObject> {
    hit_objects
        .iter()
        .map(|hit_object| {
            let start_multiplier = multiplier_lookup.at(hit_object.start_time as f64);
            let end_multiplier = multiplier_lookup.at(hit_object.end_time as f64);
            PreparedTaikoHitObject {
                hit_object: hit_object.clone(),
                start_multiplier,
                end_multiplier,
                min_multiplier: start_multiplier.min(end_multiplier),
                max_multiplier: start_multiplier.max(end_multiplier),
                hidden: mods.is_some_and(|mods| mods.hidden),
                swell_required_hits: if hit_object.hit_type & SWELL_FLAG != 0 {
                    required_swell_hits(
                        hit_object.end_time - hit_object.start_time,
                        overall_difficulty,
                    )
                } else {
                    0
                },
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

/// 普通音符从 preempt 起线性淡出，经过滚动距离的 37.5% 后完全消失。
/// 连打条/气球按头部时间整体淡出，连打刻度仍跟随自己的 SV。
pub fn hidden_alpha(note_time: f64, snapshot_time: i64, multiplier: f64, time_range: f64) -> f64 {
    let preempt = time_range / multiplier;
    if preempt <= 0.0 {
        return 0.0;
    }
    (((note_time - snapshot_time as f64) / preempt - 0.625) / 0.375).clamp(0.0, 1.0)
}

pub fn taiko_visibility_timeline(
    objects: &[TaikoHitObject],
    breaks: &[crate::domain::models::BreakPeriod],
) -> VisibilityTimeline {
    VisibilityTimeline::new(
        objects
            .iter()
            .filter(|object| object.hit_type & (DRUMROLL_FLAG | SWELL_FLAG) == 0)
            .map(|object| object.start_time as f64),
        breaks,
    )
}

pub fn taiko_flashlight(
    timeline: &VisibilityTimeline,
    time: i64,
    layout: &AnimationLayout,
    row: i64,
) -> Flashlight {
    Flashlight {
        center: [
            judgement_line_x(layout) as f64,
            gif_row_center_y(row, layout) as f64,
        ],
        radius: 200.0 * layout.row_height as f64 / TAIKO_BASE_HEIGHT
            * timeline.flashlight_scale(FlashlightKind::Taiko, time),
        band: false,
        dim: 0.0,
    }
}

pub fn prepare_measure_lines(
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

fn gif_unit_size(layout: &AnimationLayout) -> (i64, i64) {
    let sizing = taiko_constants::gif::sizing();
    let info_bottom = if taiko_constants::gif::style::SHOW_TIME_LABEL {
        sizing.INFO_MARGIN_BOTTOM
    } else {
        0
    };
    (
        sizing.INFO_MARGIN_LEFT + layout.image_width + sizing.INFO_MARGIN_RIGHT,
        sizing.INFO_MARGIN_TOP + layout.image_height + info_bottom,
    )
}

fn gif_canvas_size(layout: &AnimationLayout) -> (i64, i64) {
    let config = crate::config::current();
    let structure = &config.render.taiko.gif.structure;
    let sizing = taiko_constants::gif::sizing();
    let (unit_width, unit_height) = gif_unit_size(layout);
    (
        sizing.PAGE_MARGIN_LEFT
            + sizing.PAGE_MARGIN_RIGHT
            + structure.IMAGES_PER_ROW as i64 * unit_width
            + (structure.IMAGES_PER_ROW as i64 - 1) * sizing.GRID_GAP,
        sizing.PAGE_MARGIN_TOP
            + sizing.PAGE_MARGIN_BOTTOM
            + structure.ROW_COUNT as i64 * unit_height
            + (structure.ROW_COUNT as i64 - 1) * sizing.GRID_GAP,
    )
}

fn gif_frame_origin(index: usize, layout: &AnimationLayout) -> (i64, i64) {
    let columns = crate::config::current()
        .render
        .taiko
        .gif
        .structure
        .IMAGES_PER_ROW;
    let sizing = taiko_constants::gif::sizing();
    let (unit_width, unit_height) = gif_unit_size(layout);
    (
        sizing.PAGE_MARGIN_LEFT
            + sizing.INFO_MARGIN_LEFT
            + (index % columns) as i64 * (unit_width + sizing.GRID_GAP),
        sizing.PAGE_MARGIN_TOP
            + sizing.INFO_MARGIN_TOP
            + (index / columns) as i64 * (unit_height + sizing.GRID_GAP),
    )
}

pub fn build_animation_layout_with_format(
    time_range: f64,
    output_format: crate::render::geometry::OutputFormat,
) -> AnimationLayout {
    let geometry = crate::render::geometry::taiko_geometry(output_format);
    let render_scale = crate::render::geometry::taiko_playfield_scale(output_format);
    let row_height = geometry.playfield.height;
    let left_panel_width = pyround(
        row_height as f64 * crate::render::cpu::modes::taiko::constants::DRUM_PANEL_WIDTH_RATIO,
    );
    let right_panel_width = geometry.playfield.width - left_panel_width;
    let inner_padding = crate::render::geometry::scale_px(
        crate::render::cpu::modes::taiko::constants::ROW_INNER_PADDING_X as f64,
        render_scale,
    );
    let segment_width = (right_panel_width - inner_padding * 2).max(1);

    let (image_width, image_height, playfield_left, first_row_top, row_stride) =
        (geometry.content.width, row_height, 0, 0, row_height);

    let normal_note_diameter = pyround(
        row_height as f64 * crate::render::cpu::modes::taiko::constants::NORMAL_NOTE_SIZE_RATIO,
    );
    let big_note_diameter = pyround(
        normal_note_diameter as f64 * crate::render::cpu::modes::taiko::constants::BIG_NOTE_SCALE,
    );
    let (track_background_color, track_edge_color, track_accent_color, judgement_line_color) =
        match output_format {
            crate::render::geometry::OutputFormat::Gif => (
                taiko_constants::gif::style::TRACK_BACKGROUND_COLOR,
                taiko_constants::gif::style::TRACK_EDGE_COLOR,
                taiko_constants::gif::style::TRACK_ACCENT_COLOR,
                taiko_constants::gif::style::JUDGEMENT_LINE_COLOR,
            ),
            crate::render::geometry::OutputFormat::Mp4 => (
                taiko_constants::mp4::style::TRACK_BACKGROUND_COLOR,
                taiko_constants::mp4::style::TRACK_EDGE_COLOR,
                taiko_constants::mp4::style::TRACK_ACCENT_COLOR,
                taiko_constants::mp4::style::JUDGEMENT_LINE_COLOR,
            ),
            crate::render::geometry::OutputFormat::Png => unreachable!("PNG 不使用动画布局"),
        };

    AnimationLayout {
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
        render_scale,
        content: crate::render::geometry::PixelRect {
            x: playfield_left,
            y: first_row_top,
            width: geometry.content.width,
            height: row_height,
        },
        track_background_color,
        track_edge_color,
        track_accent_color,
        judgement_line_color,
        output_format,
    }
}

/// 完整动画画布：GIF 使用宫格单帧尺寸，实时预览与 MP4 使用视频尺寸。
/// 轨道按游戏的相对高度定位，物件只在完整画布边界裁剪。
pub fn build_video_animation_layout(
    time_range: f64,
    output_format: crate::render::geometry::OutputFormat,
) -> AnimationLayout {
    let layout = build_animation_layout_with_format(time_range, output_format);
    let geometry = crate::render::geometry::taiko_geometry(output_format);
    let canvas = if output_format == crate::render::geometry::OutputFormat::Gif {
        let (width, height) =
            crate::render::geometry::preview_frame_size(crate::render::geometry::output_scale(
                crate::render::geometry::GameMode::Taiko,
                output_format,
            ));
        crate::render::geometry::VideoCanvas {
            width: width as u32,
            height: height as u32,
            origin_x: 0,
            origin_y: 0,
        }
    } else {
        crate::render::geometry::video_canvas(geometry.content)
    };
    let top = crate::render::geometry::taiko_playfield_top(canvas.height);
    AnimationLayout {
        image_width: canvas.width as i64,
        image_height: canvas.height as i64,
        playfield_left: layout.playfield_left + canvas.origin_x,
        first_row_top: layout.first_row_top + top,
        content: crate::render::geometry::PixelRect {
            x: canvas.origin_x,
            y: top,
            width: geometry.content.width,
            height: layout.row_height,
        },
        ..layout
    }
}

pub fn gif_row_top(row_index: i64, layout: &AnimationLayout) -> i64 {
    layout.first_row_top + row_index * layout.row_stride
}

pub fn gif_row_center_y(row_index: i64, layout: &AnimationLayout) -> i64 {
    gif_row_top(row_index, layout) + layout.row_height / 2
}

pub fn judgement_line_x(layout: &AnimationLayout) -> i64 {
    layout.playfield_left + layout.left_panel_width + gif_judgement_line_offset(layout.row_height)
}

// ─── 绘制 ───

fn draw_judgement_circle(image: &mut Img, layout: &AnimationLayout, row_index: i64) {
    let diameter = layout.normal_note_diameter as f64;
    image.stroke_circle_aa(
        judgement_line_x(layout) as f64,
        gif_row_center_y(row_index, layout) as f64,
        diameter / 2.0,
        diameter * NOTE_RING_THICKNESS_RATIO,
        layout.judgement_line_color,
    );
}

/// 绘制单段背景：鼓面板 + 轨道 + 判定圆（程序化，无图片）。
pub fn draw_row_background(image: &mut Img, layout: &AnimationLayout, row_index: i64) {
    let row_top = gif_row_top(row_index, layout);

    draw_drum_panel(
        image,
        layout.playfield_left,
        row_top,
        layout.left_panel_width,
        layout.row_height,
        layout.track_accent_color,
        layout.render_scale,
    );
    draw_track_background(
        image,
        layout.playfield_left + layout.left_panel_width,
        row_top,
        layout.right_panel_width,
        layout.row_height,
        layout.track_background_color,
        layout.track_edge_color,
        crate::render::geometry::scale_stroke_px(1.0, layout.render_scale),
    );

    draw_judgement_circle(image, layout, row_index);
}

pub fn draw_hit_objects(
    image: &mut Img,
    hit_objects: &[PreparedTaikoHitObject],
    measure_lines: &[PreparedAnimationPoint],
    layout: &AnimationLayout,
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
    layout: &AnimationLayout,
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
        let width = crate::render::geometry::scale_stroke_px(
            MEASURE_LINE_WIDTH as f64,
            layout.render_scale,
        );
        let left = center_x - width / 2;
        let right = left + width;
        if right <= left_bound || left >= right_bound {
            continue;
        }
        let mut color = base_color;
        color[3] = pyround(color[3] as f64 * alpha).clamp(0, 255) as u8;
        image.fill_rect_size(
            left.max(left_bound),
            top,
            right.min(right_bound) - left.max(left_bound),
            height,
            color,
        );
    }
}

/// Overlapping PositionAt：x = judgement_x + (t - now) / timeRange * multiplier * scrollLength。
pub fn object_x(
    note_time: f64,
    snapshot_time: f64,
    multiplier: f64,
    layout: &AnimationLayout,
) -> i64 {
    let judgement_x = judgement_line_x(layout);
    let offset =
        (note_time - snapshot_time) / layout.time_range * multiplier * layout.segment_width as f64;
    pyround(judgement_x as f64 + offset)
}

pub fn can_skip(
    hit_object: &PreparedTaikoHitObject,
    snapshot_time: i64,
    layout: &AnimationLayout,
    left_bound: i64,
    right_bound: i64,
) -> bool {
    let base = &hit_object.hit_object;
    // 滚奏的圆头在开始后固定于判定点，不能再按已滚走的起点裁掉。
    if base.hit_type & DRUMROLL_FLAG != 0 {
        if snapshot_time >= base.end_time {
            return true;
        }
        if snapshot_time >= base.start_time {
            return false;
        }
    }
    // 活动转盘和完成提示不受轨道裁剪，生命周期沿用经典皮肤的完成动画。
    if base.hit_type & SWELL_FLAG != 0 {
        let (_, completed_at) =
            swell_hit_progress(base, hit_object.swell_required_hits, snapshot_time);
        if snapshot_time as f64 > completed_at + SWELL_CLEAR_MS {
            return true;
        }
        let x = object_x(
            base.start_time as f64,
            snapshot_time as f64,
            hit_object.start_multiplier,
            layout,
        )
        .max(left_bound);
        return x - layout.big_note_diameter / 2 > right_bound;
    }
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

pub(crate) fn draw_hit_object(
    image: &mut Img,
    hit_object: &PreparedTaikoHitObject,
    layout: &AnimationLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
) {
    draw_hit_object_state::<bool>(
        image,
        hit_object,
        layout,
        row_index,
        snapshot_time,
        cache,
        None,
    );
}

pub(crate) fn draw_hit_object_state<T: TaikoReplayHitResult>(
    target: &mut impl OverlayCanvas,
    hit_object: &PreparedTaikoHitObject,
    layout: &AnimationLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
    replay_state: Option<&TaikoReplayDrawState<'_, T>>,
) {
    let base = &hit_object.hit_object;
    if base.hit_type & DRUMROLL_FLAG != 0 {
        let ended = snapshot_time >= base.end_time;
        if (ended && (replay_state.is_none() || snapshot_time - base.end_time >= 100))
            || (!ended
                && can_skip(
                    hit_object,
                    snapshot_time,
                    layout,
                    judgement_line_x(layout),
                    layout.playfield_left + layout.left_panel_width + layout.right_panel_width,
                ))
        {
            return;
        }
    }
    if hit_object.hidden && base.hit_type & DRUMROLL_FLAG != 0 {
        let alpha = hidden_alpha(
            base.start_time as f64,
            snapshot_time,
            hit_object.start_multiplier,
            layout.time_range,
        );
        if alpha <= 0.0 {
            return;
        }
        // 整条物件先合成再淡出，避免头尾重叠处因重复混合而出现亮斑。
        let (width, height) = target.dimensions();
        let mut visible = hit_object.clone();
        visible.hidden = false;
        let mut layer = Img::new(width, height, [0; 4]);
        draw_hit_object_state(
            &mut layer,
            &visible,
            layout,
            row_index,
            snapshot_time,
            cache,
            replay_state,
        );
        target.sprite_opacity(&std::sync::Arc::new(layer), 0, 0, alpha);
        return;
    }
    if base.hit_type & SWELL_FLAG != 0 {
        let progress = replay_state.map(|state| {
            let completed = state
                .nodes
                .iter()
                .flatten()
                .filter(|(time, result)| *time <= snapshot_time as f64 && result.is_hit())
                .count() as u32;
            let finished = state.end.map_or(f64::INFINITY, |judgement| judgement.0);
            (completed, finished)
        });
        let completed_at = progress.map_or_else(
            || swell_hit_progress(base, hit_object.swell_required_hits, snapshot_time).1,
            |(_, finished)| finished,
        );
        if snapshot_time as f64 > completed_at + SWELL_CLEAR_MS {
            return;
        }
        let right_bound =
            layout.playfield_left + layout.left_panel_width + layout.right_panel_width;
        let x = object_x(
            base.start_time as f64,
            snapshot_time as f64,
            hit_object.start_multiplier,
            layout,
        );
        if x - layout.big_note_diameter / 2 > right_bound {
            return;
        }
        let alpha = if hit_object.hidden {
            hidden_alpha(
                base.start_time as f64,
                snapshot_time,
                hit_object.start_multiplier,
                layout.time_range,
            )
        } else {
            1.0
        };
        if alpha <= 0.0 {
            return;
        }
        let (center_x, center_y, diameter) =
            swell_frame_geometry(hit_object, layout, row_index, snapshot_time);
        let (width, height) = target.dimensions();
        let padding = crate::render::geometry::scale_stroke_px(3.0, layout.render_scale);
        // 局部原点取偶数，保持 round-half-even 在半像素边界的奇偶性。
        let left = ((center_x - diameter as f64 / 2.0).floor() as i64 - padding)
            .max(0)
            .div_euclid(2)
            * 2;
        let top = ((center_y - diameter as f64 / 2.0).floor() as i64 - padding)
            .max(0)
            .div_euclid(2)
            * 2;
        let right =
            ((center_x + diameter as f64 / 2.0).ceil() as i64 + padding).min(i64::from(width));
        let bottom =
            ((center_y + diameter as f64 / 2.0).ceil() as i64 + padding).min(i64::from(height));
        if right <= left || bottom <= top {
            return;
        }
        let mut local_layout = layout.clone();
        local_layout.playfield_left -= left;
        local_layout.first_row_top -= top;
        let mut layer = Img::new((right - left) as u32, (bottom - top) as u32, [0; 4]);
        draw_swell_object_with_progress(
            &mut layer,
            hit_object,
            &local_layout,
            row_index,
            snapshot_time,
            cache,
            progress,
        );
        target.sprite_opacity(&std::sync::Arc::new(layer), left, top, alpha);
        return;
    }
    if base.hit_type & DRUMROLL_FLAG != 0 {
        let is_big_roll = base.hitsound & HIT_SOUNDS_STRONG != 0;
        draw_span_object_state(
            target,
            hit_object,
            layout,
            row_index,
            snapshot_time,
            cache,
            is_big_roll,
            crate::render::cpu::modes::taiko::constants::ROLL_COLOR,
            false,
            replay_state,
        );
        return;
    }
    draw_circle_object(
        target,
        hit_object,
        layout,
        row_index,
        snapshot_time,
        cache,
        replay_state,
    );
}

/// 动画转盘沿用经典皮肤在判定点右下方展开的大圆布局。
/// 返回当前帧的圆心与直径，CPU 绘制和 GPU 局部图层必须共用几何边界。
pub(crate) fn swell_frame_geometry(
    hit_object: &PreparedTaikoHitObject,
    layout: &AnimationLayout,
    row_index: i64,
    snapshot_time: i64,
) -> (f64, f64, i64) {
    let base = &hit_object.hit_object;
    let judgement_x = judgement_line_x(layout) as f64;
    let row_center_y = gif_row_center_y(row_index, layout) as f64;
    // 经典素材以 128px 音符为基准，当前音符尺寸已应用对应输出格式的 SCALE。
    let scale = layout.normal_note_diameter as f64 / 128.0;
    let max_diameter = 666.0 * 0.8 * scale;
    let target_x = judgement_x + 250.0 * scale;
    let target_y = row_center_y + 100.0 * scale;
    let warning_x = object_x(
        base.start_time as f64,
        snapshot_time as f64,
        hit_object.start_multiplier,
        layout,
    )
    .max(judgement_line_x(layout)) as f64;
    let transition =
        ((snapshot_time - base.start_time) as f64 / SWELL_TRANSITION_MS).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - transition).powi(2);
    let diameter = pyround(
        layout.normal_note_diameter as f64
            + (max_diameter - layout.normal_note_diameter as f64) * eased,
    )
    .max(1);
    (
        warning_x + (target_x - warning_x) * eased,
        row_center_y + (target_y - row_center_y) * eased,
        diameter,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_swell_object_with_progress(
    image: &mut Img,
    hit_object: &PreparedTaikoHitObject,
    layout: &AnimationLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
    progress: Option<(u32, f64)>,
) {
    let base = &hit_object.hit_object;
    let elapsed = (snapshot_time - base.start_time) as f64;
    let (completed, completed_at) = progress
        .unwrap_or_else(|| swell_hit_progress(base, hit_object.swell_required_hits, snapshot_time));
    let clear_elapsed = snapshot_time as f64 - completed_at;
    if clear_elapsed > SWELL_CLEAR_MS {
        return;
    }
    let (center_x, center_y, diameter) =
        swell_frame_geometry(hit_object, layout, row_index, snapshot_time);
    let (
        color,
        text_color,
        hit_font,
        clear_font,
        show_hits,
        show_inner,
        show_clear,
        ring_ratio,
        show_marker,
    ) = match layout.output_format {
        crate::render::geometry::OutputFormat::Gif => {
            let sizing = taiko_constants::gif::sizing();
            (
                taiko_constants::gif::style::SWELL_COLOR,
                taiko_constants::gif::style::SWELL_TEXT_COLOR,
                sizing.SWELL_HIT_COUNT_FONT_SIZE,
                sizing.SWELL_CLEAR_FONT_SIZE,
                taiko_constants::gif::style::SHOW_SWELL_HIT_COUNT,
                taiko_constants::gif::style::SHOW_SWELL_APPROACH_CIRCLE,
                taiko_constants::gif::style::SHOW_SWELL_CLEAR,
                taiko_constants::gif::style::SWELL_RING_THICKNESS_RATIO,
                taiko_constants::gif::style::SHOW_SWELL_MARKER,
            )
        }
        crate::render::geometry::OutputFormat::Mp4 => {
            let sizing = taiko_constants::mp4::sizing();
            (
                taiko_constants::mp4::style::SWELL_COLOR,
                taiko_constants::mp4::style::SWELL_TEXT_COLOR,
                sizing.SWELL_HIT_COUNT_FONT_SIZE,
                sizing.SWELL_CLEAR_FONT_SIZE,
                taiko_constants::mp4::style::SHOW_SWELL_HIT_COUNT,
                taiko_constants::mp4::style::SHOW_SWELL_APPROACH_CIRCLE,
                taiko_constants::mp4::style::SHOW_SWELL_CLEAR,
                taiko_constants::mp4::style::SWELL_RING_THICKNESS_RATIO,
                taiko_constants::mp4::style::SHOW_SWELL_MARKER,
            )
        }
        crate::render::geometry::OutputFormat::Png => unreachable!("PNG 不使用动画转盘"),
    };
    let circle = super::notes::cached_animation_swell_disc(
        cache,
        diameter,
        color,
        ring_ratio,
        show_marker && elapsed < 0.0,
    );
    let alpha = 1.0 - (clear_elapsed / SWELL_BODY_FADE_MS).clamp(0.0, 1.0);
    image.alpha_composite_scaled(
        circle,
        pyround(center_x - diameter as f64 / 2.0),
        pyround(center_y - diameter as f64 / 2.0),
        alpha,
    );
    if elapsed < 0.0 {
        return;
    }
    // 内圈和文字跟随同一个转盘中心，尺寸限制在外圈内。
    if show_inner && alpha > 0.0 {
        let progress = (elapsed / (base.end_time - base.start_time).max(1) as f64).clamp(0.0, 1.0);
        let inner = diameter as f64 * (0.78 - 0.68 * progress);
        let mut inner_color = text_color;
        inner_color[3] = pyround(text_color[3] as f64 * alpha * 0.8) as u8;
        image.stroke_circle_aa(center_x, center_y, inner / 2.0, inner * 0.035, inner_color);
    }
    let label = if clear_elapsed >= 0.0 {
        if !show_clear {
            return;
        }
        if completed >= hit_object.swell_required_hits {
            "CLEAR"
        } else {
            "MISS"
        }
        .to_owned()
    } else {
        if !show_hits {
            return;
        }
        hit_object
            .swell_required_hits
            .saturating_sub(completed)
            .to_string()
    };
    let font = if clear_elapsed >= 0.0 {
        clear_font
    } else {
        hit_font
    };
    let (width, height) = text_size(&label, font);
    // 开始击打后隐藏预告标志，数字独占圆心；字号仍限制在外圈内。
    let available = (diameter as f64 * 0.75).max(1.0) as u32;
    let font = if width.max(height) > available {
        (font as u64 * available as u64 / width.max(height) as u64).max(1) as u32
    } else {
        font
    };
    let (width, height) = text_size(&label, font);
    let mut text_color = text_color;
    let text_alpha = if clear_elapsed >= 0.0 {
        (1.0 - clear_elapsed / SWELL_CLEAR_MS).clamp(0.0, 1.0)
    } else {
        alpha
    };
    text_color[3] = pyround(text_color[3] as f64 * text_alpha) as u8;
    draw_text(
        image,
        pyround(center_x - width as f64 / 2.0),
        pyround(center_y - height as f64 / 2.0),
        &label,
        font,
        text_color,
    );
}

fn draw_circle_object<T: TaikoReplayHitResult>(
    target: &mut impl OverlayCanvas,
    hit_object: &PreparedTaikoHitObject,
    layout: &AnimationLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
    replay_state: Option<&TaikoReplayDrawState<'_, T>>,
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
    if (replay_state.is_none() && center_x < judgement_x) || center_x > right_bound {
        return;
    }

    let judged = replay_state
        .and_then(|state| state.head)
        .filter(|(time, _)| *time <= snapshot_time as f64);
    if judged.is_some_and(|(_, result)| result.is_hit()) {
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
        crate::render::cpu::modes::taiko::constants::RIM_NOTE_COLOR
    } else {
        crate::render::cpu::modes::taiko::constants::CENTRE_NOTE_COLOR
    };

    let mut alpha = if hit_object.hidden {
        hidden_alpha(
            base.start_time as f64,
            snapshot_time,
            hit_object.start_multiplier,
            layout.time_range,
        )
    } else {
        1.0
    };
    if let Some((judged_at, _)) = judged {
        alpha *= (1.0 - (snapshot_time as f64 - judged_at) / 100.0).clamp(0.0, 1.0);
    }
    if alpha > 0.0 {
        let sprite = cached_note_disc_arc(
            cache,
            color,
            diameter,
            crate::render::geometry::scale_stroke_px(1.0, layout.render_scale),
            false,
        );
        target.sprite_opacity(
            sprite,
            pyround(center_x as f64 - diameter as f64 / 2.0),
            pyround(center_y as f64 - diameter as f64 / 2.0),
            alpha,
        );
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn draw_span_object(
    image: &mut Img,
    hit_object: &PreparedTaikoHitObject,
    layout: &AnimationLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
    use_large_geometry: bool,
    span_color: [u8; 3],
    draw_swell_marker: bool,
) {
    draw_span_object_state::<bool>(
        image,
        hit_object,
        layout,
        row_index,
        snapshot_time,
        cache,
        use_large_geometry,
        span_color,
        draw_swell_marker,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_span_object_state<T: TaikoReplayHitResult>(
    target: &mut impl OverlayCanvas,
    hit_object: &PreparedTaikoHitObject,
    layout: &AnimationLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
    use_large_geometry: bool,
    span_color: [u8; 3],
    draw_swell_marker: bool,
    replay_state: Option<&TaikoReplayDrawState<'_, T>>,
) {
    let end_time = hit_object.hit_object.end_time;
    if replay_state.is_some() && snapshot_time >= end_time {
        let alpha = (1.0 - (snapshot_time - end_time) as f64 / 100.0).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        let (width, height) = target.dimensions();
        let diameter = if use_large_geometry {
            layout.big_note_diameter
        } else {
            layout.normal_note_diameter
        };
        let center_x = judgement_line_x(layout);
        let center_y = gif_row_center_y(row_index, layout);
        let padding = crate::render::geometry::scale_stroke_px(2.0, layout.render_scale);
        let left = (center_x - diameter / 2 - padding).max(0).div_euclid(2) * 2;
        let top = (center_y - diameter / 2 - padding).max(0).div_euclid(2) * 2;
        let tail_width = pyround(diameter as f64 * SPAN_BODY_HEIGHT_RATIO / 2.0).max(1);
        let right = (center_x + diameter / 2 + tail_width + padding).min(i64::from(width));
        let bottom = (center_y + diameter / 2 + padding).min(i64::from(height));
        if right <= left || bottom <= top {
            return;
        }
        let mut local_layout = layout.clone();
        local_layout.playfield_left -= left;
        local_layout.first_row_top -= top;
        local_layout.image_width = right - left;
        local_layout.image_height = bottom - top;
        let mut layer = Img::new(
            local_layout.image_width as u32,
            local_layout.image_height as u32,
            [0; 4],
        );
        draw_span_object_inner(
            &mut layer,
            hit_object,
            &local_layout,
            row_index,
            snapshot_time,
            cache,
            use_large_geometry,
            span_color,
            draw_swell_marker,
            replay_state,
            true,
        );
        target.sprite_opacity(&std::sync::Arc::new(layer), left, top, alpha);
    } else {
        draw_span_object_inner(
            target,
            hit_object,
            layout,
            row_index,
            snapshot_time,
            cache,
            use_large_geometry,
            span_color,
            draw_swell_marker,
            replay_state,
            false,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_span_object_inner<T: TaikoReplayHitResult>(
    target: &mut impl OverlayCanvas,
    hit_object: &PreparedTaikoHitObject,
    layout: &AnimationLayout,
    row_index: i64,
    snapshot_time: i64,
    cache: &mut RenderCache,
    use_large_geometry: bool,
    span_color: [u8; 3],
    draw_swell_marker: bool,
    replay_state: Option<&TaikoReplayDrawState<'_, T>>,
    draw_after_end: bool,
) {
    let base = &hit_object.hit_object;
    // 只有尾巴随时间缩短；到结束时整条物件（包括圆头）一次性消失。
    if snapshot_time >= base.end_time && !draw_after_end {
        return;
    }
    let start_x = object_x(
        base.start_time as f64,
        snapshot_time as f64,
        hit_object.start_multiplier,
        layout,
    )
    .max(judgement_line_x(layout));
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
        crate::render::cpu::modes::taiko::constants::SWELL_BODY_HEIGHT_RATIO
    } else {
        crate::render::cpu::modes::taiko::constants::SPAN_BODY_HEIGHT_RATIO
    };
    let body_height = pyround(head_diameter as f64 * body_ratio);

    draw_roll_body(
        target,
        span_color,
        start_x,
        end_x,
        center_y,
        body_height,
        clip_left,
        clip_right,
    );
    draw_span_tail(
        target,
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
            target,
            &hit_object.drum_roll_ticks,
            layout,
            snapshot_time,
            center_y,
            cache,
            clip_left,
            clip_right,
            hit_object.hidden,
            replay_state.map(|state| state.nodes),
        );
    }
    draw_span_head(
        target,
        span_color,
        start_x,
        center_y,
        head_diameter,
        cache,
        crate::render::geometry::scale_stroke_px(1.0, layout.render_scale),
        draw_swell_marker,
        // 圆头保持完整，判定点左侧的半圆不参与尾巴裁剪。
        0,
        clip_right,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_drum_roll_ticks<T: TaikoReplayHitResult>(
    target: &mut impl OverlayCanvas,
    ticks: &[PreparedAnimationPoint],
    layout: &AnimationLayout,
    snapshot_time: i64,
    center_y: i64,
    cache: &mut RenderCache,
    clip_left: i64,
    clip_right: i64,
    hidden: bool,
    replay_nodes: Option<&[Option<(f64, T)>]>,
) {
    let base_diameter =
        pyround(layout.normal_note_diameter as f64 * DRUM_ROLL_TICK_DIAMETER_RATIO).max(1);

    for (index, tick) in ticks.iter().enumerate() {
        let judged = replay_nodes
            .and_then(|nodes| nodes.get(index))
            .copied()
            .flatten()
            .filter(|(time, _)| *time <= snapshot_time as f64);
        let (mut alpha, scale) = match judged {
            Some((time, result)) if result.is_hit() => {
                drum_roll_tick_transform(time, snapshot_time)
            }
            Some(_) => (0.0, 1.0),
            // 回放只在实际击中时播放节点动画；普通预览仍按节点时间自动触发。
            None if replay_nodes.is_some() => (1.0, 1.0),
            None => drum_roll_tick_transform(tick.time, snapshot_time),
        };
        if hidden {
            alpha *= hidden_alpha(tick.time, snapshot_time, tick.multiplier, layout.time_range);
        }
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
        let sprite = cached_drum_roll_tick_arc(cache, diameter, alpha);
        target.sprite_clipped(sprite, x, y, clip_left, clip_right, 1.0);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_roll_body(
    target: &mut impl OverlayCanvas,
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
    target.fill_rect_size(
        visible_left,
        y0,
        visible_right - visible_left,
        height,
        [color[0], color[1], color[2], 255],
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_span_head(
    target: &mut impl OverlayCanvas,
    color: [u8; 3],
    center_x: i64,
    center_y: i64,
    diameter: i64,
    cache: &mut RenderCache,
    edge_width: i64,
    draw_swell_marker: bool,
    clip_left: i64,
    clip_right: i64,
) {
    let sprite_x = pyround(center_x as f64 - diameter as f64 / 2.0);
    let sprite_y = pyround(center_y as f64 - diameter as f64 / 2.0);
    let disc = cached_note_disc_arc(cache, color, diameter, edge_width, draw_swell_marker);
    target.sprite_clipped(disc, sprite_x, sprite_y, clip_left, clip_right, 1.0);
}

#[allow(clippy::too_many_arguments)]
fn draw_span_tail(
    target: &mut impl OverlayCanvas,
    color: [u8; 3],
    join_x: i64,
    center_y: i64,
    height: i64,
    cache: &mut RenderCache,
    clip_left: i64,
    clip_right: i64,
) {
    let y = pyround(center_y as f64 - height as f64 / 2.0);
    let tail = cached_roll_tail_arc(cache, color, height);
    target.sprite_clipped(tail, join_x, y, clip_left, clip_right, 1.0);
}

fn draw_time_label(
    image: &mut Img,
    start_time: i64,
    duration_ms: i64,
    frame_origin: (i64, i64),
    layout: &AnimationLayout,
    is_preview: bool,
    time_axis: crate::domain::shared::time_selection::TimeAxis,
) {
    let taiko_gif_sizing = taiko_constants::gif::sizing();
    let (frame_x, frame_y) = frame_origin;
    let y = frame_y + layout.image_height + taiko_gif_sizing.TIME_LABEL_TOP_GAP;
    let label = format!(
        "{} - {}",
        crate::render::text::format_mmssmmm(time_axis.to_display(start_time)),
        crate::render::text::format_mmssmmm(time_axis.to_display(start_time + duration_ms))
    );
    let color = if is_preview {
        taiko_constants::gif::style::PREVIEW_TIME_LABEL_COLOR
    } else {
        taiko_constants::gif::style::TIME_LABEL_COLOR
    };
    let note_color = if is_preview {
        taiko_constants::gif::style::PREVIEW_TIME_LABEL_COLOR
    } else {
        taiko_constants::gif::style::TIME_LABEL_NOTE_COLOR
    };
    crate::render::text::draw_time_label(
        image,
        &label,
        frame_x,
        y,
        is_preview.then_some("Preview Time"),
        layout.image_width,
        taiko_gif_sizing.TIME_LABEL_FONT_SIZE,
        taiko_gif_sizing.TIME_LABEL_NOTE_FONT_SIZE,
        taiko_gif_sizing.TIME_LABEL_NOTE_TOP_GAP,
        color,
        note_color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_swell_uses_the_same_classic_geometry_for_gif_and_video() {
        for scale in [0.5, 1.0, 1.5, 2.0] {
            let mut config = crate::config::CoreConfig::default();
            config.render.taiko.mp4.SCALE = scale;
            crate::config::with_config(std::sync::Arc::new(config), || {
                let layout = build_video_animation_layout(
                    1000.0,
                    crate::render::geometry::OutputFormat::Mp4,
                );
                let objects = prepare_hit_objects(
                    &[TaikoHitObject {
                        start_time: 1000,
                        end_time: 4000,
                        hit_type: SWELL_FLAG,
                        hitsound: 0,
                        samples: Vec::new(),
                    }],
                    &MultiplierLookup {
                        points: build_multiplier_points(&[], 1.0),
                    },
                    &[],
                    1.0,
                );
                let classic_scale = layout.normal_note_diameter as f64 / 128.0;
                let (x, y, diameter) = swell_frame_geometry(&objects[0], &layout, 0, 1200);
                assert_eq!(x, judgement_line_x(&layout) as f64 + 250.0 * classic_scale);
                assert_eq!(
                    y,
                    gif_row_center_y(0, &layout) as f64 + 100.0 * classic_scale
                );
                let radius = diameter as f64 / 2.0;
                assert!(x - radius >= 0.0 && x + radius <= layout.image_width as f64);
                assert!(y - radius >= 0.0 && y + radius <= layout.image_height as f64);
                assert!(
                    y + radius > (layout.first_row_top + layout.row_height) as f64,
                    "视频转盘应展开到轨道外"
                );
                let (_, waiting_y, waiting_diameter) =
                    swell_frame_geometry(&objects[0], &layout, 0, 900);
                assert_eq!(waiting_y, gif_row_center_y(0, &layout) as f64);
                assert_eq!(waiting_diameter, layout.normal_note_diameter);
                let mut gif = layout.clone();
                gif.output_format = crate::render::geometry::OutputFormat::Gif;
                let (_, gif_y, gif_diameter) = swell_frame_geometry(&objects[0], &gif, 0, 1200);
                assert_eq!(gif_y, y);
                assert_eq!(gif_diameter, diameter);
            });
        }
    }

    #[test]
    fn gif_grid_preserves_independent_full_frames_and_format_scales() {
        use crate::render::geometry::OutputFormat;
        let beatmap = crate::domain::parser::parse_beatmap_bytes(
            b"osu file format v14\n[General]\nMode:1\nPreviewTime:1000\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,8,0,4000,0:0:0:0:\n256,192,1100,1,0,0:0:0:0:\n256,192,5000,1,0,0:0:0:0:\n"
        ).unwrap();
        let deadline = RequestDeadline::new(
            std::time::Instant::now(),
            "gif",
            std::time::Duration::from_secs(60),
        );
        for (scale, rows, columns) in [(0.5, 1, 1), (0.75, 2, 2), (1.0, 2, 2), (1.25, 2, 3)] {
            let mut config = crate::config::CoreConfig::default();
            config.render.standard.gif.SCALE = scale;
            config.render.standard.gif.structure.ROW_COUNT = rows;
            config.render.standard.gif.structure.IMAGES_PER_ROW = columns;
            let (standard_canvas, standard_origins, standard_sizing) =
                crate::config::with_config(std::sync::Arc::new(config.clone()), || {
                    use crate::render::cpu::modes::standard::{constants, context};
                    (
                        context::gif_canvas_size(),
                        (0..rows * columns)
                            .map(context::gif_frame_origin)
                            .collect::<Vec<_>>(),
                        constants::gif::sizing(),
                    )
                });
            config.render.standard.gif.SCALE = 2.75;
            config.render.taiko.gif.SCALE = scale;
            config.render.taiko.mp4.SCALE = 3.0;
            config.render.taiko.png.SCALE = 4.0;
            config.render.taiko.gif.structure.ROW_COUNT = rows;
            config.render.taiko.gif.structure.IMAGES_PER_ROW = columns;
            config.render.taiko.gif.style.SHOW_MEASURE_LINES = false;
            crate::config::with_config(std::sync::Arc::new(config), || {
                let times: Vec<_> = (0..rows * columns)
                    .map(|index| 1000 + index as i64 * 100)
                    .collect();
                let mods = ModSettings {
                    autoplay: true,
                    ..ModSettings::new()
                };
                let frames = prepare_taiko_gif_frames(
                    &beatmap,
                    Some(&mods),
                    GifRenderOptions::Segments {
                        times_ms: Some(times.clone()),
                        duration_seconds: Some(0.1),
                        time_axis: crate::domain::shared::time_selection::TimeAxis::new(0),
                    },
                    Some(10),
                    &deadline,
                )
                .unwrap();
                let image = frames.render(0);
                let layout = build_video_animation_layout(compute_time_range(), OutputFormat::Gif);
                let sizing = taiko_constants::gif::sizing();
                let width = sizing.PAGE_MARGIN_LEFT
                    + sizing.PAGE_MARGIN_RIGHT
                    + columns as i64 * layout.image_width
                    + (columns as i64 - 1) * sizing.GRID_GAP;
                let height = sizing.PAGE_MARGIN_TOP
                    + sizing.PAGE_MARGIN_BOTTOM
                    + rows as i64 * (layout.image_height + sizing.INFO_MARGIN_BOTTOM)
                    + (rows as i64 - 1) * sizing.GRID_GAP;
                assert_eq!((image.w as i64, image.h as i64), (width, height));
                assert_eq!((width, height), standard_canvas);
                let hit_objects = taiko_hit_objects(&beatmap);
                let objects = prepare_hit_objects(
                    &hit_objects,
                    &MultiplierLookup {
                        points: build_multiplier_points(&beatmap.timing_points, 1.4),
                    },
                    &beatmap.timing_points,
                    1.0,
                );
                let autoplay = super::super::autoplay::Autoplay::new(&objects, &layout);
                for (index, time) in times.into_iter().enumerate() {
                    let (x, y) = gif_frame_origin(index, &layout);
                    assert_eq!((x, y), standard_origins[index]);
                    let mut expected = Img::new(
                        layout.image_width as u32,
                        layout.image_height as u32,
                        taiko_constants::gif::style::IMAGE_BACKGROUND,
                    );
                    draw_row_background(&mut expected, &layout, 0);
                    draw_hit_objects(
                        &mut expected,
                        &objects,
                        &[],
                        &layout,
                        0,
                        time,
                        &mut RenderCache::default(),
                    );
                    autoplay.draw(&mut expected, &layout, 0, time, &mut RenderCache::default());
                    let actual = image.crop(
                        x as u32,
                        y as u32,
                        (x + layout.image_width) as u32,
                        (y + layout.image_height) as u32,
                    );
                    assert_eq!(
                        actual.data, expected.data,
                        "第 {index} 格必须保留独立画布与选段时间"
                    );
                    let mut expected_label = Img::new(
                        layout.image_width as u32,
                        standard_sizing.INFO_MARGIN_BOTTOM as u32,
                        taiko_constants::gif::style::IMAGE_BACKGROUND,
                    );
                    let label = format!(
                        "{} - {}",
                        crate::render::text::format_mmssmmm(time),
                        crate::render::text::format_mmssmmm(time + 100),
                    );
                    let color = if time == 1000 {
                        taiko_constants::gif::style::PREVIEW_TIME_LABEL_COLOR
                    } else {
                        taiko_constants::gif::style::TIME_LABEL_COLOR
                    };
                    crate::render::text::draw_time_label(
                        &mut expected_label,
                        &label,
                        0,
                        standard_sizing.TIME_LABEL_TOP_GAP,
                        (time == 1000).then_some("Preview Time"),
                        layout.image_width,
                        standard_sizing.TIME_LABEL_FONT_SIZE,
                        standard_sizing.TIME_LABEL_NOTE_FONT_SIZE,
                        standard_sizing.TIME_LABEL_NOTE_TOP_GAP,
                        color,
                        color,
                    );
                    let actual_label = image.crop(
                        x as u32,
                        (y + layout.image_height) as u32,
                        (x + layout.image_width) as u32,
                        (y + layout.image_height + sizing.INFO_MARGIN_BOTTOM) as u32,
                    );
                    assert_eq!(
                        actual_label.data, expected_label.data,
                        "时间标签的字号、间距和格式必须与 STD 相同"
                    );
                }
            });
        }
    }

    fn swell_test_layout(scale: f64) -> AnimationLayout {
        let mut layout = AnimationLayout {
            segment_width: 100,
            row_height: 100,
            left_panel_width: 100,
            right_panel_width: 300,
            image_width: 400,
            image_height: 300,
            normal_note_diameter: 48,
            big_note_diameter: 74,
            time_range: 1000.0,
            playfield_left: 0,
            first_row_top: 80,
            row_stride: 300,
            judgement_line_color: [255, 255, 255, 255],
            output_format: crate::render::geometry::OutputFormat::Gif,
            render_scale: scale,
            content: crate::render::geometry::PixelRect {
                x: 0,
                y: 0,
                width: 400,
                height: 100,
            },
            track_background_color: [55, 55, 55, 255],
            track_edge_color: [80, 80, 80, 255],
            track_accent_color: [254, 59, 1, 255],
        };
        layout.segment_width = pyround(layout.segment_width as f64 * scale);
        layout.row_height = pyround(layout.row_height as f64 * scale);
        layout.left_panel_width = pyround(layout.left_panel_width as f64 * scale);
        layout.right_panel_width = pyround(layout.right_panel_width as f64 * scale);
        layout.image_width = pyround(layout.image_width as f64 * scale);
        layout.image_height = pyround(layout.image_height as f64 * scale);
        layout.first_row_top = pyround(layout.first_row_top as f64 * scale);
        layout.row_stride = layout.image_height;
        layout.normal_note_diameter = pyround(layout.normal_note_diameter as f64 * scale);
        layout.big_note_diameter = pyround(layout.big_note_diameter as f64 * scale);
        layout.content.width = layout.image_width;
        layout.content.height = layout.row_height;
        layout.content.y = layout.first_row_top;
        layout
    }

    fn render_test_swell(snapshot_time: i64) -> Img {
        let layout = swell_test_layout(1.0);
        let lookup = MultiplierLookup {
            points: build_multiplier_points(&[], 1.0),
        };
        let objects = prepare_hit_objects(
            &[TaikoHitObject {
                start_time: 1000,
                end_time: 3000,
                hit_type: SWELL_FLAG,
                hitsound: 0,
                samples: Vec::new(),
            }],
            &lookup,
            &[],
            1.0,
        );
        let mut image = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            [0, 0, 0, 0],
        );
        draw_hit_objects(
            &mut image,
            &objects,
            &[],
            &layout,
            0,
            snapshot_time,
            &mut RenderCache::default(),
        );
        image
    }

    #[test]
    fn swell_animation_expands_beyond_the_track_within_each_frame() {
        for scale in [0.5, 1.0, 2.0] {
            let layout = swell_test_layout(scale);
            let objects = prepare_hit_objects(
                &[TaikoHitObject {
                    start_time: 1000,
                    end_time: 3000,
                    hit_type: SWELL_FLAG,
                    hitsound: 0,
                    samples: Vec::new(),
                }],
                &MultiplierLookup {
                    points: build_multiplier_points(&[], 1.0),
                },
                &[],
                1.0,
            );
            let mut cache = RenderCache::default();
            for time in (900..2500).step_by(25) {
                let mut image = Img::new(
                    layout.image_width as u32,
                    layout.image_height as u32,
                    [0, 0, 0, 0],
                );
                draw_hit_objects(&mut image, &objects, &[], &layout, 0, time, &mut cache);
                if let Some(bounds) = image.alpha_bbox() {
                    assert!(bounds.1 > 0 && bounds.3 < layout.image_height as u32);
                    if time == 1200 {
                        assert!(
                            bounds.3 > (layout.first_row_top + layout.row_height) as u32,
                            "转盘必须能展开到轨道外"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn swell_has_no_drum_roll_tail_and_expires_at_end_time() {
        let image = render_test_swell(900);
        let layout = swell_test_layout(1.0);
        let x = judgement_line_x(&layout) as u32;
        assert_eq!(
            image.get(x + 100, gif_row_center_y(0, &layout) as u32)[3],
            0,
            "转盘不能绘制成连打长条"
        );
        assert!(render_test_swell(3001)
            .data
            .chunks_exact(4)
            .all(|pixel| pixel[3] == 0));
    }

    #[test]
    fn judgement_target_is_a_hollow_circle() {
        let layout = swell_test_layout(1.0);
        let mut image = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            [0, 0, 0, 0],
        );
        draw_judgement_circle(&mut image, &layout, 0);
        let x = judgement_line_x(&layout) as u32;
        let y = gif_row_center_y(0, &layout) as u32;
        assert_eq!(image.get(x, y)[3], 0);
        assert_eq!(image.get(x, layout.first_row_top as u32 + 5)[3], 0);
        assert!(image.get(x + layout.normal_note_diameter as u32 / 2 - 1, y)[3] > 0);
    }

    #[test]
    fn swell_marker_only_appears_before_the_first_hit() {
        for format in [
            crate::render::geometry::OutputFormat::Gif,
            crate::render::geometry::OutputFormat::Mp4,
        ] {
            let mut layout = swell_test_layout(1.0);
            layout.output_format = format;
            let objects = prepare_hit_objects(
                &[TaikoHitObject {
                    start_time: 1000,
                    end_time: 3000,
                    hit_type: SWELL_FLAG,
                    hitsound: 0,
                    samples: Vec::new(),
                }],
                &MultiplierLookup {
                    points: build_multiplier_points(&[], 1.0),
                },
                &[],
                1.0,
            );
            let render = |time| {
                let mut image = Img::new(
                    layout.image_width as u32,
                    layout.image_height as u32,
                    [0, 0, 0, 0],
                );
                draw_hit_object(
                    &mut image,
                    &objects[0],
                    &layout,
                    0,
                    time,
                    &mut RenderCache::default(),
                );
                image
            };
            let waiting = render(900);
            // 中心标志为青绿色小环；击打后的数字和计时圈为白色。
            let marker_region = |image: &Img, time| {
                let (cx, cy, diameter) = swell_frame_geometry(&objects[0], &layout, 0, time);
                (0..image.h)
                    .flat_map(|y| (0..image.w).map(move |x| (x, y)))
                    .filter(|&(x, y)| {
                        (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)
                            < (diameter as f64 * 0.3).powi(2)
                    })
                    .map(|(x, y)| image.get(x, y))
                    .filter(|pixel| pixel[3] > 0 && pixel[..3] == SWELL_COLOR)
                    .count()
            };
            assert!(marker_region(&waiting, 900) > 0, "击打前必须显示中心标志");
            assert!(
                waiting
                    .data
                    .chunks_exact(4)
                    .filter(|p| p[3] > 0)
                    .all(|p| p[..3] == SWELL_COLOR),
                "击打前不能出现数字"
            );
            for time in [1000, 1100, 1200, 1800] {
                assert!(
                    marker_region(&render(time), time) == 0,
                    "开始击打后必须隐藏中心标志"
                );
            }
        }
    }

    #[test]
    fn swell_restores_remaining_count_inner_circle_and_clear() {
        let layout = swell_test_layout(1.0);
        let classic_scale = layout.normal_note_diameter as f64 / 128.0;
        let x = pyround(judgement_line_x(&layout) as f64 + 250.0 * classic_scale);
        let y = pyround(gif_row_center_y(0, &layout) as f64 + 100.0 * classic_scale);
        for (time, label, font) in [(1200, "11", 24), (1800, "CLEAR", 16)] {
            let image = render_test_swell(time);
            let (w, h) = text_size(label, font);
            let left = pyround(x as f64 - w as f64 / 2.0);
            let top = pyround(y as f64 - h as f64 / 2.0);
            assert!(
                (top..top + h as i64).any(|row| (left..left + w as i64)
                    .any(|col| image.get(col as u32, row as u32)[..3] == NOTE_RING_COLOR[..3])),
                "转盘必须显示计数或完成文字"
            );
        }
    }

    #[test]
    fn swell_animation_scales_and_ignores_end_scroll_speed() {
        let lookup = MultiplierLookup {
            points: vec![
                MultiplierPoint {
                    time: 0.0,
                    multiplier: 1.0,
                },
                MultiplierPoint {
                    time: 2500.0,
                    multiplier: 0.01,
                },
            ],
        };
        let objects = prepare_hit_objects(
            &[TaikoHitObject {
                start_time: 1000,
                end_time: 3000,
                hit_type: SWELL_FLAG,
                hitsound: 0,
                samples: Vec::new(),
            }],
            &lookup,
            &[],
            1.0,
        );
        for scale in [0.5, 1.0, 2.0] {
            let layout = swell_test_layout(scale);
            let render = |time, cache: &mut RenderCache| {
                let mut image = Img::new(
                    layout.image_width as u32,
                    layout.image_height as u32,
                    [0, 0, 0, 0],
                );
                draw_hit_objects(&mut image, &objects, &[], &layout, 0, time, cache);
                image
            };
            let mut cache = RenderCache::default();
            let first = render(1200, &mut cache);
            let next = render(1050, &mut cache);
            assert_ne!(first.data, next.data, "转盘圆盘和计时圈必须随时间变化");
            assert!(
                render(1400, &mut cache)
                    .data
                    .chunks_exact(4)
                    .any(|pixel| pixel[3] > 0),
                "末端 SV 不能提前隐藏活动转盘"
            );
        }
    }

    #[test]
    fn hidden_spans_fade_and_disappear_as_a_whole() {
        let layout =
            build_video_animation_layout(1000.0, crate::render::geometry::OutputFormat::Gif);
        for hit_type in [DRUMROLL_FLAG, SWELL_FLAG] {
            let mut object = PreparedTaikoHitObject {
                hit_object: TaikoHitObject {
                    start_time: 2000,
                    end_time: 2200,
                    hit_type,
                    hitsound: 0,
                    samples: Vec::new(),
                },
                start_multiplier: 1.0,
                end_multiplier: 1.0,
                min_multiplier: 1.0,
                max_multiplier: 1.0,
                drum_roll_ticks: vec![PreparedAnimationPoint {
                    time: 2100.0,
                    multiplier: 1.0,
                }],
                hidden: true,
                swell_required_hits: 1,
            };
            let render = |object: &PreparedTaikoHitObject, time| {
                let mut image = Img::new(
                    layout.image_width as u32,
                    layout.image_height as u32,
                    [0, 0, 0, 0],
                );
                draw_hit_object(
                    &mut image,
                    object,
                    &layout,
                    0,
                    time,
                    &mut RenderCache::default(),
                );
                image
            };
            assert!(render(&object, 1375).alpha_bbox().is_none());
            let faded = render(&object, 1188);
            assert!(faded.alpha_bbox().is_some());
            assert!(faded.data.chunks_exact(4).all(|pixel| pixel[3] <= 128));
            object.hidden = false;
            let normal = render(&object, 1375);
            assert!(normal.data.chunks_exact(4).any(|pixel| pixel[3] == 255));
        }
    }

    #[test]
    fn hidden_notes_fade_with_preempt_and_scroll_velocity() {
        for multiplier in [0.5, 1.0, 2.0] {
            let preempt = 1000.0 / multiplier;
            for (remaining, expected) in [(1.0, 1.0), (0.8125, 0.5), (0.625, 0.0), (0.0, 0.0)] {
                let snapshot = pyround(4000.0 - preempt * remaining);
                assert!(
                    (hidden_alpha(4000.0, snapshot, multiplier, 1000.0) - expected).abs() <= 0.002
                );
            }
        }
    }

    #[test]
    fn drum_roll_end_tick_is_drawn_above_tail() {
        let layout = AnimationLayout {
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
            render_scale: 1.0,
            content: crate::render::geometry::PixelRect {
                x: 0,
                y: 0,
                width: 216,
                height: 80,
            },
            track_background_color: [0, 0, 0, 255],
            track_edge_color: [255, 255, 255, 255],
            track_accent_color: [255, 0, 0, 255],
            judgement_line_color: [255, 255, 255, 255],
            output_format: crate::render::geometry::OutputFormat::Gif,
        };
        let hit_object = PreparedTaikoHitObject {
            hit_object: TaikoHitObject {
                start_time: 0,
                end_time: 1000,
                hit_type: DRUMROLL_FLAG,
                hitsound: 0,
                samples: Vec::new(),
            },
            start_multiplier: 1.0,
            end_multiplier: 1.0,
            min_multiplier: 1.0,
            max_multiplier: 1.0,
            drum_roll_ticks: vec![PreparedAnimationPoint {
                time: 1000.0,
                multiplier: 1.0,
            }],
            hidden: false,
            swell_required_hits: 0,
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
        let layout = AnimationLayout {
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
            render_scale: 1.0,
            content: crate::render::geometry::PixelRect {
                x: 0,
                y: 0,
                width: 216,
                height: 80,
            },
            track_background_color: [0, 0, 0, 255],
            track_edge_color: [255, 255, 255, 255],
            track_accent_color: [255, 0, 0, 255],
            judgement_line_color: [255, 255, 255, 255],
            output_format: crate::render::geometry::OutputFormat::Gif,
        };
        let hit_object = PreparedTaikoHitObject {
            hit_object: TaikoHitObject {
                start_time: 0,
                end_time: 1000,
                hit_type: DRUMROLL_FLAG,
                hitsound: HIT_SOUNDS_STRONG,
                samples: Vec::new(),
            },
            start_multiplier: 1.0,
            end_multiplier: 1.0,
            min_multiplier: 1.0,
            max_multiplier: 1.0,
            drum_roll_ticks: vec![PreparedAnimationPoint {
                time: 500.0,
                multiplier: 1.0,
            }],
            hidden: false,
            swell_required_hits: 0,
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

    #[test]
    fn video_layout_matches_game_track_position_without_resizing_notes() {
        for (scale, expected_top) in [(0.75, 81), (1.0, 109), (1.25, 135), (2.0, 216)] {
            let mut custom = crate::config::CoreConfig::default();
            custom.render.taiko.mp4.SCALE = scale;
            custom.render.taiko.gif.SCALE = 3.0;
            custom.render.taiko.png.SCALE = 4.0;
            crate::config::with_config(std::sync::Arc::new(custom), || {
                let time_range = compute_time_range();
                let content = build_animation_layout_with_format(
                    time_range,
                    crate::render::geometry::OutputFormat::Mp4,
                );
                let video = build_video_animation_layout(
                    time_range,
                    crate::render::geometry::OutputFormat::Mp4,
                );
                let geometry = crate::render::geometry::taiko_geometry(
                    crate::render::geometry::OutputFormat::Mp4,
                );
                let canvas = crate::render::geometry::video_canvas(geometry.content);
                assert_eq!(
                    (video.image_width, video.image_height),
                    (canvas.width as i64, canvas.height as i64)
                );
                assert_eq!(video.content.x, canvas.origin_x);
                assert_eq!(video.content.y, expected_top);
                assert_eq!(video.content.width, geometry.content.width);
                assert_eq!(video.content.height, content.row_height);
                assert_eq!(video.playfield_left, canvas.origin_x);
                assert_eq!(video.first_row_top, expected_top);
                // 行高与音符直径都不随画布变化。
                assert_eq!(video.row_height, content.row_height);
                assert_eq!(video.normal_note_diameter, content.normal_note_diameter);
                assert_eq!(video.big_note_diameter, content.big_note_diameter);
            });
        }
    }

    #[test]
    fn flashlight_masks_each_grid_cell_without_affecting_labels() {
        let mut config = crate::config::CoreConfig::default();
        config.render.taiko.gif.SCALE = 0.75;
        config.render.taiko.gif.structure.ROW_COUNT = 2;

        config.render.taiko.gif.style.SHOW_MEASURE_LINES = false;
        crate::config::with_config(std::sync::Arc::new(config), || {
            let layout = build_video_animation_layout(
                compute_time_range(),
                crate::render::geometry::OutputFormat::Gif,
            );
            let right_bound =
                layout.playfield_left + layout.left_panel_width + layout.right_panel_width;
            let multiplier = 1.4 * MULTIPLIER_BASE_BEAT_LENGTH / 500.0;
            let note_time = 1000
                + pyround(
                    (right_bound - judgement_line_x(&layout) - 1) as f64 * layout.time_range
                        / (multiplier * layout.segment_width as f64),
                );
            let beatmap = crate::domain::parser::parse_beatmap_bytes(
                format!(
                    "osu file format v14\n\n[General]\nMode:1\n\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,{note_time},1,0,0:0:0:0:\n256,192,5000,1,0,0:0:0:0:\n"
                ).as_bytes(),
            ).unwrap();
            let deadline = RequestDeadline::new(
                std::time::Instant::now(),
                "gif",
                std::time::Duration::from_secs(60),
            );
            let prepare = |mods: Option<&ModSettings>| {
                prepare_taiko_gif_frames(
                    &beatmap,
                    mods,
                    GifRenderOptions::ResolvedSegments {
                        times_ms: vec![1000, 1020, 1040, 1060],
                        duration_seconds: Some(1.0),
                        time_axis: crate::domain::shared::time_selection::TimeAxis::new(0),
                    },
                    Some(20),
                    &deadline,
                )
                .unwrap()
                .render(0)
            };
            let normal = prepare(None);
            let mods = ModSettings {
                flashlight: true,
                ..ModSettings::new()
            };
            let masked = prepare(Some(&mods));
            for index in 0..4 {
                let (x, y) = gif_frame_origin(index, &layout);
                let sizing = taiko_constants::gif::sizing();
                let labels = |image: &Img| {
                    image.crop(
                        x as u32,
                        (y + layout.image_height) as u32,
                        (x + layout.image_width) as u32,
                        (y + layout.image_height + sizing.INFO_MARGIN_BOTTOM) as u32,
                    )
                };
                let normal_labels = labels(&normal);
                assert!(normal_labels
                    .data
                    .chunks_exact(4)
                    .any(|pixel| pixel[..3] != [0, 0, 0]));
                let masked_labels = labels(&masked);
                for (offset, (source, actual)) in normal_labels
                    .data
                    .chunks_exact(4)
                    .zip(masked_labels.data.chunks_exact(4))
                    .enumerate()
                {
                    assert_eq!(
                        source,
                        actual,
                        "格 {index}，标签像素 ({}, {})",
                        offset % normal_labels.w as usize,
                        offset / normal_labels.w as usize
                    );
                }
            }
            for index in 0..2 {
                let (origin_x, origin_y) = gif_frame_origin(index, &layout);
                let x = (origin_x + right_bound - 5) as u32;
                let y = (origin_y + gif_row_center_y(0, &layout)) as u32;
                assert!(normal.get(x, y)[0] > 50, "测试音符必须出现在轨道右端");
                assert_eq!(masked.get(x, y), [0, 0, 0, 255], "FL 必须覆盖当前格的音符");
            }
        });
    }

    #[test]
    fn rate_mods_change_clock_without_changing_note_spacing() {
        let beatmap = crate::domain::parser::parse_beatmap_bytes(
            b"osu file format v14\n\n[General]\nMode:1\n\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,2000,1,8,0:0:0:0:\n",
        )
        .unwrap();
        let mut config = crate::config::CoreConfig::default();
        config.render.taiko.gif.SCALE = 0.5;
        config.render.taiko.gif.structure.ROW_COUNT = 1;
        config.render.taiko.gif.structure.IMAGES_PER_ROW = 1;

        config.render.taiko.gif.style.SHOW_MEASURE_LINES = false;
        crate::config::with_config(std::sync::Arc::new(config), || {
            let deadline = RequestDeadline::new(
                std::time::Instant::now(),
                "gif",
                std::time::Duration::from_secs(60),
            );
            for base in ["", "EZ", "HR", "SW CS"] {
                let prepare = |tokens: &str| {
                    let mods = crate::domain::mods::parse_mods(
                        &tokens
                            .split_whitespace()
                            .map(str::to_owned)
                            .collect::<Vec<_>>(),
                    )
                    .unwrap();
                    prepare_taiko_gif_frames(
                        &beatmap,
                        Some(&mods),
                        GifRenderOptions::Segments {
                            times_ms: Some(vec![1_000]),
                            duration_seconds: Some(1.0),
                            time_axis: crate::domain::shared::time_selection::TimeAxis::new(0),
                        },
                        Some(20),
                        &deadline,
                    )
                    .unwrap()
                };
                let layout = build_video_animation_layout(
                    compute_time_range(),
                    crate::render::geometry::OutputFormat::Gif,
                );
                // 时间标签显示输出时间，倍速比较只检查相同谱面时间的轨道内容。
                let (origin_x, origin_y) = gif_frame_origin(0, &layout);
                let track = |image: Img| {
                    image
                        .crop(
                            origin_x as u32,
                            (origin_y + layout.first_row_top) as u32,
                            (origin_x + layout.image_width) as u32,
                            (origin_y + layout.first_row_top + layout.row_height) as u32,
                        )
                        .data
                };
                let normal = prepare(base);
                for (rate, frame_index, normal_index) in [
                    ("DT", 4, 6),
                    ("HT", 8, 6),
                    ("DT1.25", 8, 10),
                    ("HT0.5", 16, 8),
                ] {
                    let changed = prepare(&format!("{base} {rate}"));
                    assert_eq!(changed.frame_count(), 20);
                    assert!(
                        track(changed.render(frame_index)) == track(normal.render(normal_index)),
                        "{base} {rate} 在相同谱面时间不能额外改变音符间距"
                    );
                }
            }
        });
    }
    #[test]
    fn drum_roll_keeps_complete_head_until_end_and_only_shortens_tail() {
        for scale in [0.5, 1.0, 2.0] {
            let layout = swell_test_layout(scale);
            let lookup = MultiplierLookup {
                points: build_multiplier_points(&[], 1.0),
            };
            for strong in [false, true] {
                let objects = prepare_hit_objects(
                    &[TaikoHitObject {
                        start_time: 1000,
                        end_time: 3000,
                        hit_type: DRUMROLL_FLAG,
                        hitsound: if strong { HIT_SOUNDS_STRONG } else { 0 },
                        samples: Vec::new(),
                    }],
                    &lookup,
                    &[],
                    1.0,
                );
                let diameter = if strong {
                    layout.big_note_diameter
                } else {
                    layout.normal_note_diameter
                };
                let x = judgement_line_x(&layout);
                let y = gif_row_center_y(0, &layout);
                let mut cache = RenderCache::default();
                let mut last_right = u32::MAX;
                for time in [1000, 2200, 2500, 2999] {
                    let mut image = Img::new(
                        layout.image_width as u32,
                        layout.image_height as u32,
                        [0, 0, 0, 0],
                    );
                    draw_hit_objects(&mut image, &objects, &[], &layout, 0, time, &mut cache);
                    let expected = super::super::notes::build_note_disc(
                        ROLL_COLOR,
                        diameter,
                        crate::render::geometry::scale_stroke_px(1.0, layout.render_scale),
                        false,
                    );
                    let left = pyround(x as f64 - diameter as f64 / 2.0);
                    let top = pyround(y as f64 - diameter as f64 / 2.0);
                    for row in 0..diameter {
                        for col in 0..diameter / 2 {
                            assert_eq!(
                                image.get((left + col) as u32, (top + row) as u32),
                                expected.get(col as u32, row as u32),
                                "判定点左半圆必须完整保留"
                            );
                        }
                    }
                    let right = image.alpha_bbox().unwrap().2;
                    assert!(right <= last_right, "尾巴只能缩短");
                    last_right = right;
                }
                for time in [3000, 3001] {
                    let mut image = Img::new(
                        layout.image_width as u32,
                        layout.image_height as u32,
                        [0, 0, 0, 0],
                    );
                    draw_hit_objects(&mut image, &objects, &[], &layout, 0, time, &mut cache);
                    assert!(image.alpha_bbox().is_none(), "完成后整条滚奏必须消失");
                }
            }
        }
    }
}
