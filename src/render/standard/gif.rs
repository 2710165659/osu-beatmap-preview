//! osu!standard GIF renderer: 2×2 segment preview or single-screen clip.

use crate::common::time_selection::{GifClipRange, GifRenderOptions};
use crate::core::errors::{PreviewError, Result};
use crate::core::models::Beatmap;
use crate::core::mods::ModSettings;
use crate::render::canvas::Img;
use crate::render::composer::save_animated_gif_streamed;
use crate::render::text::format_mmssmmm;
use std::cell::RefCell;
use std::path::Path;

use super::constants::*;
use super::context::*;
use super::draw_time_label;
use super::objects::render_frame;

pub(crate) fn render_standard_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    options: GifRenderOptions,
    output_path: &Path,
) -> Result<()> {
    match options {
        GifRenderOptions::Segments(times_ms) => {
            render_standard_segment_gif(beatmap, mods, times_ms, output_path)
        }
        GifRenderOptions::Clip {
            range,
            show_time_label,
        } => render_standard_clip_gif(beatmap, mods, range, show_time_label, output_path),
    }
}

fn render_standard_segment_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    times_ms: Option<Vec<i64>>,
    output_path: &Path,
) -> Result<()> {
    if let Some(times) = &times_ms {
        if times.len() > GIF_ROW_COUNT * GIF_IMAGES_PER_ROW {
            return Err(PreviewError::new("--times accepts at most 4 time points"));
        }
    }

    let hit_objects = standard_objects(beatmap)?;
    let hit_objects = apply_standard_object_mods(hit_objects, mods);
    let context = build_render_context(beatmap, hit_objects, mods);
    let speed_multiplier = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let gameplay_segment_duration = py_round(GIF_DURATION_MS as f64 * speed_multiplier);
    let row_timings = choose_row_start_times(
        beatmap,
        &context.hit_objects,
        GIF_ROW_COUNT * GIF_IMAGES_PER_ROW,
        2,
        gameplay_segment_duration,
        times_ms,
    )?;

    let (canvas_w, canvas_h) = gif_canvas_size();
    let frame_count = (((GIF_DURATION_MS * GIF_FPS) as f64 / 1000.0).round() as usize).max(1);
    let frame_duration_ms = ((1000.0 / GIF_FPS as f64).round() as u32).max(1);

    let segment_snapshot_times: Vec<Vec<i64>> = row_timings
        .iter()
        .map(|rt| {
            (0..frame_count)
                .map(|fi| {
                    rt.start_time + py_round(fi as f64 * 1000.0 * speed_multiplier / GIF_FPS as f64)
                })
                .collect()
        })
        .collect();
    let segment_visible_indexes: Vec<Vec<Vec<usize>>> = segment_snapshot_times
        .iter()
        .map(|snapshot_times| {
            build_visible_indexes_by_snapshot(
                &context.hit_objects,
                snapshot_times,
                context.settings.preempt_ms,
            )
        })
        .collect();

    // Per-thread render cache avoids serialising parallel render_frame calls
    // behind a single Mutex — rayon's chunk-parallel render would otherwise queue
    // on the lock, halving throughput.  Each thread gets its own cache; the
    // first few frames rebuild procedural textures, then cache hits dominate.
    thread_local! {
        static STD_GIF_CACHE: RefCell<RenderCache> = RefCell::new(RenderCache::default());
    }

    let render = move |frame_index: usize| -> Img {
        let mut canvas = Img::new(canvas_w as u32, canvas_h as u32, CANVAS_BACKGROUND_COLOR);
        for (segment_index, row_timing) in row_timings.iter().enumerate() {
            let (x, y) = gif_frame_origin(segment_index);
            let snapshot_time = segment_snapshot_times[segment_index][frame_index];
            let frame = STD_GIF_CACHE.with(|cache| {
                render_frame(
                    &context,
                    &mut *cache.borrow_mut(),
                    snapshot_time,
                    &row_timing.break_periods,
                    &segment_visible_indexes[segment_index][frame_index],
                )
            });
            canvas.alpha_composite(&frame, x, y);
            let note = if row_timing.is_preview {
                Some("Preview Time")
            } else {
                None
            };
            let label = format!(
                "{} - {}",
                format_mmssmmm(row_timing.start_time),
                format_mmssmmm(row_timing.start_time + gameplay_segment_duration)
            );
            draw_time_label(
                &mut canvas,
                &label,
                x,
                y + IMAGE_HEIGHT + TIME_LABEL_TOP_GAP,
                note,
                if row_timing.is_preview {
                    PREVIEW_TIME_LABEL_COLOR
                } else {
                    TIME_LABEL_COLOR
                },
                if row_timing.is_preview {
                    PREVIEW_TIME_LABEL_COLOR
                } else {
                    TIME_LABEL_NOTE_COLOR
                },
            );
        }
        canvas
    };

    save_animated_gif_streamed(frame_count, render, output_path, frame_duration_ms)
}

