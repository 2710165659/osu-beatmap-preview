//! osu!catch MP4 renderer: full-chart continuous playback (no 2×2 segment
//! preview). Reuses `render_gif_frame` from the GIF path so per-frame pixels
//! match the GIF's single-segment look; only the time axis and framing differ.
//!
//! Time range: first note − 2s → last note + 2s, or `[t1, t2]` when
//! `--time=t1+t2` is given. 15 fps, letterboxed to 16:9 by `video::save_mp4_streamed`.

use crate::canvas::Img;
use crate::errors::{PreviewError, Result};
use crate::models::Beatmap;
use crate::mods::ModSettings;
use crate::video::save_mp4_streamed;
use std::path::Path;

use super::constants::GIF_FPS;
use super::gif::{build_gif_layout, render_gif_frame};
use super::objects::{build_catch_render_objects, effective_difficulty};
use super::png::rhe;

pub(crate) fn render_catch_video(
    beatmap: &Beatmap, mods: Option<&ModSettings>, times_ms: Option<Vec<i64>>,
    output_path: &Path,
) -> Result<()> {
    let hit_objects = match beatmap.hit_objects.as_catch() {
        Some(v) if !v.is_empty() => v,
        _ => return Err(PreviewError::render("catch beatmap has no hit objects")),
    };
    let difficulty = effective_difficulty(beatmap, mods);
    let mut render_objects = build_catch_render_objects(beatmap, hit_objects, mods, &difficulty)?;

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

    let layout = build_gif_layout(difficulty.cs, difficulty.ar);
    render_objects.sort_by_key(|o| std::cmp::Reverse(o.start_time));
    let start_times: Vec<i64> = render_objects.iter().map(|o| o.start_time).collect();

    let render = move |frame_index: usize| -> (Img, i64) {
        let snapshot_time = start + rhe(frame_index as f64 * 1000.0 * speed / fps as f64);
        let frame = render_gif_frame(&render_objects, &start_times, snapshot_time, &layout);
        (frame, snapshot_time)
    };

    save_mp4_streamed(frame_count, end, render, output_path, fps)
}
