//! osu!taiko MP4 渲染器：完整谱面连续播放（无四行分段预览）。
//! 复用共享动画层的行背景和音符绘制，并使用单行布局；省略每段底部标签，
//! 全局右上角标签由 `video::save_mp4_streamed` 绘制。
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
use std::cell::RefCell;
use std::path::Path;

use super::animation_render::{
    build_multiplier_points, build_video_animation_layout, compute_time_range, draw_hit_objects,
    draw_row_background, prepare_hit_objects, prepare_measure_lines, pyround, AnimationLayout,
    MultiplierLookup,
};
use super::notes::RenderCache;
use super::timing::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_taiko_video(
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
    let fps = fps.unwrap_or_else(|| crate::config::current().render.taiko.mp4.style.FPS as u32);
    let frame_count = ((total_ms as f64 * fps as f64 / (1000.0 * speed)).round() as usize).max(1);

    let slider_multiplier = effective_slider_multiplier(beatmap, mods)?;
    let timing_points = effective_timing_points(beatmap, mods);
    let multiplier_lookup = MultiplierLookup {
        points: build_multiplier_points(&timing_points, slider_multiplier),
    };
    let slider_tick_rate = beatmap.difficulty.get_f64_or("SliderTickRate", 1.0);
    let prepared_hit_objects = prepare_hit_objects(
        &hit_objects,
        &multiplier_lookup,
        &timing_points,
        slider_tick_rate,
    );
    let prepared_measure_lines = prepare_measure_lines(
        &hit_objects,
        &timing_points,
        &multiplier_lookup,
        crate::config::current()
            .render
            .taiko
            .mp4
            .style
            .SHOW_MEASURE_LINES,
    );
    let time_range = compute_time_range() / speed;
    let layout = build_video_layout(time_range);

    let static_bg = {
        // 背景图在最终视频画布上统一处理；这里仅绘制 Taiko 自身的轨道面板。
        // 物件层与视频画布同尺寸，底色只填内容带，补边仍由画布底色决定。
        let mut bg = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            [0, 0, 0, 0],
        );
        if background.is_none() {
            let content = layout.content;
            bg.fill_rect_size(
                content.x,
                content.y,
                content.width,
                content.height,
                crate::config::current()
                    .render
                    .taiko
                    .mp4
                    .style
                    .IMAGE_BACKGROUND,
            );
        }
        draw_row_background(&mut bg, &layout, 0);
        bg
    };

    // 每线程独立渲染缓存，避免并行 draw_hit_objects 调用在同一 Mutex 后串行化；
    // 视频帧数远多于 GIF。
    thread_local! {
        static TAIKO_VIDEO_CACHE: RefCell<RenderCache> = RefCell::new(RenderCache::default());
    }

    let render = move |frame_index: usize| -> Result<(Img, i64)> {
        let snapshot_time = start + pyround(frame_index as f64 * 1000.0 * speed / fps as f64);
        let mut canvas = static_bg.clone();
        TAIKO_VIDEO_CACHE.with(|cache| {
            draw_hit_objects(
                &mut canvas,
                &prepared_hit_objects,
                &prepared_measure_lines,
                &layout,
                0,
                snapshot_time,
                &mut cache.borrow_mut(),
            );
        });
        Ok((canvas, snapshot_time))
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
        beatmap.clone(),
        background,
        time_axis,
        deadline,
        crate::export::geometry::GameMode::Taiko,
        crate::media::FrameComposition::Canvas,
    )
}

/// MP4 的单行布局：帧尺寸为视频画布（物件只在视频边界被裁），单行内容带居中
///（无四行堆叠、无行间距、无底部标签条）。
fn build_video_layout(time_range: f64) -> AnimationLayout {
    build_video_animation_layout(time_range, crate::export::geometry::OutputFormat::Mp4)
}
