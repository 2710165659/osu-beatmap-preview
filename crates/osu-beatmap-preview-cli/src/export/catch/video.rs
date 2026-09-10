//! osu!catch MP4 渲染器：完整谱面连续播放（无 2×2 分段预览）。
//! 复用共享动画路径的 `render_animation_frame`，确保每帧像素与 GIF 单段外观一致，
//! 仅时间轴和取景方式不同。
//!
//! 时间范围由 `--time-points` 和 `--duration-time` 控制。

use crate::export::canvas::Img;
use crate::media::audio::AudioSourceJob;
use crate::media::{resolve_video_time_range, save_mp4_streamed};
use osu_beatmap_preview_core::model::mods::ModSettings;
use osu_beatmap_preview_core::model::Beatmap;
use osu_beatmap_preview_core::processing::timeline::TimeAxis;
use osu_beatmap_preview_core::processing::validation::TimePoint;
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
use osu_beatmap_preview_core::support::timeout::RequestDeadline;
use std::path::Path;

use super::animation::{build_animation_layout, render_animation_frame};
use super::objects::{build_catch_render_objects, effective_difficulty};
use super::png::rhe;

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_catch_video(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    start_time: Option<TimePoint>,
    duration_time: Option<f64>,
    output_path: &Path,
    background: Option<Img>,
    audio_job: AudioSourceJob,
    time_axis: TimeAxis,
    fps: Option<u32>,
    deadline: &RequestDeadline,
) -> Result<()> {
    deadline.check()?;
    let hit_objects = match beatmap.hit_objects.as_catch() {
        Some(v) if !v.is_empty() => v,
        _ => return Err(PreviewError::render("catch beatmap has no hit objects")),
    };
    let difficulty = effective_difficulty(beatmap, mods);
    let mut render_objects =
        build_catch_render_objects(beatmap, hit_objects, mods, &difficulty, false)?;

    let speed = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let first = hit_objects.iter().map(|h| h.start_time).min().unwrap_or(0);
    let last = hit_objects.iter().map(|h| h.end_time).max().unwrap_or(0);
    let range = resolve_video_time_range(beatmap, first, last, start_time, duration_time, speed)?;
    let (start, end) = (range.start, range.end);
    let total_ms = end - start;
    let fps = fps.unwrap_or_else(|| crate::config::current().render.catch.mp4.style.FPS as u32);
    let frame_count = ((total_ms as f64 * fps as f64 / (1000.0 * speed)).round() as usize).max(1);

    let layout = build_animation_layout(
        difficulty.cs,
        difficulty.ar,
        crate::export::geometry::OutputFormat::Mp4,
    );
    // 视频背景在最终 16:9 画布上统一处理，playfield 只提供透明对象层。
    let frame_background = background.as_ref().map(|_| {
        Img::new(
            layout.frame_width as u32,
            layout.frame_height as u32,
            [0, 0, 0, 0],
        )
    });
    render_objects.sort_by_key(|o| std::cmp::Reverse(o.start_time));
    let start_times: Vec<i64> = render_objects.iter().map(|o| o.start_time).collect();

    let render = move |frame_index: usize| -> Result<(Img, i64)> {
        let snapshot_time = start + rhe(frame_index as f64 * 1000.0 * speed / fps as f64);
        let frame = render_animation_frame(
            &render_objects,
            &start_times,
            snapshot_time,
            &layout,
            frame_background.as_ref(),
        );
        Ok((frame, snapshot_time))
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
        background,
        time_axis,
        deadline,
        crate::export::geometry::GameMode::Catch,
        crate::media::FrameComposition::Playfield,
    )
}