fn render_standard_clip_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    range: GifClipRange,
    show_time_label: bool,
    output_path: &Path,
) -> Result<()> {
    let hit_objects = standard_objects(beatmap)?;
    let hit_objects = apply_standard_object_mods(hit_objects, mods);
    let context = build_render_context(beatmap, hit_objects, mods);
    let speed_multiplier = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let frame_count = (((range.end - range.start) as f64 * GIF_FPS as f64
        / (1000.0 * speed_multiplier))
        .round() as usize)
        .max(1);
    let frame_duration_ms = ((1000.0 / GIF_FPS as f64).round() as u32).max(1);
    let canvas_w = HORIZONTAL_PAGE_MARGIN * 2 + IMAGE_WIDTH;
    let label_height = if show_time_label {
        TIME_LABEL_TOP_GAP + TIME_LABEL_HEIGHT
    } else {
        0
    };
    let canvas_h = VERTICAL_PAGE_MARGIN * 2 + IMAGE_HEIGHT + label_height;

    let snapshot_times: Vec<i64> = (0..frame_count)
        .map(|fi| range.start + py_round(fi as f64 * 1000.0 * speed_multiplier / GIF_FPS as f64))
        .collect();
    let visible_indexes = build_visible_indexes_by_snapshot(
        &context.hit_objects,
        &snapshot_times,
        context.settings.preempt_ms,
    );
    let break_periods = range.break_periods.clone();

    thread_local! {
        static STD_GIF_CLIP_CACHE: RefCell<RenderCache> = RefCell::new(RenderCache::default());
    }

    let render = move |frame_index: usize| -> Img {
        let mut canvas = Img::new(canvas_w as u32, canvas_h as u32, CANVAS_BACKGROUND_COLOR);
        let x = HORIZONTAL_PAGE_MARGIN;
        let y = VERTICAL_PAGE_MARGIN;
        let frame = STD_GIF_CLIP_CACHE.with(|cache| {
            render_frame(
                &context,
                &mut *cache.borrow_mut(),
                snapshot_times[frame_index],
                &break_periods,
                &visible_indexes[frame_index],
            )
        });
        canvas.alpha_composite(&frame, x, y);
        if show_time_label {
            let label = format!(
                "{} - {}",
                format_mmssmmm(range.start),
                format_mmssmmm(range.end)
            );
            draw_time_label(
                &mut canvas,
                &label,
                x,
                y + IMAGE_HEIGHT + TIME_LABEL_TOP_GAP,
                range.is_preview.then_some("Preview Time"),
                if range.is_preview {
                    PREVIEW_TIME_LABEL_COLOR
                } else {
                    TIME_LABEL_COLOR
                },
                if range.is_preview {
                    PREVIEW_TIME_LABEL_COLOR
                } else {
                    TIME_LABEL_NOTE_COLOR
                },
            );
        }
        canvas
    };

    save_animated_gif_streamed(frame_count, render, output_path, frame_duration_ms)
}
