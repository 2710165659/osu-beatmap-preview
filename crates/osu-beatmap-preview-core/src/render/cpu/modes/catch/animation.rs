//! osu!catch GIF 渲染器：2×2 分段动画预览或单屏区间预览。
//!
//! 单帧 683×384（16:9），playfield 的位置与缩放按游戏内 1080p 等比换算
//! （见 `render.catch.gif` 中的 playfield 配置），上下左右留白与游戏一致。

use crate::domain::errors::{PreviewError, Result};
use crate::domain::models::Beatmap;
use crate::domain::mods::ModSettings;
use crate::domain::shared::time_selection::{GifRenderOptions, PreviewTimeSelector, TimeAxis};
use crate::domain::timeout::RequestDeadline;
use crate::render::canvas::Img;
use crate::render::cpu::AnimationFrames;
use crate::render::text::{draw_text, text_size};

use super::drawing::{draw_catch_object, object_diameter};
use super::objects::{build_catch_render_objects, effective_difficulty, RenderObject};
fn rhe(value: f64) -> i64 {
    crate::domain::parser::round_half_even(value)
}

// ─── GIF 布局 ───

pub struct AnimationLayout {
    pub canvas_width: i64,
    pub canvas_height: i64,
    /// playfield（512 宽坐标系）在帧内的缩放。
    pub playfield_scale: f64,
    /// playfield 左边缘在帧内的 x 坐标。
    pub playfield_left: f64,
    /// playfield 顶部在帧内的 y 坐标。
    pub playfield_top: f64,
    pub object_scale: f64,
    pub pixels_per_ms: f64,
    pub render_scale: f64,
    pub frame_width: i64,
    pub frame_height: i64,
    /// 物件层缓冲内的内容框。视频物件层的帧就是最终画布，底色只填这块区域，
    /// 物件允许溢出到画布边缘；GIF 与实时链路的内容框等于整帧。
    pub content: crate::render::geometry::PixelRect,
    pub playfield_background: [u8; 4],
    pub judgement_line_color: [u8; 4],
}

pub fn build_animation_layout(
    circle_size: f64,
    approach_rate: f64,
    output_format: crate::render::geometry::OutputFormat,
) -> AnimationLayout {
    let geometry = crate::render::geometry::catch_geometry(output_format);
    let render_scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Catch,
        output_format,
    );
    let playfield_scale =
        crate::render::cpu::modes::catch::constants::PLAYFIELD_SCALE * render_scale;
    let playfield_left = geometry.playfield.x as f64;
    let playfield_top = geometry.playfield.y as f64;
    let object_scale = super::objects::circle_scale(circle_size);

    // 下落速度：AR 时间窗对应「起始高度→接手」的可视下落距离
    let time_range = super::objects::catch_time_range(approach_rate);
    let visible_fall_height = (crate::render::cpu::modes::catch::constants::STABLE_CATCHER_Y
        - crate::render::cpu::modes::catch::constants::STABLE_FRUIT_START_Y)
        * playfield_scale;
    let pixels_per_ms = visible_fall_height / time_range;

    let (canvas_width, canvas_height) = match output_format {
        crate::render::geometry::OutputFormat::Gif => {
            let config = &crate::config::current().render.catch.gif;
            let unit_width = geometry.content.width
                + config.sizing.INFO_MARGIN_LEFT
                + config.sizing.INFO_MARGIN_RIGHT;
            let info_bottom = if config.style.SHOW_TIME_LABEL {
                config.sizing.INFO_MARGIN_BOTTOM
            } else {
                0
            };
            let unit_height = geometry.content.height + config.sizing.INFO_MARGIN_TOP + info_bottom;
            (
                config.sizing.PAGE_MARGIN_LEFT
                    + config.sizing.PAGE_MARGIN_RIGHT
                    + config.structure.IMAGES_PER_ROW as i64 * unit_width
                    + (config.structure.IMAGES_PER_ROW as i64 - 1) * config.sizing.GRID_GAP,
                config.sizing.PAGE_MARGIN_TOP
                    + config.sizing.PAGE_MARGIN_BOTTOM
                    + config.structure.ROW_COUNT as i64 * unit_height
                    + (config.structure.ROW_COUNT as i64 - 1) * config.sizing.GRID_GAP,
            )
        }
        crate::render::geometry::OutputFormat::Mp4 => {
            (geometry.content.width, geometry.content.height)
        }
        crate::render::geometry::OutputFormat::Png => unreachable!("PNG 不使用动画布局"),
    };

    AnimationLayout {
        canvas_width,
        canvas_height,
        playfield_scale,
        playfield_left,
        playfield_top,
        object_scale,
        pixels_per_ms,
        render_scale,
        frame_width: geometry.content.width,
        frame_height: geometry.content.height,
        content: crate::render::geometry::PixelRect {
            x: 0,
            y: 0,
            width: geometry.content.width,
            height: geometry.content.height,
        },
        playfield_background: match output_format {
            crate::render::geometry::OutputFormat::Mp4 => {
                crate::config::current()
                    .render
                    .catch
                    .mp4
                    .style
                    .PLAYFIELD_BACKGROUND
            }
            _ => {
                crate::config::current()
                    .render
                    .catch
                    .gif
                    .style
                    .PLAYFIELD_BACKGROUND
            }
        },
        judgement_line_color:
            crate::render::cpu::modes::catch::constants::ANIMATION_JUDGEMENT_LINE_COLOR,
    }
}

