//! osu!standard MP4 renderer: full-chart continuous playback (no 2×2 segment
//! preview). Reuses `render_frame` from the GIF path so per-frame pixels match
//! the GIF's single-segment look; only the time axis and framing differ.
//!
//! Time range: first note − 2s → last note + 2s, or `[t1, t2]` when
//! `--time=t1+t2` is given. 15 fps, letterboxed to 16:9 by `video::save_mp4_streamed`.

use crate::canvas::Img;
use crate::errors::{PreviewError, Result};
use crate::models::Beatmap;
use crate::mods::ModSettings;
use crate::parser::round_half_even;
use crate::video::save_mp4_streamed;
use std::cell::RefCell;
use std::path::Path;

use super::constants::GIF_FPS;
use super::context::{
    apply_standard_object_mods, build_render_context, build_visible_indexes_by_snapshot,
    standard_objects, RenderCache,
};
use super::objects::render_frame;

/// Padding around the first/last note when rendering the full chart.
const PAD_MS: i64 = 2000;

pub(crate) fn render_standard_video(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    times_ms: Option<Vec<i64>>,
    output_path: &Path,
) -> Result<()> {
    let hit_objects = standard_objects(beatmap)?;
    // Resolve the rendered time span: full chart (±2s) or explicit [t1, t2].
    let (start, end) = match &times_ms {
        None => {
            let first = hit_objects.iter().map(|o| o.start_time).min().unwrap_or(0);
            let last = hit_objects.iter().map(|o| o.end_time).max().unwrap_or(0);
            (first - PAD_MS, last + PAD_MS)
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

    let hit_objects = apply_standard_object_mods(hit_objects, mods);
    let context = build_render_context(beatmap, hit_objects, mods);
    let speed = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let total_ms = end - start;
    let fps = GIF_FPS as u32;
    let frame_count =
        ((total_ms as f64 * fps as f64 / (1000.0 * speed)).round() as usize).max(1);

    let break_periods = beatmap.break_periods.clone();
    let context_ref = &context;
    let break_ref = &break_periods;

    // Per-thread render cache avoids serialising parallel render_frame calls
    // behind a single Mutex (video has far more frames than GIF).
    thread_local! {
        static STD_VIDEO_CACHE: RefCell<RenderCache> = RefCell::new(RenderCache::default());
    }

    let render = move |frame_index: usize| -> (Img, i64) {
        let snapshot_time =
            start + round_half_even(frame_index as f64 * 1000.0 * speed / fps as f64);
        let groups = build_visible_indexes_by_snapshot(
            &context_ref.hit_objects,
            &[snapshot_time],
            context_ref.settings.preempt_ms,
        );
        let frame = STD_VIDEO_CACHE.with(|cache| {
            render_frame(
                context_ref,
                &mut *cache.borrow_mut(),
                snapshot_time,
                break_ref,
                &groups[0],
            )
        });
        (frame, snapshot_time)
    };

    save_mp4_streamed(frame_count, end, render, output_path, fps)
}
