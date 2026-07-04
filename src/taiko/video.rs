//! osu!taiko MP4 renderer: full-chart continuous playback (no 4-row segment
//! preview). Reuses the GIF row background + hit-object drawing on a one-row
//! layout; the per-segment bottom label is dropped (the global top-right label
//! is drawn by `video::save_mp4_streamed`).
//!
//! Time range: first note − 2s → last note + 2s, or `[t1, t2]` when
//! `--time=t1+t2` is given. 15 fps, letterboxed to 16:9.

use crate::canvas::Img;
use crate::errors::{PreviewError, Result};
use crate::models::Beatmap;
use crate::mods::ModSettings;
use crate::video::save_mp4_streamed;
use std::cell::RefCell;
use std::path::Path;

use super::constants::*;
use super::gif::{
    build_gif_layout, build_multiplier_points, compute_time_range, draw_hit_objects,
    draw_row_background, prepare_hit_objects, pyround, GifLayout, MultiplierLookup,
};
use super::notes::RenderCache;
use super::timing::*;

pub(crate) fn render_taiko_video(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    times_ms: Option<Vec<i64>>,
    output_path: &Path,
) -> Result<()> {
    let hit_objects = apply_taiko_object_mods(taiko_hit_objects(beatmap), mods);
    if hit_objects.is_empty() {
        return Err(PreviewError::render("taiko beatmap has no hit objects"));
    }

    let (start, end) = match &times_ms {
        None => {
            let first = hit_objects.iter().map(|h| h.start_time).min().unwrap_or(0);
            let last = hit_objects.iter().map(|h| h.end_time).max().unwrap_or(0);
            (first - 2000, last + 2000)
        }
        Some(t) if t.len() == 2 => (t[0], t[1]),
        Some(_) => {
            return Err(PreviewError::new(
                "--time for mp4 needs exactly 2 values t1+t2 (or omit for the full chart)",
            ));
        }
    };
    if end <= start {
        return Err(PreviewError::new("mp4 time range is empty"));
    }

    let speed = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let total_ms = end - start;
    let fps = GIF_FPS as u32;
    let frame_count = ((total_ms as f64 * fps as f64 / (1000.0 * speed)).round() as usize).max(1);

    let slider_multiplier = effective_slider_multiplier(beatmap, mods)?;
    let timing_points = effective_timing_points(beatmap, mods);
    let multiplier_lookup = MultiplierLookup {
        points: build_multiplier_points(&timing_points, slider_multiplier),
    };
    let prepared_hit_objects = prepare_hit_objects(&hit_objects, &multiplier_lookup);
    let time_range = compute_time_range() / speed;
    let layout = build_video_layout(time_range);

    let static_bg = {
        let mut bg = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            IMAGE_BACKGROUND,
        );
        draw_row_background(&mut bg, &layout, 0);
        bg
    };

    // Per-thread render cache avoids serialising parallel draw_hit_objects
    // calls behind a single Mutex (video has far more frames than GIF).
    thread_local! {
        static TAIKO_VIDEO_CACHE: RefCell<RenderCache> = RefCell::new(RenderCache::default());
    }

    let render = move |frame_index: usize| -> (Img, i64) {
        let snapshot_time = start + pyround(frame_index as f64 * 1000.0 * speed / fps as f64);
        let mut canvas = static_bg.clone();
        TAIKO_VIDEO_CACHE.with(|cache| {
            draw_hit_objects(
                &mut canvas,
                &prepared_hit_objects,
                &layout,
                0,
                snapshot_time,
                &mut *cache.borrow_mut(),
            );
        });
        (canvas, snapshot_time)
    };

    save_mp4_streamed(frame_count, end, render, output_path, fps)
}

/// Single-row layout for MP4: same width as the GIF, height trimmed to one row
/// (no 4-row stack, no inter-row gap, no bottom label strip).
fn build_video_layout(time_range: f64) -> GifLayout {
    let mut layout = build_gif_layout(time_range);
    layout.image_height = PAGE_MARGIN_Y * 2 + GIF_ROW_HEIGHT;
    layout
}