/// MP4 物件层布局：帧尺寸 = 视频画布，playfield 与内容框按 [`video_canvas`] 居中。
///
/// 物件层与最终画布同尺寸后，水果只会被视频边界裁剪，不会再被内容框切掉一半；
/// `playfield_scale` 与物件直径完全沿用内容框布局，因此分辨率与物件大小都不变。
pub fn build_video_animation_layout(
    circle_size: f64,
    approach_rate: f64,
    output_format: crate::render::geometry::OutputFormat,
) -> AnimationLayout {
    let layout = build_animation_layout(circle_size, approach_rate, output_format);
    let geometry = crate::render::geometry::catch_geometry(output_format);
    let canvas = crate::render::geometry::video_canvas(geometry.content);
    AnimationLayout {
        canvas_width: canvas.width as i64,
        canvas_height: canvas.height as i64,
        playfield_left: layout.playfield_left + canvas.origin_x as f64,
        playfield_top: layout.playfield_top + canvas.origin_y as f64,
        frame_width: canvas.width as i64,
        frame_height: canvas.height as i64,
        content: crate::render::geometry::PixelRect {
            x: canvas.origin_x,
            y: canvas.origin_y,
            width: layout.frame_width,
            height: layout.frame_height,
        },
        ..layout
    }
}

/// 第 segment_index 段在画布上的左上角。
fn frame_origin(segment_index: usize, layout: &AnimationLayout) -> (i64, i64) {
    let config = &crate::config::current().render.catch.gif;
    let row_index = segment_index as i64
        / crate::config::current()
            .render
            .catch
            .gif
            .structure
            .IMAGES_PER_ROW as i64;
    let col_index = segment_index as i64
        % crate::config::current()
            .render
            .catch
            .gif
            .structure
            .IMAGES_PER_ROW as i64;
    let unit_width =
        layout.frame_width + config.sizing.INFO_MARGIN_LEFT + config.sizing.INFO_MARGIN_RIGHT;
    let info_bottom = if config.style.SHOW_TIME_LABEL {
        config.sizing.INFO_MARGIN_BOTTOM
    } else {
        0
    };
    let unit_height = layout.frame_height + config.sizing.INFO_MARGIN_TOP + info_bottom;
    let x = config.sizing.PAGE_MARGIN_LEFT
        + config.sizing.INFO_MARGIN_LEFT
        + col_index * (unit_width + config.sizing.GRID_GAP);
    let y = config.sizing.PAGE_MARGIN_TOP
        + config.sizing.INFO_MARGIN_TOP
        + row_index * (unit_height + config.sizing.GRID_GAP);
    (x, y)
}

// ─── 对外接口 ───

pub fn prepare_catch_gif_frames(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    options: GifRenderOptions,
    fps: Option<u32>,
    deadline: &RequestDeadline,
) -> Result<AnimationFrames> {
    deadline.check()?;
    match options {
        GifRenderOptions::Segments {
            times_ms,
            duration_seconds,
            time_axis,
        } => prepare_catch_segment_gif_frames(
            beatmap,
            mods,
            times_ms,
            duration_seconds,
            time_axis,
            fps,
            deadline,
        ),
    }
}

