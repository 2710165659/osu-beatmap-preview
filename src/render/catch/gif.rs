//! osu!catch GIF 渲染器：2×2 分段动画预览或单屏区间预览。
//!
//! 单帧 683×384（16:9），playfield 的位置与缩放按游戏内 1080p 等比换算
//! （见 `config::layout::catch::gif` 中的 playfield 配置），上下左右留白与游戏一致。

use crate::common::time_selection::{GifRenderOptions, PreviewTimeSelector, TimeAxis};
use crate::config::layout::catch::gif::*;
use crate::core::errors::{PreviewError, Result};
use crate::core::models::Beatmap;
use crate::core::mods::ModSettings;
use crate::render::canvas::Img;
use crate::render::composer::save_animated_gif_streamed;
use crate::render::text::{draw_text, text_size};
use std::path::Path;

use super::drawing::{draw_catch_object, object_diameter};
use super::objects::{build_catch_render_objects, effective_difficulty, RenderObject};
use super::png::rhe;

// ─── GIF 布局 ───

pub(crate) struct GifLayout {
    canvas_width: i64,
    canvas_height: i64,
    /// playfield（512 宽坐标系）在帧内的缩放。
    playfield_scale: f64,
    /// playfield 左边缘在帧内的 x 坐标。
    playfield_left: f64,
    /// playfield 顶部在帧内的 y 坐标。
    playfield_top: f64,
    object_scale: f64,
    pixels_per_ms: f64,
}

pub(crate) fn build_gif_layout(circle_size: f64, approach_rate: f64) -> GifLayout {
    let playfield_scale = PLAYFIELD_SCALE;
    let playfield_left = (IMAGE_WIDTH as f64 - PLAYFIELD_WIDTH * playfield_scale) / 2.0;
    let playfield_top = PLAYFIELD_TOP;
    let object_scale = super::objects::circle_scale(circle_size);

    // 下落速度：AR 时间窗对应「起始高度→接手」的可视下落距离
    let time_range = super::objects::catch_time_range(approach_rate);
    let visible_fall_height = (STABLE_CATCHER_Y - STABLE_FRUIT_START_Y) * playfield_scale;
    let pixels_per_ms = visible_fall_height / time_range;

    let row_height = IMAGE_HEIGHT
        + if SHOW_TIME_LABEL {
            TIME_LABEL_TOP_GAP + TIME_LABEL_HEIGHT
        } else {
            0
        };
    let canvas_width =
        PAGE_MARGIN_X * 2 + IMAGES_PER_ROW * IMAGE_WIDTH + (IMAGES_PER_ROW - 1) * GRID_GAP;
    let canvas_height = PAGE_MARGIN_Y * 2 + ROW_COUNT * row_height + (ROW_COUNT - 1) * GRID_GAP;

    GifLayout {
        canvas_width,
        canvas_height,
        playfield_scale,
        playfield_left,
        playfield_top,
        object_scale,
        pixels_per_ms,
    }
}

/// 第 segment_index 段在画布上的左上角。
fn frame_origin(segment_index: usize) -> (i64, i64) {
    let row_index = segment_index as i64 / IMAGES_PER_ROW;
    let col_index = segment_index as i64 % IMAGES_PER_ROW;
    let row_height = IMAGE_HEIGHT
        + if SHOW_TIME_LABEL {
            TIME_LABEL_TOP_GAP + TIME_LABEL_HEIGHT
        } else {
            0
        };
    let x = PAGE_MARGIN_X + col_index * (IMAGE_WIDTH + GRID_GAP);
    let y = PAGE_MARGIN_Y + row_index * (row_height + GRID_GAP);
    (x, y)
}

// ─── 对外接口 ───

pub(crate) fn render_catch_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    options: GifRenderOptions,
    output_path: &Path,
) -> Result<()> {
    match options {
        GifRenderOptions::Segments {
            times_ms,
            time_axis,
        } => render_catch_segment_gif(beatmap, mods, times_ms, time_axis, output_path),
    }
}

fn render_catch_segment_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    times_ms: Option<Vec<i64>>,
    time_axis: TimeAxis,
    output_path: &Path,
) -> Result<()> {
    let hit_objects = match beatmap.hit_objects.as_catch() {
        Some(v) if !v.is_empty() => v,
        _ => return Err(PreviewError::render("catch beatmap has no hit objects")),
    };

    let difficulty = effective_difficulty(beatmap, mods);
    let mut render_objects = build_catch_render_objects(beatmap, hit_objects, mods, &difficulty)?;

    let speed_multiplier = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let gameplay_segment_duration = rhe(DURATION_MS * speed_multiplier);
    let spans: Vec<(i64, i64)> = hit_objects
        .iter()
        .map(|h| (h.start_time, h.end_time))
        .collect();
    let segment_timings = PreviewTimeSelector::new(
        beatmap,
        spans,
        (ROW_COUNT * IMAGES_PER_ROW) as usize,
        gameplay_segment_duration,
        times_ms,
    )?
    .choose()?;

    let layout = build_gif_layout(difficulty.cs, difficulty.ar);
    let frame_count = rhe(DURATION_MS * FPS / 1000.0).max(1) as usize;
    let frame_duration_ms = rhe(1000.0 / FPS).max(1) as u32;

    let segment_snapshot_times: Vec<Vec<i64>> = segment_timings
        .iter()
        .map(|timing| {
            (0..frame_count)
                .map(|frame_index| {
                    timing.start_time + rhe(frame_index as f64 * 1000.0 * speed_multiplier / FPS)
                })
                .collect()
        })
        .collect();

    // 按开始时间降序排序，先画晚出现的对象，后画早出现的（早的盖在上层）
    render_objects.sort_by_key(|o| std::cmp::Reverse(o.start_time));
    // 各段按时间二分裁剪可见窗口，避免每帧全量遍历
    let start_times: Vec<i64> = render_objects.iter().map(|o| o.start_time).collect();

    let render = move |frame_index: usize| -> Img {
        let mut canvas = Img::new(
            layout.canvas_width as u32,
            layout.canvas_height as u32,
            PLAYFIELD_BACKGROUND,
        );
        for (segment_index, segment_timing) in segment_timings.iter().enumerate() {
            let snapshot_time = segment_snapshot_times[segment_index][frame_index];
            let (frame_x, frame_y) = frame_origin(segment_index);
            let frame = render_gif_frame(&render_objects, &start_times, snapshot_time, &layout);
            canvas.alpha_composite(&frame, frame_x, frame_y);
            if SHOW_TIME_LABEL {
                draw_gif_time_label(
                    &mut canvas,
                    segment_timing.start_time,
                    gameplay_segment_duration,
                    frame_x,
                    frame_y,
                    segment_timing.is_preview,
                    time_axis,
                );
            }
        }
        canvas
    };

    save_animated_gif_streamed(frame_count, render, output_path, frame_duration_ms)
}

