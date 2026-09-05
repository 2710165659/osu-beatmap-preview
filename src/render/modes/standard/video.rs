//! osu!standard MP4 渲染器：完整谱面连续播放（无 2×2 分段预览）。
//! 复用 GIF 路径的 `render_frame`，确保每帧像素与 GIF 单段外观一致，
//! 仅时间轴和取景方式不同。
//!
//! 时间范围由 `--time-points` 和 `--duration-time` 控制。

use crate::domain::errors::Result;
use crate::domain::models::Beatmap;
use crate::domain::mods::ModSettings;
use crate::domain::parser::round_half_even;
use crate::domain::shared::time_selection::TimeAxis;
use crate::domain::timeout::RequestDeadline;
use crate::domain::validate::TimePoint;
use crate::infrastructure::media::audio::AudioSourceJob;
use crate::infrastructure::media::{resolve_video_time_range, save_mp4_streamed};
use crate::render::canvas::Img;
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
    background: Option<Img>,
    audio_job: AudioSourceJob,
    time_axis: TimeAxis,
    fps: Option<u32>,
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
    let context = build_render_context(
        beatmap,
        hit_objects,
        mods,
        time_axis,
        crate::render::geometry::OutputFormat::Mp4,
    );
    let total_ms = end - start;
    let fps = fps.unwrap_or(
        crate::infrastructure::config::current()
            .render
            .standard
            .mp4
            .style
            .FPS as u32,
    );
    let frame_count = ((total_ms as f64 * fps as f64 / (1000.0 * speed)).round() as usize).max(1);
    // 视频帧会被并行且可能乱序地请求；先按时间轴一次性生成可见物件索引，
    // 避免每帧重复排序完整谱面并分配临时 Vec。索引仅保存 usize，
    // 相比 RGBA 帧缓冲占用很小，且不改变任何帧的物件顺序。
    let snapshot_times: Vec<i64> = (0..frame_count)
        .map(|frame_index| {
            start + round_half_even(frame_index as f64 * 1000.0 * speed / fps as f64)
        })
        .collect();
    let visible_indexes = build_visible_indexes_by_snapshot(
        &context.hit_objects,
        &snapshot_times,
        context.settings.preempt_ms,
    );
    // 视频背景在最终 16:9 画布上统一处理；playfield 只提供透明对象层，
    // 避免同一张图在 playfield 和画布中被分别缩放、裁剪。
    let frame_background = background.as_ref().map(|_| {
        Img::new(
            context.frame_layout.frame_width as u32,
            context.frame_layout.frame_height as u32,
            [0, 0, 0, 0],
        )
    });

    let break_periods = beatmap.break_periods.clone();
    let context_ref = &context;
    let break_ref = &break_periods;

    // 每线程独立渲染缓存，避免并行 render_frame 调用在同一 Mutex 后串行化（视频帧数远多于 GIF）。
    thread_local! {
        static STD_VIDEO_CACHE: RefCell<RenderCache> = RefCell::new(RenderCache::default());
    }

    let render = move |frame_index: usize| -> (Img, i64) {
        let snapshot_time = snapshot_times[frame_index];
        let frame = STD_VIDEO_CACHE.with(|cache| {
            render_frame(
                context_ref,
                &mut cache.borrow_mut(),
                snapshot_time,
                break_ref,
                &visible_indexes[frame_index],
                frame_background.as_ref(),
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
        background,
        time_axis,
        deadline,
        crate::render::geometry::GameMode::Standard,
    )
}
