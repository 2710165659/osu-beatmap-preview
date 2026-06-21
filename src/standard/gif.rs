//! osu!standard GIF renderer: 2×2 segment animated preview.

use crate::canvas::Img;
use crate::composer::save_animated_gif_streamed;
use crate::errors::{PreviewError, Result};
use crate::models::Beatmap;
use crate::mods::ModSettings;
use crate::text::format_mmssmmm;
use std::path::Path;
use std::sync::Mutex;

use super::constants::*;
use super::context::*;
use super::objects::render_frame;
use super::draw_time_label;

pub(crate) fn render_standard_gif(
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
                    rt.start_time
                        + py_round(fi as f64 * 1000.0 * speed_multiplier / GIF_FPS as f64)
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

    let cache = Mutex::new(RenderCache::default());
    let render = move |frame_index: usize| -> Img {
        let mut canvas = Img::new(canvas_w as u32, canvas_h as u32, CANVAS_BACKGROUND_COLOR);
        for (segment_index, row_timing) in row_timings.iter().enumerate() {
            let (x, y) = gif_frame_origin(segment_index);
            let snapshot_time = segment_snapshot_times[segment_index][frame_index];
            let frame = render_frame(
                &context,
                &mut *cache.lock().unwrap(),
                snapshot_time,
                &row_timing.break_periods,
                &segment_visible_indexes[segment_index][frame_index],
            );
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