fn prepare_catch_segment_gif_frames(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    times_ms: Option<Vec<i64>>,
    duration_seconds: Option<f64>,
    time_axis: TimeAxis,
    fps: Option<u32>,
    deadline: &RequestDeadline,
) -> Result<AnimationFrames> {
    deadline.check()?;
    let hit_objects = match beatmap.hit_objects.as_catch() {
        Some(v) if !v.is_empty() => v,
        _ => return Err(PreviewError::render("catch beatmap has no hit objects")),
    };

    let difficulty = effective_difficulty(beatmap, mods);
    let mut render_objects =
        build_catch_render_objects(beatmap, hit_objects, mods, &difficulty, false)?;

    let speed_multiplier = mods.map(|m| m.speed_multiplier).unwrap_or(1.0);
    let segment_duration_ms = duration_seconds
        .map(|seconds| seconds * 1000.0)
        .unwrap_or(crate::config::current().render.catch.gif.style.DURATION_MS);
    let gameplay_segment_duration = rhe(segment_duration_ms * speed_multiplier);
    let fps = fps
        .map(f64::from)
        .unwrap_or(crate::config::current().render.catch.gif.style.FPS);
    let spans: Vec<(i64, i64)> = hit_objects
        .iter()
        .map(|h| (h.start_time, h.end_time))
        .collect();
    let segment_timings = PreviewTimeSelector::new(
        beatmap,
        spans,
        (crate::config::current()
            .render
            .catch
            .gif
            .structure
            .ROW_COUNT
            * crate::config::current()
                .render
                .catch
                .gif
                .structure
                .IMAGES_PER_ROW) as usize,
        gameplay_segment_duration,
        times_ms,
    )?
    .choose()?;

    let layout = build_animation_layout(
        difficulty.cs,
        difficulty.ar,
        crate::render::geometry::OutputFormat::Gif,
    );
    let frame_count = rhe(segment_duration_ms * fps / 1000.0).max(1) as usize;
    let frame_duration_ms = rhe(1000.0 / fps).max(1) as u32;

    let segment_snapshot_times: Vec<Vec<i64>> = segment_timings
        .iter()
        .map(|timing| {
            (0..frame_count)
                .map(|frame_index| {
                    timing.start_time + rhe(frame_index as f64 * 1000.0 * speed_multiplier / fps)
                })
                .collect()
        })
        .collect();

    // 按开始时间降序排序，先画晚出现的对象，后画早出现的（早的盖在上层）
    render_objects.sort_by_key(|o| std::cmp::Reverse(o.start_time));
    // 各段按时间二分裁剪可见窗口，避免每帧全量遍历
    let start_times: Vec<i64> = render_objects.iter().map(|o| o.start_time).collect();

    let render = move |frame_index: usize| -> Img {
        let mut canvas = Img::new(
            layout.canvas_width as u32,
            layout.canvas_height as u32,
            crate::config::current()
                .render
                .catch
                .gif
                .style
                .PLAYFIELD_BACKGROUND,
        );
        for (segment_index, segment_timing) in segment_timings.iter().enumerate() {
            let snapshot_time = segment_snapshot_times[segment_index][frame_index];
            let (frame_x, frame_y) = frame_origin(segment_index, &layout);
            let frame =
                render_animation_frame(&render_objects, &start_times, snapshot_time, &layout, None);
            canvas.alpha_composite(&frame, frame_x, frame_y);
            if crate::config::current()
                .render
                .catch
                .gif
                .style
                .SHOW_TIME_LABEL
            {
                draw_gif_time_label(
                    &mut canvas,
                    segment_timing.start_time,
                    gameplay_segment_duration,
                    frame_x,
                    frame_y,
                    segment_timing.is_preview,
                    time_axis,
                    &layout,
                );
            }
        }
        canvas
    };

    Ok(AnimationFrames::new(frame_count, frame_duration_ms, render))
}

