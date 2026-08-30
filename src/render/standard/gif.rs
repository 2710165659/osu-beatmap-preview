//! osu!standard GIF 渲染器：2×2 分段预览或单画面片段。

use crate::common::time_selection::{GifRenderOptions, TimeAxis};
use crate::core::errors::Result;
use crate::core::models::Beatmap;
use crate::core::mods::ModSettings;
use crate::core::timeout::RequestDeadline;
use crate::render::canvas::Img;
use crate::render::composer::save_animated_gif_streamed;
use crate::render::text::format_mmssmmm;
use std::cell::RefCell;
use std::path::Path;

use super::context::*;
use super::draw_time_label;
use super::objects::render_frame;

pub(crate) fn render_standard_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    options: GifRenderOptions,
    output_path: &Path,
    deadline: &RequestDeadline,
) -> Result<()> {
    deadline.check()?;
    match options {
        GifRenderOptions::Segments {
            times_ms,
            duration_seconds,
            time_axis,
        } => render_standard_segment_gif(
            beatmap,
            mods,
            times_ms,
            duration_seconds,
            time_axis,
            output_path,
            deadline,
        ),
    }
}

fn render_standard_segment_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    times_ms: Option<Vec<i64>>,
    duration_seconds: Option<f64>,
    time_axis: TimeAxis,
    output_path: &Path,
    deadline: &RequestDeadline,
) -> Result<()> {
    let hit_objects = standard_objects(beatmap)?;
    let hit_objects = apply_standard_object_mods(hit_objects, mods);
    let context = build_render_context(beatmap, hit_objects, mods, time_axis);
    let speed_multiplier = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let segment_duration_ms = duration_seconds
        .map(|seconds| seconds * 1000.0)
        .unwrap_or(crate::config::current().layout.standard.gif.DURATION_MS as f64);
    let gameplay_segment_duration = py_round(segment_duration_ms * speed_multiplier);
    let row_timings = choose_row_start_times(
        beatmap,
        &context.hit_objects,
        crate::config::current().layout.standard.gif.ROW_COUNT
            * crate::config::current().layout.standard.gif.IMAGES_PER_ROW,
        2,
        gameplay_segment_duration,
        times_ms,
    )?;

    let (canvas_w, canvas_h) = gif_canvas_size();
    let frame_count =
        ((segment_duration_ms * crate::config::current().layout.standard.gif.FPS as f64 / 1000.0)
            .round() as usize)
            .max(1);
    let frame_duration_ms =
        ((1000.0 / crate::config::current().layout.standard.gif.FPS as f64).round() as u32).max(1);

    let segment_snapshot_times: Vec<Vec<i64>> = row_timings
        .iter()
        .map(|rt| {
            (0..frame_count)
                .map(|fi| {
                    rt.start_time
                        + py_round(
                            fi as f64 * 1000.0 * speed_multiplier
                                / crate::config::current().layout.standard.gif.FPS as f64,
                        )
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

    // 每线程独立渲染缓存，避免并行 render_frame 调用在同一 Mutex 后串行化。
    // 否则 rayon 分块渲染会在锁上排队，使吞吐量减半。每个线程拥有自己的缓存；
    // 前几帧构建程序化纹理，之后主要命中缓存。
    thread_local! {
        static STD_GIF_CACHE: RefCell<RenderCache> = RefCell::new(RenderCache::default());
    }

    let render = move |frame_index: usize| -> Img {
        let mut canvas = Img::new(
            canvas_w as u32,
            canvas_h as u32,
            crate::config::current()
                .layout
                .standard
                .gif
                .CANVAS_BACKGROUND_COLOR,
        );
        for (segment_index, row_timing) in row_timings.iter().enumerate() {
            let (x, y) = gif_frame_origin(segment_index);
            let snapshot_time = segment_snapshot_times[segment_index][frame_index];
            let frame = STD_GIF_CACHE.with(|cache| {
                render_frame(
                    &context,
                    &mut cache.borrow_mut(),
                    snapshot_time,
                    &row_timing.break_periods,
                    &segment_visible_indexes[segment_index][frame_index],
                    None,
                )
            });
            canvas.alpha_composite(&frame, x, y);
            let note = if row_timing.is_preview {
                Some("Preview Time")
            } else {
                None
            };
            let label = format!(
                "{} - {}",
                format_mmssmmm(time_axis.to_display(row_timing.start_time)),
                format_mmssmmm(
                    time_axis.to_display(row_timing.start_time + gameplay_segment_duration)
                )
            );
            if crate::config::current().layout.standard.gif.SHOW_TIME_LABEL {
                draw_time_label(
                    &mut canvas,
                    &label,
                    x,
                    y + crate::render::standard::constants::IMAGE_HEIGHT
                        + crate::config::current()
                            .layout
                            .standard
                            .gif
                            .TIME_LABEL_TOP_GAP,
                    note,
                    if row_timing.is_preview {
                        crate::config::current()
                            .layout
                            .standard
                            .gif
                            .PREVIEW_TIME_LABEL_COLOR
                    } else {
                        crate::config::current()
                            .layout
                            .standard
                            .gif
                            .TIME_LABEL_COLOR
                    },
                    if row_timing.is_preview {
                        crate::config::current()
                            .layout
                            .standard
                            .gif
                            .PREVIEW_TIME_LABEL_COLOR
                    } else {
                        crate::config::current()
                            .layout
                            .standard
                            .gif
                            .TIME_LABEL_NOTE_COLOR
                    },
                );
            }
        }
        canvas
    };

    save_animated_gif_streamed(
        frame_count,
        render,
        output_path,
        frame_duration_ms,
        deadline,
    )
}
