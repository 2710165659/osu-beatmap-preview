//! osu!standard MP4 renderer: full-chart continuous playback (no 2×2 segment
//! preview). Reuses `render_frame` from the GIF path so per-frame pixels match
//! the GIF's single-segment look; only the time axis and framing differ.
//!
//! Time range is controlled by `--time-points` and `--duration-time`.

use crate::common::time_selection::TimeAxis;
use crate::core::errors::Result;
use crate::core::models::Beatmap;
use crate::core::mods::ModSettings;
use crate::core::timeout::RequestDeadline;
use crate::core::validate::TimePoint;
use crate::parser::round_half_even;
use crate::render::canvas::Img;
use crate::render::video::audio::AudioSourceJob;
use crate::render::video::{resolve_video_time_range, save_mp4_streamed};
use std::cell::RefCell;
use std::path::Path;

use super::context::{
    apply_standard_object_mods, build_render_context, build_visible_indexes_by_snapshot,
    standard_objects, RenderCache,
};
use super::objects::render_frame;

pub(crate) fn render_standard_video(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    start_time: Option<TimePoint>,
    duration_time: Option<f64>,
    output_path: &Path,
    audio_job: AudioSourceJob,
    time_axis: TimeAxis,
    deadline: &RequestDeadline,
) -> Result<()> {
    deadline.check()?;
    let hit_objects = standard_objects(beatmap)?;
    let speed = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let first = hit_objects.iter().map(|o| o.start_time).min().unwrap_or(0);
    let last = hit_objects.iter().map(|o| o.end_time).max().unwrap_or(0);
    let range = resolve_video_time_range(beatmap, first, last, start_time, duration_time, speed)?;
    let (start, end) = (range.start, range.end);
    let hit_objects = apply_standard_object_mods(hit_objects, mods);
    let context = build_render_context(beatmap, hit_objects, mods, time_axis);
    let total_ms = end - start;
    let fps = crate::config::current().layout.standard.mp4.FPS as u32;
    let frame_count = ((total_ms as f64 * fps as f64 / (1000.0 * speed)).round() as usize).max(1);

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
        deadline,
    )
}