/// 渲染单段单帧：背景 + 判定线 + 接手 + 可见的下落对象。
pub fn render_animation_frame(
    render_objects: &[RenderObject],
    start_times_desc: &[i64],
    snapshot_time: i64,
    layout: &AnimationLayout,
    background: Option<&Img>,
) -> Img {
    let mut frame = match background {
        Some(background) => background.clone(),
        None => {
            // 视频物件层的帧是最终画布，底色只填内容框：内容框之外的补边仍由
            // 合成阶段用画布底色处理，水果则可以溢出到画布边缘而不被切掉。
            let mut frame = Img::new(
                layout.frame_width as u32,
                layout.frame_height as u32,
                [0, 0, 0, 0],
            );
            let content = layout.content;
            frame.fill_rect_size(
                content.x,
                content.y,
                content.width,
                content.height,
                layout.playfield_background,
            );
            frame
        }
    };

    let playfield_left = layout.playfield_left;
    let playfield_right = playfield_left
        + crate::render::cpu::modes::catch::constants::PLAYFIELD_WIDTH * layout.playfield_scale;
    // playfield 区域底色
    if background.is_none() {
        frame.set_rect_size(
            rhe(playfield_left),
            layout.content.y,
            rhe(playfield_right) - rhe(playfield_left),
            layout.content.height,
            layout.playfield_background,
        );
    }

    // 判定线（接手所在高度）
    let judgement_y = layout.playfield_top
        + crate::render::cpu::modes::catch::constants::STABLE_CATCHER_Y * layout.playfield_scale;
    let judgement_y_px = rhe(judgement_y);
    let line_height = crate::render::geometry::scale_stroke_px(2.0, layout.render_scale);
    frame.set_rect_size(
        rhe(playfield_left),
        judgement_y_px,
        rhe(playfield_right) - rhe(playfield_left),
        line_height,
        layout.judgement_line_color,
    );

    // 可见时间窗：对象在 [snapshot, snapshot + 下落时间窗 + 余量] 内才可能出现在帧中。
    // 下落时间窗按内容框高度计算，与画布补边无关。
    let fall_window_ms = (layout.content.height as f64 / layout.pixels_per_ms).ceil() as i64 + 2000;
    // start_times_desc 为降序；找到可见区间 [lo, hi)
    let lo = start_times_desc.partition_point(|&t| t > snapshot_time + fall_window_ms);
    let hi = start_times_desc.partition_point(|&t| t >= snapshot_time - 2000);

    for catch_object in &render_objects[lo..hi] {
        draw_gif_object(&mut frame, catch_object, snapshot_time, judgement_y, layout);
    }

    frame
}

/// 绘制单个下落中的对象（超出帧范围的直接跳过）。
fn draw_gif_object(
    frame: &mut Img,
    catch_object: &RenderObject,
    snapshot_time: i64,
    judgement_y: f64,
    layout: &AnimationLayout,
) {
    let local_time = catch_object.start_time - snapshot_time;
    let center_x = layout.playfield_left + catch_object.x * layout.playfield_scale;
    let center_y = judgement_y - local_time as f64 * layout.pixels_per_ms;
    let diameter = object_diameter(
        layout.object_scale,
        layout.playfield_scale,
        catch_object.scale_factor,
    );

    if center_y + diameter / 2.0 < 0.0 || center_y - diameter / 2.0 > judgement_y {
        return;
    }

    draw_catch_object(frame, catch_object, center_x, center_y, diameter);
}