/// 渲染单段单帧：背景 + 判定线 + 接手 + 可见的下落对象。
pub(crate) fn render_gif_frame(
    render_objects: &[RenderObject],
    start_times_desc: &[i64],
    snapshot_time: i64,
    layout: &GifLayout,
) -> Img {
    let mut frame = Img::new(
        IMAGE_WIDTH as u32,
        IMAGE_HEIGHT as u32,
        PLAYFIELD_BACKGROUND,
    );

    let playfield_left = layout.playfield_left;
    let playfield_right = playfield_left + PLAYFIELD_WIDTH * layout.playfield_scale;
    // playfield 区域底色
    frame.set_rect(
        rhe(playfield_left),
        0,
        rhe(playfield_right),
        IMAGE_HEIGHT,
        PLAYFIELD_BACKGROUND,
    );

    // 判定线（接手所在高度）
    let judgement_y = layout.playfield_top + STABLE_CATCHER_Y * layout.playfield_scale;
    let judgement_y_px = rhe(judgement_y);
    frame.set_rect(
        rhe(playfield_left),
        judgement_y_px,
        rhe(playfield_right),
        judgement_y_px + 1,
        [238, 238, 238, 200],
    );

    // 可见时间窗：对象在 [snapshot, snapshot + 下落时间窗 + 余量] 内才可能出现在帧中
    let fall_window_ms = (IMAGE_HEIGHT as f64 / layout.pixels_per_ms).ceil() as i64 + 2000;
    // start_times_desc 为降序；找到可见区间 [lo, hi)
    let lo = start_times_desc.partition_point(|&t| t > snapshot_time + fall_window_ms);
    let hi = start_times_desc.partition_point(|&t| t >= snapshot_time - 2000);

    for catch_object in &render_objects[lo..hi] {
        draw_gif_object(&mut frame, catch_object, snapshot_time, judgement_y, layout);
    }

    frame
}

/// 绘制单个下落中的对象（超出帧范围的直接跳过）。
fn draw_gif_object(
    frame: &mut Img,
    catch_object: &RenderObject,
    snapshot_time: i64,
    judgement_y: f64,
    layout: &GifLayout,
) {
    let local_time = catch_object.start_time - snapshot_time;
    let center_x = layout.playfield_left + catch_object.x * layout.playfield_scale;
    let center_y = judgement_y - local_time as f64 * layout.pixels_per_ms;
    let diameter = object_diameter(
        layout.object_scale,
        layout.playfield_scale,
        catch_object.scale_factor,
    );

    if center_y + diameter / 2.0 < 0.0 || center_y - diameter / 2.0 > judgement_y {
        return;
    }

    draw_catch_object(frame, catch_object, center_x, center_y, diameter);
}

fn draw_gif_time_label(
    canvas: &mut Img,
    start_time: i64,
    duration_ms: i64,
    frame_x: i64,
    frame_y: i64,
    is_preview: bool,
    time_axis: TimeAxis,
) {
    let label = format!(
        "{} - {}",
        crate::render::text::format_mmss_floor(time_axis.to_display(start_time)),
        crate::render::text::format_mmss_floor(time_axis.to_display(start_time + duration_ms))
    );
    let color = if is_preview {
        PREVIEW_TIME_LABEL_COLOR
    } else {
        TIME_LABEL_COLOR
    };
    let note_color = if is_preview {
        PREVIEW_TIME_LABEL_COLOR
    } else {
        TIME_LABEL_NOTE_COLOR
    };
    let (label_w, label_h) = text_size(&label, TIME_LABEL_FONT_SIZE);
    let x = frame_x + (IMAGE_WIDTH - label_w as i64) / 2;
    let y = frame_y + IMAGE_HEIGHT + TIME_LABEL_TOP_GAP;
    draw_text(canvas, x, y, &label, TIME_LABEL_FONT_SIZE, color);

    if is_preview {
        let note = "Preview Time";
        let (note_w, _) = text_size(note, TIME_LABEL_NOTE_FONT_SIZE);
        let note_x = frame_x + (IMAGE_WIDTH - note_w as i64) / 2;
        draw_text(
            canvas,
            note_x,
            y + label_h as i64 + TIME_LABEL_NOTE_TOP_GAP,
            note,
            TIME_LABEL_NOTE_FONT_SIZE,
            note_color,
        );
    }
}
