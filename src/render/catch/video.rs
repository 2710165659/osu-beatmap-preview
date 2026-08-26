//! osu!catch MP4 renderer: full-chart continuous playback (no 2×2 segment
//! preview). Reuses `render_gif_frame` from the GIF path so per-frame pixels
//! match the GIF's single-segment look; only the time axis and framing differ.
//!
//! Time range is controlled by `--time-points` and `--duration-time`.

use crate::common::time_selection::TimeAxis;
use crate::core::errors::{PreviewError, Result};
use crate::core::models::Beatmap;
use crate::core::mods::ModSettings;
use crate::core::validate::TimePoint;
use crate::render::canvas::Img;
use crate::render::video::audio::AudioSourceJob;
use crate::render::video::{resolve_video_time_range, save_mp4_streamed};
use std::path::Path;

use super::gif::{build_gif_layout, render_gif_frame};
use super::objects::{build_catch_render_objects, effective_difficulty};
use super::png::rhe;

pub(crate) fn render_catch_video(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    start_time: Option<TimePoint>,
    duration_time: Option<f64>,
    output_path: &Path,
    audio_job: AudioSourceJob,
    time_axis: TimeAxis,
) -> Result<()> {
    let hit_objects = match beatmap.hit_objects.as_catch() {
        Some(v) if !v.is_empty() => v,
        _ => return Err(PreviewError::render("catch beatmap has no hit objects")),
    };
    let difficulty = effective_difficulty(beatmap, mods);
    let mut render_objects = build_catch_render_objects(beatmap, hit_objects, mods, &difficulty)?;

    let speed = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let first = hit_objects.iter().map(|h| h.start_time).min().unwrap_or(0);
    let last = hit_objects.iter().map(|h| h.end_time).max().unwrap_or(0);
    let range = resolve_video_time_range(beatmap, first, last, start_time, duration_time, speed)?;
    let (start, end) = (range.start, range.end);
    let total_ms = end - start;
    let fps = crate::config::current().layout.catch.mp4.FPS as u32;
    let frame_count = ((total_ms as f64 * fps as f64 / (1000.0 * speed)).round() as usize).max(1);

    let layout = build_gif_layout(difficulty.cs, difficulty.ar);
    render_objects.sort_by_key(|o| std::cmp::Reverse(o.start_time));
    let start_times: Vec<i64> = render_objects.iter().map(|o| o.start_time).collect();

    let render = move |frame_index: usize| -> (Img, i64) {
        let snapshot_time = start + rhe(frame_index as f64 * 1000.0 * speed / fps as f64);
        let frame = render_gif_frame(&render_objects, &start_times, snapshot_time, &layout);
        (frame, snapshot_time)
    };

    save_mp4_streamed(
        frame_count,
        start,
        last,
        speed,
        render,
        output_path,
        fps,
        audio_job,
        time_axis,
    )
}