#[allow(clippy::too_many_arguments)]
fn draw_gif_time_label(
    canvas: &mut Img,
    start_time: i64,
    duration_ms: i64,
    frame_x: i64,
    frame_y: i64,
    is_preview: bool,
    time_axis: TimeAxis,
    layout: &AnimationLayout,
) {
    let label = format!(
        "{} - {}",
        crate::render::text::format_mmss_floor(time_axis.to_display(start_time)),
        crate::render::text::format_mmss_floor(time_axis.to_display(start_time + duration_ms))
    );
    let color = if is_preview {
        crate::config::current()
            .render
            .catch
            .gif
            .style
            .PREVIEW_TIME_LABEL_COLOR
    } else {
        crate::config::current()
            .render
            .catch
            .gif
            .style
            .TIME_LABEL_COLOR
    };
    let note_color = if is_preview {
        crate::config::current()
            .render
            .catch
            .gif
            .style
            .PREVIEW_TIME_LABEL_COLOR
    } else {
        crate::config::current()
            .render
            .catch
            .gif
            .style
            .TIME_LABEL_NOTE_COLOR
    };
    let (label_w, label_h) = text_size(
        &label,
        crate::config::current()
            .render
            .catch
            .gif
            .sizing
            .TIME_LABEL_FONT_SIZE,
    );
    let x = frame_x + (layout.frame_width - label_w as i64) / 2;
    let y = frame_y
        + layout.frame_height
        + crate::config::current()
            .render
            .catch
            .gif
            .sizing
            .TIME_LABEL_TOP_GAP;
    draw_text(
        canvas,
        x,
        y,
        &label,
        crate::config::current()
            .render
            .catch
            .gif
            .sizing
            .TIME_LABEL_FONT_SIZE,
        color,
    );

    if is_preview {
        let note = "Preview Time";
        let (note_w, _) = text_size(
            note,
            crate::config::current()
                .render
                .catch
                .gif
                .sizing
                .TIME_LABEL_NOTE_FONT_SIZE,
        );
        let note_x = frame_x + (layout.frame_width - note_w as i64) / 2;
        draw_text(
            canvas,
            note_x,
            y + label_h as i64
                + crate::config::current()
                    .render
                    .catch
                    .gif
                    .sizing
                    .TIME_LABEL_NOTE_TOP_GAP,
            note,
            crate::config::current()
                .render
                .catch
                .gif
                .sizing
                .TIME_LABEL_NOTE_FONT_SIZE,
            note_color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::cpu::modes::catch::drawing::object_diameter;
    use crate::render::cpu::modes::catch::objects::{ObjType, RenderObject};

    #[test]
    fn video_layout_centers_content_box_without_resizing_fruit() {
        for scale in [1.0_f64, 2.0] {
            let mut custom = crate::config::CoreConfig::default();
            custom.render.catch.mp4.SCALE = scale;
            crate::config::with_config(std::sync::Arc::new(custom), || {
                let content =
                    build_animation_layout(4.0, 5.0, crate::render::geometry::OutputFormat::Mp4);
                let video = build_video_animation_layout(
                    4.0,
                    5.0,
                    crate::render::geometry::OutputFormat::Mp4,
                );
                let geometry = crate::render::geometry::catch_geometry(
                    crate::render::geometry::OutputFormat::Mp4,
                );
                let canvas = crate::render::geometry::video_canvas(geometry.content);
                assert_eq!(
                    (video.frame_width, video.frame_height),
                    (canvas.width as i64, canvas.height as i64)
                );
                assert_eq!(
                    (video.content.x, video.content.y),
                    (canvas.origin_x, canvas.origin_y)
                );
                assert_eq!(
                    (video.content.width, video.content.height),
                    (content.frame_width, content.frame_height)
                );
                assert_eq!(
                    video.playfield_left,
                    content.playfield_left + canvas.origin_x as f64
                );
                assert_eq!(
                    video.playfield_top,
                    content.playfield_top + canvas.origin_y as f64
                );
                // 缩放与物件尺寸都不随画布变化。
                assert_eq!(video.playfield_scale, content.playfield_scale);
                assert_eq!(video.object_scale, content.object_scale);
                assert_eq!(video.pixels_per_ms, content.pixels_per_ms);
                assert_eq!(
                    object_diameter(video.object_scale, video.playfield_scale, 1.0),
                    object_diameter(content.object_scale, content.playfield_scale, 1.0)
                );
                assert_ne!(video.frame_width, content.frame_width);
            });
        }
    }

    #[test]
    fn video_layout_allows_fruit_beyond_content_box() {
        let layout =
            build_video_animation_layout(4.0, 5.0, crate::render::geometry::OutputFormat::Mp4);
        let start_time = 5_000;
        let objects = vec![RenderObject {
            object_type: ObjType::Fruit,
            x: 0.0,
            start_time,
            color: [255, 0, 128],
            scale_factor: 1.0,
            event_time: None,
            hyper_dash: true,
            edge: false,
            banana_shower_id: None,
            banana_route_x: None,
        }];
        let frame = render_animation_frame(&objects, &[start_time], start_time, &layout, None);

        let centre_x = layout.playfield_left;
        let radius = object_diameter(layout.object_scale, layout.playfield_scale, 1.0) / 2.0;
        let hyper_dash_outer = radius * 1.6;
        assert!(
            centre_x - hyper_dash_outer < layout.content.x as f64,
            "测试前提：hyperdash 外环必须越过内容框"
        );
        let judge_y = layout.playfield_top
            + crate::render::cpu::modes::catch::constants::STABLE_CATCHER_Y
                * layout.playfield_scale;
        let mut painted_left_of_content = false;
        for x in (layout.content.x - 30)..layout.content.x {
            for y in (judge_y.round() as i64 - 12)..=(judge_y.round() as i64 + 12) {
                if x >= 0 && y >= 0 && frame.get(x as u32, y as u32)[3] > 0 {
                    painted_left_of_content = true;
                }
            }
        }
        assert!(
            painted_left_of_content,
            "内容框左侧仍应画出水果外环（此前会被内容框裁掉一半）"
        );
        // 补边本身保持透明，让合成阶段的画布底色透出来。
        assert_eq!(frame.get(0, 0)[3], 0);
    }
}
