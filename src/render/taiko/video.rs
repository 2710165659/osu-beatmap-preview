//! osu!taiko MP4 渲染器：完整谱面连续播放（无四行分段预览）。
//! 复用 GIF 的行背景和音符绘制，并使用单行布局；省略每段底部标签，
//! 全局右上角标签由 `video::save_mp4_streamed` 绘制。
//!
//! 时间范围由 `--time-points` 和 `--duration-time` 控制。

use crate::common::time_selection::TimeAxis;
use crate::core::errors::{PreviewError, Result};
use crate::core::models::Beatmap;
use crate::core::mods::ModSettings;
use crate::core::timeout::RequestDeadline;
use crate::core::validate::TimePoint;
use crate::render::canvas::Img;
use crate::render::video::audio::AudioSourceJob;
use crate::render::video::{resolve_video_time_range, save_mp4_streamed};
use std::cell::RefCell;
use std::path::Path;

use super::gif::{
    build_gif_layout, build_multiplier_points, compute_time_range, draw_hit_objects,
    draw_row_background, prepare_hit_objects, pyround, GifLayout, MultiplierLookup,
};
use super::notes::RenderCache;
use super::timing::*;

pub(crate) fn render_taiko_video(
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
    let hit_objects = apply_taiko_object_mods(taiko_hit_objects(beatmap), mods);
    if hit_objects.is_empty() {
        return Err(PreviewError::render("taiko beatmap has no hit objects"));
    }

    let speed = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let first = hit_objects.iter().map(|h| h.start_time).min().unwrap_or(0);
    let last = hit_objects.iter().map(|h| h.end_time).max().unwrap_or(0);
    let range = resolve_video_time_range(beatmap, first, last, start_time, duration_time, speed)?;
    let (start, end) = (range.start, range.end);
    let total_ms = end - start;
    let fps = crate::config::current().layout.taiko.mp4.FPS as u32;
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
            crate::config::current().layout.taiko.mp4.IMAGE_BACKGROUND,
        );
        draw_row_background(&mut bg, &layout, 0);
        bg
    };

    // 每线程独立渲染缓存，避免并行 draw_hit_objects 调用在同一 Mutex 后串行化；
    // 视频帧数远多于 GIF。
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
                &mut cache.borrow_mut(),
            );
        });
        (canvas, snapshot_time)
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

/// MP4 的单行布局：宽度与 GIF 相同，高度裁剪为一行
///（无四行堆叠、无行间距、无底部标签条）。
fn build_video_layout(time_range: f64) -> GifLayout {
    let mut layout = build_gif_layout(time_range);
    layout.image_height = crate::render::taiko::constants::PAGE_MARGIN_Y * 2
        + crate::render::taiko::constants::ROW_HEIGHT;
    layout
}
