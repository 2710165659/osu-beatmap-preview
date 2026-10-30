//! osu!mania 动画渲染器：网格 GIF 与单画面视频的下落式音符预览。
//! 移植自 beatmap_preview/mania/gif_renderer.py。

use crate::render::cpu::modes::mania::constants as mania_constants;

use crate::domain::errors::{PreviewError, Result};
use crate::domain::models::{Beatmap, ManiaHitObject, TimingPoint};
use crate::domain::mods::ModSettings;
use crate::domain::parser::round_half_even;
use crate::domain::shared::time_selection::{GifRenderOptions, TimeAxis};
use crate::domain::timeout::RequestDeadline;
use crate::render::canvas::{Img, Rgba};
use crate::render::cpu::AnimationFrames;
use crate::render::text::{draw_text, render_text_sprite, text_size};
use crate::render::visibility::{Flashlight, FlashlightKind, ManiaHidden, VisibilityTimeline};

use super::{
    apply_hold_off_mod, apply_inverse_mod, apply_mirror_mod, build_sv_changes, darken,
    format_sv_label, is_native_mania, mania_objects, resolve_key_count,
};

use super::hit_flash::HitFlashes;
use super::skin::load_mania_skin_config;

pub struct AnimationLayout {
    pub segment_width: i64,
    pub playfield_height: i64,
    pub lane_area_width: i64,
    pub image_width: i64,
    pub image_height: i64,
    pub hit_position_y: i64,
    pub scroll_length: i64,
    pub note_head_height: i64,
    pub column_left_offsets: Vec<i64>,
    pub column_widths: Vec<i64>,
    pub playfield_left: i64,
    pub playfield_top: i64,
    pub left_panel_width: i64,
    pub note_side_padding: i64,
    pub sv_info_margin_left: i64,
    pub sv_label_inset: i64,
    pub sv_font_size: u32,
    pub sv_text_color: Rgba,
    pub render_scale: f64,
    /// 单帧内的舞台内容框；GIF 拼格在完整帧绘制后进行。
    /// 视频物件层的帧就是最终画布，底色只填这块区域，物件允许溢出到画布边缘。
    pub content: crate::render::geometry::PixelRect,
    pub lane_background: Rgba,
    pub column_background: Rgba,
    pub left_panel_background: Rgba,
    pub judgement_line_color: Rgba,
}

pub fn mania_visibility_timeline(
    objects: &[ManiaHitObject],
    breaks: &[crate::domain::models::BreakPeriod],
) -> VisibilityTimeline {
    let mut hit_times = Vec::with_capacity(objects.len() * 2);
    for object in objects {
        hit_times.push(object.start_time as f64);
        if object.is_long_note {
            hit_times.push(object.end_time as f64);
        }
    }
    VisibilityTimeline::new(hit_times, breaks)
}

pub fn mania_hidden(
    timeline: &VisibilityTimeline,
    time: i64,
    layout: &AnimationLayout,
) -> ManiaHidden {
    ManiaHidden::new(
        timeline,
        time,
        layout.playfield_top,
        layout.playfield_top + layout.hit_position_y,
        layout.render_scale * 0.5,
    )
}

pub fn mania_flashlight(
    timeline: &VisibilityTimeline,
    time: i64,
    layout: &AnimationLayout,
    left: i64,
) -> Flashlight {
    Flashlight {
        center: [
            (left + layout.segment_width / 2) as f64,
            (layout.playfield_top + layout.playfield_height / 2) as f64,
        ],
        radius: 50.0
            * layout.render_scale
            * 0.5
            * timeline.flashlight_scale(FlashlightKind::Mania, time),
        band: true,
        dim: 0.0,
    }
}

/// 将谱面时间映射为连续滚动距离，同时处理 BPM 和 SV 变化。
pub struct ScrollMap {
    starts: Vec<f64>,
    positions: Vec<f64>,
    multipliers: Vec<f64>,
}

impl ScrollMap {
    pub fn position_at(&self, time: f64) -> f64 {
        let index = self
            .starts
            .partition_point(|s| *s <= time)
            .saturating_sub(1);
        self.positions[index] + (time - self.starts[index]) * self.multipliers[index]
    }
}

pub fn prepare_mania_gif_frames(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    options: GifRenderOptions,
    fps: Option<u32>,
    deadline: &RequestDeadline,
) -> Result<AnimationFrames> {
    deadline.check()?;
    let key_count = resolve_key_count(beatmap)?;
    let palette = super::lane_palette(key_count);
    let original_objects = mania_objects(beatmap);
    let mut hit_objects = original_objects.clone();
    if mods.is_some_and(|m| m.inverse) {
        hit_objects = apply_inverse_mod(&hit_objects, &beatmap.timing_points);
    }
    if mods.is_some_and(|m| m.hold_off) {
        hit_objects = apply_hold_off_mod(&hit_objects);
    }
    if mods.is_some_and(|m| m.mirror) {
        apply_mirror_mod(&mut hit_objects, key_count);
    }
    let cs_mode = mods.is_some_and(|m| m.cs_override);
    let hidden = mods.is_some_and(|mods| mods.hidden);
    let flashlight = mods.is_some_and(|mods| mods.flashlight);
    let visibility = (hidden || flashlight)
        .then(|| mania_visibility_timeline(&hit_objects, &beatmap.break_periods));
    if hit_objects.is_empty() {
        return Err(PreviewError::render("mania beatmap has no hit objects"));
    }

    // DT/HT 只改变谱面时间推进速度；GIF 每段的播放时长由参数或配置决定。
    let speed_multiplier = mods.map_or(1.0, |m| m.speed_multiplier);
    let fps = fps
        .map(f64::from)
        .unwrap_or(crate::config::current().render.mania.gif.style.FPS as f64);
    let (times_ms, duration_seconds, time_axis) = options.into_parts();
    let (segment_timings, segment_duration, frame_count, show_time_label, time_axis) = {
        let config = crate::config::current();
        let gif_config = &config.render.mania.gif;
        let segment_duration_ms = duration_seconds
            .map(|seconds| seconds * 1000.0)
            .unwrap_or(crate::config::current().render.mania.gif.style.DURATION_MS as f64);
        let gameplay_segment_duration = round_half_even(segment_duration_ms * speed_multiplier);
        let spans: Vec<(i64, i64)> = hit_objects
            .iter()
            .map(|ho| (ho.start_time, ho.end_time))
            .collect();
        let segment_timings = times_ms.choose(
            beatmap,
            spans,
            gif_config
                .structure
                .ROW_COUNT
                .checked_mul(gif_config.structure.IMAGES_PER_ROW)
                .filter(|&count| count > 0)
                .ok_or_else(|| PreviewError::render("preview segment capacity must be positive"))?,
            gameplay_segment_duration,
        )?;
        let frame_count = round_half_even(segment_duration_ms * fps / 1000.0).max(1);
        (
            segment_timings,
            gameplay_segment_duration,
            frame_count,
            mania_constants::gif::style::SHOW_TIME_LABEL,
            time_axis,
        )
    };

    let skin_config = load_mania_skin_config(key_count, crate::render::geometry::OutputFormat::Gif);
    let layout = build_layout(&skin_config, crate::render::geometry::OutputFormat::Gif);
    let native_mania = is_native_mania(beatmap);
    let hit_flashes = crate::config::current()
        .render
        .mania
        .gif
        .style
        .SHOW_HIT_FLASH
        .then(|| HitFlashes::new(&hit_objects, &palette, &layout));
    // CS 是恒定滚动：保留 33 速时间窗口，但跳过 SV 倍率。
    let scroll_map = build_scroll_map(beatmap, &original_objects, cs_mode, native_mania);
    // time_range 是 33 速下从判定线到顶部可见的谱面时间跨度。
    let time_range = compute_time_range(
        speed_multiplier,
        skin_config.hit_position,
        crate::config::current().render.mania.gif.style.SCROLL_SPEED,
    );
    let pixels_per_scroll_unit = layout.scroll_length as f64 / time_range;
    let max_segment_end = segment_timings
        .iter()
        .map(|t| t.start_time + segment_duration)
        .max()
        .unwrap_or(0);
    let sv_changes = if cs_mode
        || !native_mania
        || !crate::config::current()
            .render
            .mania
            .gif
            .style
            .SHOW_SV_LABEL
    {
        Vec::new()
    } else {
        build_sv_changes(
            &beatmap.timing_points,
            max_segment_end + round_half_even(time_range),
        )
    };

    let segment_snapshot_times: Vec<Vec<i64>> = segment_timings
        .iter()
        .map(|timing| {
            (0..frame_count)
                .map(|frame_index| {
                    timing.start_time
                        + round_half_even(frame_index as f64 * 1000.0 * speed_multiplier / fps)
                })
                .collect()
        })
        .collect();

    let hold_colors: Vec<Rgba> = palette.iter().map(|&c| darken(c, 0.5)).collect();

    // 预计算每个音符的滚动距离位置，供排序后的二分查找裁剪使用：position_at 随时间
    // 严格单调、hit_objects 按 start_time 排序，因此 `pos_start` 升序。可变 SV 下必须
    // 按滚动距离而不是谱面时间裁剪——时间与位置非线性，慢 SV 会把很长的谱面时间压缩
    // 到少量屏幕像素，按时间窗口会误删可见音符。
    let pos_start: Vec<f64> = hit_objects
        .iter()
        .map(|ho| scroll_map.position_at(ho.start_time as f64))
        .collect();
    let pos_end: Vec<f64> = hit_objects
        .iter()
        .map(|ho| scroll_map.position_at(ho.end_time as f64))
        .collect();
    // 计算滚动距离空间中最宽的长按主体，并从下界减去该值，
    // 防止头部已远离但主体仍在屏幕内的长按被跳过；单点贡献 0。
    let max_hold_position: f64 = pos_start
        .iter()
        .zip(&pos_end)
        .map(|(&start, &end)| (end - start).max(0.0))
        .fold(0.0_f64, f64::max);

    let (canvas_width, canvas_height) = gif_canvas_size(&layout, show_time_label);
    if !(1..=i64::from(u16::MAX)).contains(&canvas_width)
        || !(1..=i64::from(u16::MAX)).contains(&canvas_height)
    {
        return Err(PreviewError::render("Mania GIF 画布尺寸超出格式范围"));
    }
    let origins: Vec<_> = (0..segment_timings.len())
        .map(|index| gif_frame_origin(index, &layout, show_time_label))
        .collect();
    let static_bg = {
        let mut frame = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            mania_constants::gif::style::IMAGE_BACKGROUND,
        );
        draw_segment_background(&mut frame, layout.playfield_left, &layout);
        frame
    };

    // 每段的时间标签（及 "Preview Time" 提示）只预渲染一次为精灵图。
    // 段内文字恒定，可避免每段 150 次 format!、text_size、draw_text 调用；
    // 每帧只需合成预构建精灵图。
    let label_y = layout.image_height + mania_constants::gif::sizing().TIME_LABEL_TOP_GAP;
    let pre_labels: Vec<PreLabel> = if show_time_label {
        segment_timings
            .iter()
            .enumerate()
            .map(|(si, st)| {
                let (x, y) = origins[si];
                build_pre_label(st, segment_duration, &layout, x, y + label_y, time_axis)
            })
            .collect()
    } else {
        Vec::new()
    };

    // 预渲染 SV 标签精灵图：format_sv_label 涉及 String 分配且文字不变，
    // 每帧只有 y 位置发生滚动。
    let sv_sprites: Vec<(f64, Img)> = sv_changes
        .iter()
        .map(|&(time, sv)| {
            (
                scroll_map.position_at(time as f64),
                render_text_sprite(
                    &format_sv_label(sv),
                    layout.sv_font_size,
                    layout.sv_text_color,
                ),
            )
        })
        .collect();

    // 每格独立绘制完整帧，遮罩和叠加层不会跨格；后续回放可复用同一拼格入口。
    let render_frame = move |frame_index: usize| -> Img {
        let mut canvas = Img::new(
            canvas_width as u32,
            canvas_height as u32,
            mania_constants::gif::style::IMAGE_BACKGROUND,
        );
        for (segment_index, _) in segment_timings.iter().enumerate() {
            let mut frame = static_bg.clone();
            let mut notes = hidden.then(|| Img::new(frame.w, frame.h, [0, 0, 0, 0]));
            let seg_left = layout.playfield_left;
            let snapshot_time = segment_snapshot_times[segment_index][frame_index];
            let snapshot_pos = scroll_map.position_at(snapshot_time as f64);
            draw_gif_sv_indicators_fast(
                &mut frame,
                &sv_sprites,
                seg_left,
                snapshot_pos,
                &layout,
                pixels_per_scroll_unit,
            );
            // 二分查找预计算的滚动距离位置，避免扫描全部音符。
            // 按距离而非谱面时间裁剪，在可变 SV 下仍然正确；
            // draw_gif_hit_object 内仍执行 y 裁剪以保证像素级精确。
            let (lo_pos, hi_pos) = visible_pos_window(
                snapshot_pos,
                &layout,
                pixels_per_scroll_unit,
                max_hold_position,
            );
            let start_idx = pos_start.partition_point(|&p| p < lo_pos);
            for idx in start_idx..hit_objects.len() {
                if pos_start[idx] > hi_pos {
                    break;
                }
                draw_gif_hit_object(
                    notes.as_mut().unwrap_or(&mut frame),
                    &hit_objects[idx],
                    &palette,
                    &hold_colors,
                    seg_left,
                    pos_start[idx],
                    pos_end[idx],
                    snapshot_pos,
                    &layout,
                    pixels_per_scroll_unit,
                );
            }
            if let Some(flashes) = &hit_flashes {
                flashes.draw(&mut frame, snapshot_time, seg_left, &layout);
            }
            if let Some(timeline) = &visibility {
                let rect = crate::render::geometry::PixelRect {
                    x: seg_left,
                    y: layout.playfield_top,
                    width: layout.segment_width,
                    height: layout.playfield_height,
                };
                if let Some(notes) = &mut notes {
                    mania_hidden(timeline, snapshot_time, &layout).apply(notes, rect);
                }
            }
            if let Some(notes) = notes {
                frame.alpha_composite(&notes, 0, 0);
            }
            if flashlight {
                if let Some(timeline) = &visibility {
                    mania_flashlight(timeline, snapshot_time, &layout, seg_left).apply(
                        &mut frame,
                        crate::render::geometry::PixelRect {
                            x: 0,
                            y: 0,
                            width: layout.image_width,
                            height: layout.image_height,
                        },
                    );
                }
            }
            let (x, y) = origins[segment_index];
            canvas.alpha_composite(&frame, x, y);
            if show_time_label {
                // 贴入预渲染时间标签精灵图，避免每帧调用 format!/text_size。
                let pl = &pre_labels[segment_index];
                canvas.alpha_composite(&pl.sprite, pl.x, pl.y);
                if let Some(ref note) = pl.note {
                    canvas.alpha_composite(&note.sprite, note.x, note.y);
                }
            }
        }
        canvas
    };

    Ok(AnimationFrames::with_frame_rate(
        frame_count as usize,
        fps,
        render_frame,
    ))
}

pub fn build_layout(
    skin_config: &super::skin::ManiaSkinConfig,
    output_format: crate::render::geometry::OutputFormat,
) -> AnimationLayout {
    let render_scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Mania,
        output_format,
    );
    let (column_widths, column_left_offsets, lane_area_width) = build_scaled_columns(
        &skin_config.column_widths,
        &skin_config.column_line_widths,
        render_scale,
    );
    let left_panel_width = crate::render::geometry::scale_px(
        crate::render::geometry::scale_px(
            crate::render::cpu::modes::mania::constants::LEFT_PANEL_WIDTH as f64,
            0.5,
        ) as f64,
        render_scale,
    );
    let note_side_padding = crate::render::geometry::scale_px(
        crate::render::geometry::scale_px(
            crate::render::cpu::modes::mania::constants::NOTE_SIDE_PADDING as f64,
            0.5,
        ) as f64,
        render_scale,
    );
    let segment_width = left_panel_width * 2 + lane_area_width;
    let playfield_height = crate::render::geometry::scale_px(
        crate::render::geometry::MANIA_PLAYFIELD_HEIGHT,
        crate::render::geometry::output_scale(
            crate::render::geometry::GameMode::Mania,
            output_format,
        ),
    );
    let logical_hit_position = round_half_even(skin_config.hit_position * 0.5);
    let hit_position_y = playfield_height
        - crate::render::geometry::scale_px(logical_hit_position as f64, render_scale);
    let scroll_length = (hit_position_y
        - crate::render::geometry::scale_px(
            crate::render::geometry::scale_px(
                crate::render::cpu::modes::mania::constants::STAGE_TOP_PADDING as f64,
                0.5,
            ) as f64,
            render_scale,
        ))
    .max(1);
    let average_column_width =
        column_widths.iter().sum::<i64>() as f64 / column_widths.len() as f64;
    // PNG 使用 38px 轨道和 15px 音符；GIF 音符高度随皮肤缩放。
    // 随列宽缩放，避免宽列中的音符显得被压扁。
    let note_head_height = round_half_even(
        crate::render::cpu::modes::mania::constants::NOTE_HEAD_HEIGHT as f64 * average_column_width
            / crate::render::cpu::modes::mania::constants::LANE_WIDTH as f64,
    )
    .max(1);
    let (
        image_width,
        image_height,
        playfield_left,
        playfield_top,
        sv_info_margin_left,
        sv_font_size,
        sv_text_color,
        unit_width,
    ) = if output_format == crate::render::geometry::OutputFormat::Gif {
        let sizing = mania_constants::gif::sizing();
        let info_left = if crate::config::current()
            .render
            .mania
            .gif
            .style
            .SHOW_SV_LABEL
        {
            sizing.INFO_MARGIN_LEFT
        } else {
            0
        };
        let unit_width = info_left + segment_width + sizing.INFO_MARGIN_RIGHT;
        let (frame_width, frame_height) =
            crate::render::geometry::preview_frame_size(crate::render::geometry::output_scale(
                crate::render::geometry::GameMode::Mania,
                output_format,
            ));
        // 宽键数皮肤保留原始轨道尺寸；完整帧至少容纳舞台，不在绘制后拉伸。
        let frame_width = frame_width.max(unit_width);
        (
            frame_width,
            frame_height,
            (frame_width - unit_width) / 2 + info_left,
            0,
            info_left,
            sizing.SV_TEXT_FONT_SIZE,
            mania_constants::gif::style::SV_TEXT_COLOR,
            unit_width,
        )
    } else {
        let sizing = mania_constants::mp4::sizing();
        let unit_width = sizing.INFO_MARGIN_LEFT + segment_width + sizing.INFO_MARGIN_RIGHT;
        (
            unit_width,
            playfield_height,
            sizing.INFO_MARGIN_LEFT,
            0,
            sizing.INFO_MARGIN_LEFT,
            sizing.SV_TEXT_FONT_SIZE,
            mania_constants::mp4::style::SV_TEXT_COLOR,
            unit_width,
        )
    };
    let (lane_background, column_background, left_panel_background, judgement_line_color) =
        match output_format {
            crate::render::geometry::OutputFormat::Gif => (
                mania_constants::gif::style::LANE_BACKGROUND,
                crate::render::cpu::modes::mania::constants::MANIA_COLUMN_BACKGROUND,
                mania_constants::gif::style::LEFT_PANEL_BACKGROUND,
                mania_constants::gif::style::JUDGEMENT_LINE_COLOR,
            ),
            crate::render::geometry::OutputFormat::Mp4 => {
                let config = &crate::config::current().render.mania.mp4;
                let column_background = with_alpha(
                    crate::render::cpu::modes::mania::constants::MANIA_COLUMN_BACKGROUND,
                    config.style.LANE_DARKEN_ALPHA,
                );
                (
                    column_background,
                    column_background,
                    crate::render::cpu::modes::mania::constants::MP4_LEFT_PANEL_BACKGROUND,
                    crate::render::cpu::modes::mania::constants::MP4_JUDGEMENT_LINE_COLOR,
                )
            }
            crate::render::geometry::OutputFormat::Png => unreachable!("PNG 不使用动画布局"),
        };
    AnimationLayout {
        segment_width,
        playfield_height,
        lane_area_width,
        image_width,
        image_height,
        hit_position_y,
        scroll_length,
        note_head_height,
        column_left_offsets,
        column_widths,
        playfield_left,
        playfield_top,
        left_panel_width,
        note_side_padding,
        sv_info_margin_left,
        sv_label_inset: crate::render::geometry::scale_px(3.0, render_scale),
        sv_font_size,
        sv_text_color,
        render_scale,
        content: crate::render::geometry::PixelRect {
            x: playfield_left - sv_info_margin_left,
            y: playfield_top,
            width: unit_width,
            height: playfield_height,
        },
        lane_background,
        column_background,
        left_panel_background,
        judgement_line_color,
    }
}

/// MP4 物件层布局：帧尺寸 = 视频画布，单段舞台按 [`video_canvas`] 居中。
///
/// 物件层与最终画布同尺寸后，音符只会在视频边界被裁剪，不会再被内容框切掉一半；
/// 键道宽度、滚动长度与音符尺寸完全沿用内容框布局，因此分辨率与物件大小都不变。
pub fn build_video_layout(
    skin_config: &super::skin::ManiaSkinConfig,
    output_format: crate::render::geometry::OutputFormat,
) -> AnimationLayout {
    let layout = build_layout(skin_config, output_format);
    let content_width = layout.content.width;
    let content_height = layout.playfield_height;
    let canvas = crate::render::geometry::video_canvas(crate::render::geometry::PixelRect {
        x: 0,
        y: 0,
        width: content_width,
        height: content_height,
    });
    AnimationLayout {
        image_width: canvas.width as i64,
        image_height: canvas.height as i64,
        playfield_left: layout.playfield_left + canvas.origin_x,
        playfield_top: layout.playfield_top + canvas.origin_y,
        content: crate::render::geometry::PixelRect {
            x: canvas.origin_x,
            y: canvas.origin_y,
            width: content_width,
            height: content_height,
        },
        ..layout
    }
}

/// 将 0..=1 的配置透明度转换为黑色轨道叠加层的 alpha 字节。
fn with_alpha(mut colour: Rgba, alpha: f64) -> Rgba {
    colour[3] = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    colour
}

pub fn build_scaled_columns(
    source_widths: &[i64],
    source_line_widths: &[i64],
    output_scale: f64,
) -> (Vec<i64>, Vec<i64>, i64) {
    // 皮肤宽度基于 2x 坐标，先固化为 1x 逻辑边界，再应用输出倍率。
    // 若直接按 0.5 * SCALE 缩放奇数宽度，误差会在每个时间列重复累积。
    let mut widths = Vec::with_capacity(source_widths.len());
    let mut offsets = Vec::with_capacity(source_widths.len());
    let mut cursor = source_line_widths.first().copied().unwrap_or(0).max(0);
    for (index, &source_width) in source_widths.iter().enumerate() {
        let logical_left = crate::render::geometry::scale_px(cursor as f64, 0.5);
        let left = crate::render::geometry::scale_px(logical_left as f64, output_scale);
        cursor += source_width.max(0);
        let logical_right = crate::render::geometry::scale_px(cursor as f64, 0.5);
        let right = crate::render::geometry::scale_px(logical_right as f64, output_scale);
        offsets.push(left);
        widths.push((right - left).max(1));
        if index + 1 < source_line_widths.len() {
            cursor += source_line_widths[index + 1].max(0);
        }
    }
    let logical_total = crate::render::geometry::scale_px(cursor as f64, 0.5);
    let total_width = crate::render::geometry::scale_px(logical_total as f64, output_scale).max(1);
    (widths, offsets, total_width)
}

/// 对应 DrawableManiaRuleset.updateTimeRange()：根据 HitPosition 调整 33 速基础窗口。
pub fn compute_time_range(speed_multiplier: f64, hit_position: f64, scroll_speed: f64) -> f64 {
    let hit_position_scale = (crate::render::cpu::modes::mania::constants::FRAME_HEIGHT as f64
        - hit_position)
        / (crate::render::cpu::modes::mania::constants::FRAME_HEIGHT as f64
            - crate::render::cpu::modes::mania::constants::DEFAULT_HIT_POSITION_FROM_BOTTOM);
    (crate::render::cpu::modes::mania::constants::BASE_TIME_RANGE_MS / scroll_speed
        * hit_position_scale
        * speed_multiplier)
        .max(1.0)
}

pub fn build_scroll_map(
    beatmap: &Beatmap,
    hit_objects: &[ManiaHitObject],
    constant: bool,
    allow_sv: bool,
) -> ScrollMap {
    if constant {
        return ScrollMap {
            starts: vec![0.0],
            positions: vec![0.0],
            multipliers: vec![1.0],
        };
    }

    let timing_points = &beatmap.timing_points;
    let mut starts: Vec<f64> = Vec::new();
    let mut multipliers: Vec<f64> = Vec::new();
    let base_beat_length = most_common_beat_length(timing_points, hit_objects);
    let mut current_beat_length = base_beat_length;
    let mut current_scroll_speed;

    for point in timing_points {
        if point.uninherited {
            current_beat_length = point.beat_length;
            current_scroll_speed = 1.0;
        } else if allow_sv && point.beat_length < 0.0 {
            // 绿线 beat_length 为负；osu! 使用 -100 / beat_length 编码 SV。
            current_scroll_speed = -100.0 / point.beat_length;
        } else {
            continue;
        }
        starts.push(point.time);
        multipliers.push(current_scroll_speed * base_beat_length / current_beat_length);
    }

    if starts.is_empty() {
        starts.push(0.0);
        multipliers.push(1.0);
    } else if starts[0] > 0.0 {
        starts.insert(0, 0.0);
        multipliers.insert(0, multipliers[0]);
    }

    let mut positions = vec![0.0];
    for index in 1..starts.len() {
        positions.push(
            positions[index - 1] + (starts[index] - starts[index - 1]) * multipliers[index - 1],
        );
    }
    ScrollMap {
        starts,
        positions,
        multipliers,
    }
}

fn most_common_beat_length(timing_points: &[TimingPoint], hit_objects: &[ManiaHitObject]) -> f64 {
    let red_lines: Vec<&TimingPoint> = timing_points
        .iter()
        .filter(|p| p.uninherited && p.beat_length > 0.0)
        .collect();
    if red_lines.is_empty() {
        return 500.0;
    }

    let last_time = if hit_objects.is_empty() {
        red_lines.last().unwrap().time
    } else {
        hit_objects.iter().map(|ho| ho.end_time).max().unwrap() as f64
    };

    let mut buckets: Vec<(i64, f64)> = Vec::new();
    for (index, point) in red_lines.iter().enumerate() {
        let duration = if point.time > last_time {
            0.0
        } else {
            let current_time = if index == 0 { 0.0 } else { point.time };
            let next_time = if index == red_lines.len() - 1 {
                last_time
            } else {
                red_lines[index + 1].time
            };
            (next_time - current_time).max(0.0)
        };

        let key = round_half_even(point.beat_length * 1000.0);
        match buckets.iter_mut().find(|(k, _)| *k == key) {
            Some((_, total)) => *total += duration,
            None => buckets.push((key, duration)),
        }
    }

    let mut most_common = buckets[0];
    for &bucket in &buckets[1..] {
        if bucket.1 > most_common.1 {
            most_common = bucket;
        }
    }
    let most_common = most_common.0 as f64 / 1000.0;
    let min_beat_length = red_lines
        .iter()
        .map(|p| p.beat_length)
        .fold(f64::MAX, f64::min);
    let max_beat_length = red_lines
        .iter()
        .map(|p| p.beat_length)
        .fold(f64::MIN, f64::max);
    most_common.min(max_beat_length).max(min_beat_length)
}

fn gif_unit_size(layout: &AnimationLayout, show_time_label: bool) -> (i64, i64) {
    let sizing = mania_constants::gif::sizing();
    let label_height = if show_time_label {
        sizing.TIME_LABEL_TOP_GAP + sizing.INFO_MARGIN_BOTTOM
    } else {
        0
    };
    (layout.image_width, layout.image_height + label_height)
}

/// 完整帧网格画布；保留配置容量，短谱不足的片段留空。
pub fn gif_canvas_size(layout: &AnimationLayout, show_time_label: bool) -> (i64, i64) {
    let config = crate::config::current();
    let grid = &config.render.mania.gif.structure;
    let sizing = mania_constants::gif::sizing();
    let (width, height) = gif_unit_size(layout, show_time_label);
    (
        sizing.PAGE_MARGIN_LEFT
            + sizing.PAGE_MARGIN_RIGHT
            + grid.IMAGES_PER_ROW as i64 * width
            + (grid.IMAGES_PER_ROW as i64 - 1) * sizing.GRID_GAP,
        sizing.PAGE_MARGIN_TOP
            + sizing.PAGE_MARGIN_BOTTOM
            + grid.ROW_COUNT as i64 * height
            + (grid.ROW_COUNT as i64 - 1) * sizing.GRID_GAP,
    )
}

/// 单格完整帧的左上角；时间标签独立放在帧下方。
pub fn gif_frame_origin(
    index: usize,
    layout: &AnimationLayout,
    show_time_label: bool,
) -> (i64, i64) {
    let columns = crate::config::current()
        .render
        .mania
        .gif
        .structure
        .IMAGES_PER_ROW;
    let sizing = mania_constants::gif::sizing();
    let (width, height) = gif_unit_size(layout, show_time_label);
    (
        sizing.PAGE_MARGIN_LEFT + (index % columns) as i64 * (width + sizing.GRID_GAP),
        sizing.PAGE_MARGIN_TOP + (index / columns) as i64 * (height + sizing.GRID_GAP),
    )
}

pub fn draw_segment_background(canvas: &mut Img, seg_left: i64, layout: &AnimationLayout) {
    // GIF 不绘制小节线、节拍线和轨道分隔线，只保留灰色侧板与判定线。
    let playfield_top = layout.playfield_top;
    let lane_area_left = seg_left + layout.left_panel_width;
    let lane_area_right = lane_area_left + layout.lane_area_width;

    canvas.set_rect_size(
        seg_left,
        playfield_top,
        layout.segment_width,
        layout.playfield_height,
        layout.lane_background,
    );
    canvas.set_rect_size(
        seg_left,
        playfield_top,
        layout.left_panel_width,
        layout.playfield_height,
        layout.left_panel_background,
    );
    canvas.set_rect_size(
        lane_area_right,
        playfield_top,
        layout.left_panel_width,
        layout.playfield_height,
        layout.left_panel_background,
    );

    for (lane_index, &lane_width) in layout.column_widths.iter().enumerate() {
        let lane_left = lane_area_left + layout.column_left_offsets[lane_index];
        canvas.set_rect_size(
            lane_left,
            playfield_top,
            lane_width,
            layout.playfield_height,
            layout.column_background,
        );
    }

    let judgement_y = playfield_top + layout.hit_position_y;
    canvas.draw_line(
        seg_left as f64,
        judgement_y as f64,
        (seg_left + layout.segment_width) as f64,
        judgement_y as f64,
        (2.0 * layout.render_scale).max(1.0),
        layout.judgement_line_color,
    );
}

pub fn draw_gif_sv_indicators(
    canvas: &mut Img,
    sv_changes: &[(i64, f64)],
    sv_positions: &[f64],
    seg_left: i64,
    snapshot_pos: f64,
    layout: &AnimationLayout,
    pixels_per_scroll_unit: f64,
) {
    // SV 文字位于左侧灰色面板附近，只标记变化点，不绘制线条。
    debug_assert_eq!(sv_changes.len(), sv_positions.len());
    let (lo_pos, hi_pos) = visible_pos_window(snapshot_pos, layout, pixels_per_scroll_unit, 0.0);
    let start = sv_positions.partition_point(|&pos| pos < lo_pos);
    for index in start..sv_changes.len() {
        let position = sv_positions[index];
        if position > hi_pos {
            break;
        }
        let (_, sv) = sv_changes[index];
        let y = y_at_position(position, snapshot_pos, layout, pixels_per_scroll_unit);
        if y < layout.playfield_top || y > layout.playfield_top + layout.playfield_height {
            continue;
        }
        let label = format_sv_label(sv);
        let (_, label_h) = text_size(&label, layout.sv_font_size);
        let x = (seg_left - layout.sv_info_margin_left + layout.sv_label_inset).max(0);
        let label_y = (y as f64 - label_h as f64 / 2.0).floor() as i64;
        draw_text(
            canvas,
            x,
            label_y,
            &label,
            layout.sv_font_size,
            layout.sv_text_color,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_gif_hit_object(
    canvas: &mut Img,
    hit_object: &ManiaHitObject,
    palette: &[Rgba],
    hold_colors: &[Rgba],
    seg_left: i64,
    start_pos: f64,
    end_pos: f64,
    snapshot_pos: f64,
    layout: &AnimationLayout,
    pixels_per_scroll_unit: f64,
) {
    let y_start = y_at_position(start_pos, snapshot_pos, layout, pixels_per_scroll_unit);
    let y_end = y_at_position(end_pos, snapshot_pos, layout, pixels_per_scroll_unit);
    let playfield_top = layout.playfield_top;
    let playfield_bottom = playfield_top + layout.playfield_height;
    if y_start.max(y_end) < playfield_top - layout.note_head_height
        || y_start.min(y_end) > playfield_bottom + layout.note_head_height
    {
        return;
    }

    let lane = (hit_object.lane.max(0) as usize).min(layout.column_widths.len() - 1);
    let lane_color = palette[lane.min(palette.len() - 1)];
    // 长按主体保持 PNG 外观（变暗的轨道颜色），不受轨道配置影响。
    let hold_color = hold_colors[lane.min(hold_colors.len() - 1)];
    let lane_left = seg_left
        + layout.left_panel_width
        + layout.column_left_offsets[lane]
        + layout.note_side_padding;
    let lane_right = lane_left + layout.column_widths[lane] - layout.note_side_padding * 2;

    if hit_object.is_long_note {
        let body_top = playfield_top.max(y_end.min(y_start - layout.note_head_height));
        let body_bottom = playfield_bottom.min(y_start);
        if body_top < body_bottom {
            canvas.set_rect_size(
                lane_left,
                body_top,
                lane_right - lane_left,
                body_bottom - body_top,
                hold_color,
            );
        }
    }

    let head_top = playfield_top.max(y_start - layout.note_head_height);
    let head_bottom = playfield_bottom.min(y_start);
    if head_top < head_bottom {
        canvas.set_rect_size(
            lane_left,
            head_top,
            lane_right - lane_left,
            head_bottom - head_top,
            lane_color,
        );
    }
}

/// 与 `y_at_time` 相同，但接收预计算的 `snapshot_pos`，避免每次调用都对
/// `position_at(snapshot_time)` 二分查找。一帧包含数千音符时可消除数千次重复查找。
#[inline]
fn y_at_position(
    object_pos: f64,
    snapshot_pos: f64,
    layout: &AnimationLayout,
    pixels_per_scroll_unit: f64,
) -> i64 {
    let distance = object_pos - snapshot_pos;
    layout.playfield_top + layout.hit_position_y
        - round_half_even(distance * pixels_per_scroll_unit)
}

/// 滚动距离窗口 `[lo, hi]`，窗口外不可能有可见音符；用它二分查找预计算的
/// `pos_start` 数组，避免每帧扫描全部音符。
///
/// 单位是滚动距离（position_at）而非谱面时间：可变 SV 下时间与位置非线性，基于
/// 时间的窗口会丢弃屏幕内音符。`lo`/`hi` 覆盖游戏区域并额外留出 `note_head_height`，
/// 下界再减去 `max_hold_position`（距离空间中最宽的长按主体），保证 `end_time` 仍
/// 在屏幕内的长按不被截断。`draw_gif_hit_object` 内部仍做 y 裁剪保证像素精度。
#[inline]
pub fn visible_pos_window(
    snapshot_pos: f64,
    layout: &AnimationLayout,
    pixels_per_scroll_unit: f64,
    max_hold_position: f64,
) -> (f64, f64) {
    // 最远可见的未来音符头位于 y = playfield_top - note_head_height。
    // 即判定线上方距离为 hit_position_y + note_head_height。
    let future_dist =
        (layout.hit_position_y + layout.note_head_height) as f64 / pixels_per_scroll_unit;
    // 最远可见的过去音符位于 y = playfield_bottom + note_head_height，
    // 即 distance = -(playfield_height - hit_position_y + note_head_height)。
    let past_dist = (layout.playfield_height - layout.hit_position_y + layout.note_head_height)
        as f64
        / pixels_per_scroll_unit;
    (
        snapshot_pos - past_dist - max_hold_position,
        snapshot_pos + future_dist,
    )
}

/// 预渲染的时间标签精灵图及贴图位置，每段只构建一次。
struct PreLabel {
    sprite: Img,
    x: i64,
    y: i64,
    note: Option<Box<PreLabel>>,
}

fn build_pre_label(
    timing: &crate::domain::shared::time_selection::PreviewSegmentTiming,
    duration_ms: i64,
    layout: &AnimationLayout,
    seg_left: i64,
    y: i64,
    time_axis: TimeAxis,
) -> PreLabel {
    let mania_gif_sizing = mania_constants::gif::sizing();
    let label = format!(
        "{} - {}",
        crate::render::text::format_mmss_floor(time_axis.to_display(timing.start_time)),
        crate::render::text::format_mmss_floor(
            time_axis.to_display(timing.start_time + duration_ms)
        )
    );
    let color = if timing.is_preview {
        mania_constants::gif::style::PREVIEW_TIME_LABEL_COLOR
    } else {
        mania_constants::gif::style::TIME_LABEL_COLOR
    };
    let note_color = if timing.is_preview {
        mania_constants::gif::style::PREVIEW_TIME_LABEL_COLOR
    } else {
        mania_constants::gif::style::TIME_LABEL_NOTE_COLOR
    };
    let (label_w, label_h) = text_size(&label, mania_gif_sizing.TIME_LABEL_FONT_SIZE);
    let sprite = render_text_sprite(&label, mania_gif_sizing.TIME_LABEL_FONT_SIZE, color);
    let x = seg_left + (layout.image_width - label_w as i64).div_euclid(2);

    let note = if timing.is_preview {
        let note_text = "Preview Time";
        let (note_w, _) = text_size(note_text, mania_gif_sizing.TIME_LABEL_NOTE_FONT_SIZE);
        let note_sprite = render_text_sprite(
            note_text,
            mania_gif_sizing.TIME_LABEL_NOTE_FONT_SIZE,
            note_color,
        );
        let note_x = seg_left + (layout.image_width - note_w as i64).div_euclid(2);
        Some(Box::new(PreLabel {
            sprite: note_sprite,
            x: note_x,
            y: y + label_h as i64
                + crate::render::geometry::scale_px(
                    4.0,
                    crate::render::geometry::output_scale(
                        crate::render::geometry::GameMode::Mania,
                        crate::render::geometry::OutputFormat::Gif,
                    ),
                ),
            note: None,
        }))
    } else {
        None
    };

    PreLabel { sprite, x, y, note }
}

/// 使用预渲染标签精灵图绘制 SV 指示器。
/// `sv_sprites` 是一次构建的 `(scroll_position, sprite)` 对，每帧只计算 y 位置。
fn draw_gif_sv_indicators_fast(
    canvas: &mut Img,
    sv_sprites: &[(f64, Img)],
    seg_left: i64,
    snapshot_pos: f64,
    layout: &AnimationLayout,
    pixels_per_scroll_unit: f64,
) {
    // timing points 已排序且所有有效 SV 倍率为正，因此 SV 位置单调。
    // 两侧边界都纳入，以保持旧版在游戏区域边缘的像素可见性行为。
    let (lo_pos, hi_pos) = visible_pos_window(snapshot_pos, layout, pixels_per_scroll_unit, 0.0);
    let start = sv_sprites.partition_point(|(pos, _)| *pos < lo_pos);
    for &(position, ref sprite) in &sv_sprites[start..] {
        if position > hi_pos {
            break;
        }
        let y = y_at_position(position, snapshot_pos, layout, pixels_per_scroll_unit);
        if y < layout.playfield_top || y > layout.playfield_top + layout.playfield_height {
            continue;
        }
        let label_h = sprite.h as i64;
        let x = (seg_left - layout.sv_info_margin_left + layout.sv_label_inset).max(0);
        let label_y = (y as f64 - label_h as f64 / 2.0).floor() as i64;
        canvas.alpha_composite(sprite, x, label_y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 多段 GIF 布局下段左边界可以为负（页边距大于段宽）：FL 遮罩必须只作用于
    /// 画布内的像素，光圈带外压暗、带内保持原样，且负原点不能让下标回绕。
    #[test]
    fn flashlight_mask_with_negative_segment_left_stays_inside_canvas() {
        let layout = test_layout();
        let timeline = mania_visibility_timeline(&[], &[]);
        let mut canvas = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            [200; 4],
        );
        let left = -40;
        let rect = crate::render::geometry::PixelRect {
            x: left,
            y: 0,
            width: layout.segment_width,
            height: layout.playfield_height,
        };
        mania_flashlight(&timeline, 0, &layout, left).apply(&mut canvas, rect);
        // 光圈带中心（段中心 = -40 + 50 = 10、y = 20 + 384 = 404）保持原样。
        assert_eq!(canvas.get(0, 404), [200; 4]);
        // 带外（画布内仍被覆盖的行）被压暗。
        assert_eq!(canvas.get(0, 0), [0, 0, 0, 255]);
        // 段右边界（-40 + 100 = 60）之外不越界写入：更右侧的像素仍是原色。
        assert_eq!(canvas.get(59, 0), [0, 0, 0, 255]);
        assert_eq!(canvas.get(60, 0), [200; 4]);
        assert_eq!(canvas.get(canvas.w - 1, 0), [200; 4]);
    }

    #[test]
    fn hidden_fades_hold_body_and_head_without_covering_judgement_line() {
        let skin = load_mania_skin_config(4, crate::render::geometry::OutputFormat::Gif);
        let layout = build_layout(&skin, crate::render::geometry::OutputFormat::Gif);
        let object = ManiaHitObject {
            lane: 0,
            start_time: 1000,
            end_time: 3000,
            is_long_note: true,
            ..Default::default()
        };
        let palette = super::super::lane_palette(4);
        let holds: Vec<_> = palette.iter().map(|&color| darken(color, 0.5)).collect();
        let left = layout.playfield_left;
        let mut notes = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            [0, 0, 0, 0],
        );
        draw_gif_hit_object(
            &mut notes,
            &object,
            &palette,
            &holds,
            left,
            1000.0,
            3000.0,
            900.0,
            &layout,
            0.15 * layout.render_scale,
        );
        let x = (left + layout.left_panel_width + layout.column_widths[0] / 2) as u32;
        let head_y = y_at_position(1000.0, 900.0, &layout, 0.15 * layout.render_scale)
            - layout.note_head_height / 2;
        assert_eq!(notes.get(x, head_y as u32)[3], 255);
        let upper_y = layout.playfield_top + 100;
        assert_eq!(notes.get(x, upper_y as u32)[3], 255);
        let timeline = mania_visibility_timeline(std::slice::from_ref(&object), &[]);
        mania_hidden(&timeline, 900, &layout).apply(&mut notes, layout.content);
        assert_eq!(notes.get(x, head_y as u32)[3], 0);
        assert_eq!(notes.get(x, upper_y as u32)[3], 255);
        let mut canvas = Img::new(notes.w, notes.h, [0, 0, 0, 0]);
        draw_segment_background(&mut canvas, left, &layout);
        let judgement = canvas.get(x, (layout.playfield_top + layout.hit_position_y) as u32);
        canvas.alpha_composite(&notes, 0, 0);
        assert_eq!(
            canvas.get(x, (layout.playfield_top + layout.hit_position_y) as u32),
            judgement
        );
    }

    /// 下落音符模式下，未来音符位于判定线以上，过去音符位于判定线以下。
    fn y_at_time(
        time: f64,
        snapshot_time: i64,
        layout: &AnimationLayout,
        scroll_map: &ScrollMap,
        pixels_per_scroll_unit: f64,
    ) -> i64 {
        let distance = scroll_map.position_at(time) - scroll_map.position_at(snapshot_time as f64);
        layout.playfield_top + layout.hit_position_y
            - round_half_even(distance * pixels_per_scroll_unit)
    }

    fn test_layout() -> AnimationLayout {
        AnimationLayout {
            segment_width: 100,
            playfield_height: 768,
            lane_area_width: 80,
            image_width: 140,
            image_height: 808,
            hit_position_y: 640,
            scroll_length: 624,
            note_head_height: 15,
            column_left_offsets: vec![0],
            column_widths: vec![80],
            playfield_left: 0,
            playfield_top: 20,
            left_panel_width: 12,
            note_side_padding: 2,
            sv_info_margin_left: 40,
            sv_label_inset: 3,
            sv_font_size: 8,
            sv_text_color: [95, 221, 108, 255],
            render_scale: 1.0,
            content: crate::render::geometry::PixelRect {
                x: 0,
                y: 20,
                width: 140,
                height: 768,
            },
            lane_background: [0, 0, 0, 255],
            column_background: [0, 0, 0, 255],
            left_panel_background: [112, 112, 112, 255],
            judgement_line_color: [238, 238, 238, 255],
        }
    }

    fn variable_sv_map() -> ScrollMap {
        ScrollMap {
            starts: vec![0.0, 1_000.0, 2_000.0, 4_000.0],
            positions: vec![0.0, 1_000.0, 1_250.0, 5_250.0],
            multipliers: vec![1.0, 0.25, 2.0, 0.5],
        }
    }

    #[test]
    fn precomputed_positions_match_time_based_y_across_sv_changes() {
        let layout = test_layout();
        let scroll_map = variable_sv_map();
        let pixels_per_scroll_unit = 0.8;

        for snapshot in [-500, 0, 999, 1_000, 1_500, 2_000, 3_999, 4_000, 6_000] {
            let snapshot_pos = scroll_map.position_at(snapshot as f64);
            for time in [-250, 0, 500, 1_000, 1_750, 2_000, 3_000, 4_000, 5_000] {
                let old_y = y_at_time(
                    time as f64,
                    snapshot,
                    &layout,
                    &scroll_map,
                    pixels_per_scroll_unit,
                );
                let new_y = y_at_position(
                    scroll_map.position_at(time as f64),
                    snapshot_pos,
                    &layout,
                    pixels_per_scroll_unit,
                );
                assert_eq!(old_y, new_y, "snapshot={snapshot}, time={time}");
            }
        }
    }

    #[test]
    fn sv_position_window_matches_full_pixel_visibility_scan() {
        let layout = test_layout();
        let scroll_map = variable_sv_map();
        let pixels_per_scroll_unit = 0.8;
        let sv_times = [0, 500, 1_000, 1_500, 2_000, 2_000, 3_000, 4_000, 5_000];
        let sv_positions: Vec<f64> = sv_times
            .iter()
            .map(|&time| scroll_map.position_at(time as f64))
            .collect();

        for snapshot in [-500, 0, 750, 1_000, 1_750, 2_000, 3_500, 4_000, 6_000] {
            let snapshot_pos = scroll_map.position_at(snapshot as f64);
            let expected: Vec<usize> = sv_times
                .iter()
                .enumerate()
                .filter_map(|(index, &time)| {
                    let y = y_at_time(
                        time as f64,
                        snapshot,
                        &layout,
                        &scroll_map,
                        pixels_per_scroll_unit,
                    );
                    (y >= layout.playfield_top
                        && y <= layout.playfield_top + layout.playfield_height)
                        .then_some(index)
                })
                .collect();

            let (lo_pos, hi_pos) =
                visible_pos_window(snapshot_pos, &layout, pixels_per_scroll_unit, 0.0);
            let start = sv_positions.partition_point(|&pos| pos < lo_pos);
            let actual: Vec<usize> = (start..sv_positions.len())
                .take_while(|&index| sv_positions[index] <= hi_pos)
                .filter(|&index| {
                    let y = y_at_position(
                        sv_positions[index],
                        snapshot_pos,
                        &layout,
                        pixels_per_scroll_unit,
                    );
                    y >= layout.playfield_top && y <= layout.playfield_top + layout.playfield_height
                })
                .collect();

            assert_eq!(expected, actual, "snapshot={snapshot}");
        }
    }

    #[test]
    fn all_mania_keycounts_keep_lane_centers_across_output_scales() {
        for keys in 1..=18 {
            let skin = crate::render::cpu::modes::mania::skin::load_mania_skin_config(
                keys,
                crate::render::geometry::OutputFormat::Gif,
            );
            let (base_widths, base_offsets, _) =
                build_scaled_columns(&skin.column_widths, &skin.column_line_widths, 1.0);
            let base_centers: Vec<f64> = base_offsets
                .iter()
                .zip(&base_widths)
                .map(|(&left, &width)| left as f64 + width as f64 / 2.0)
                .collect();

            for output_scale in [0.5, 1.0, 1.5, 2.0] {
                let (widths, offsets, _) = build_scaled_columns(
                    &skin.column_widths,
                    &skin.column_line_widths,
                    output_scale,
                );

                assert_eq!(offsets.len(), keys as usize);
                for ((&left, &width), expected_center) in
                    offsets.iter().zip(&widths).zip(&base_centers)
                {
                    let normalized_center = (left as f64 + width as f64 / 2.0) / output_scale;
                    assert!((normalized_center - expected_center).abs() <= 1.0);
                }
            }
        }
    }

    #[test]
    fn lane_darken_alpha_is_converted_to_black_overlay_alpha() {
        assert_eq!(with_alpha([0, 0, 0, 255], 0.0), [0, 0, 0, 0]);
        assert_eq!(with_alpha([0, 0, 0, 255], 0.5), [0, 0, 0, 128]);
        assert_eq!(with_alpha([0, 0, 0, 255], 1.0), [0, 0, 0, 255]);
        assert_eq!(with_alpha([0, 0, 0, 255], -1.0), [0, 0, 0, 0]);
        assert_eq!(with_alpha([0, 0, 0, 255], 2.0), [0, 0, 0, 255]);
    }

    #[test]
    fn gif_grid_keeps_independent_full_frames_at_each_segment_time() {
        use crate::config::{with_config, CoreConfig};
        use crate::render::geometry::{preview_frame_size, OutputFormat};
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let beatmap = crate::parse_beatmap_bytes(
            b"osu file format v14\n[General]\nMode:3\n[Difficulty]\nCircleSize:4\n[TimingPoints]\n0,500,4,1,0,100,1,0\n1200,-50,4,1,0,100,0,0\n[HitObjects]\n64,192,1000,1,0\n192,192,1500,128,0,2500:0:0:0:0:\n320,192,2000,1,0\n448,192,3000,1,0\n",
        ).unwrap();
        for (scale, rows, columns, mods) in [
            (0.5, 2, 2, None),
            (0.75, 1, 4, Some("HD")),
            (0.5, 4, 1, Some("FL")),
        ] {
            let mut config = CoreConfig::default();
            config.render.mania.gif.SCALE = scale;
            config.render.mania.gif.structure.ROW_COUNT = rows;
            config.render.mania.gif.structure.IMAGES_PER_ROW = columns;
            config.render.mania.gif.style.SHOW_HIT_FLASH = true;
            config.render.mania.mp4.SCALE = 2.0;
            let mut settings = ModSettings::default();
            settings.hidden = mods == Some("HD");
            settings.flashlight = mods == Some("FL");
            with_config(Arc::new(config.clone()), || {
                let layout = build_layout(
                    &load_mania_skin_config(4, OutputFormat::Gif),
                    OutputFormat::Gif,
                );
                assert_eq!(
                    (layout.image_width, layout.image_height),
                    preview_frame_size(scale)
                );
                assert!(
                    (layout.playfield_left * 2 + layout.segment_width - layout.image_width).abs()
                        <= 1
                );
                let render = |times| {
                    prepare_mania_gif_frames(
                        &beatmap,
                        Some(&settings),
                        GifRenderOptions::ResolvedSegments {
                            times_ms: times,
                            duration_seconds: Some(0.2),
                            time_axis: TimeAxis::new(1000),
                        },
                        Some(10),
                        &RequestDeadline::new(Instant::now(), "gif", Duration::from_secs(30)),
                    )
                    .unwrap()
                    .render(1)
                };
                let times = [900, 1400, 1900, 2900];
                let grid = render(times.to_vec());
                let mut single_config = config.clone();
                single_config.render.mania.gif.structure.ROW_COUNT = 1;
                single_config.render.mania.gif.structure.IMAGES_PER_ROW = 1;
                let (width, height) = gif_unit_size(&layout, true);
                for (index, time) in times.into_iter().enumerate() {
                    let single =
                        with_config(Arc::new(single_config.clone()), || render(vec![time]));
                    let (x, y) = gif_frame_origin(index, &layout, true);
                    let (single_x, single_y) = with_config(Arc::new(single_config.clone()), || {
                        gif_frame_origin(0, &layout, true)
                    });
                    for row in 0..height as u32 {
                        let grid_start = grid.idx(x as u32, y as u32 + row);
                        let single_start = single.idx(single_x as u32, single_y as u32 + row);
                        assert_eq!(
                            &grid.data[grid_start..grid_start + width as usize * 4],
                            &single.data[single_start..single_start + width as usize * 4],
                            "scale={scale} cell={index} row={row} mods={mods:?}",
                        );
                    }
                }
                let (_, with_labels) = gif_canvas_size(&layout, true);
                let (_, without_labels) = gif_canvas_size(&layout, false);
                let sizing = mania_constants::gif::sizing();
                assert_eq!(
                    with_labels - without_labels,
                    rows as i64 * (sizing.TIME_LABEL_TOP_GAP + sizing.INFO_MARGIN_BOTTOM)
                );
            });
        }
    }

    #[test]
    fn gif_sv_labels_use_frame_space_without_expanding_canvas() {
        use crate::config::{with_config, CoreConfig};
        use crate::render::geometry::OutputFormat;
        use std::sync::Arc;

        for scale in [0.5, 1.0, 1.5] {
            let layout_for = |show_sv_label| {
                let mut config = CoreConfig::default();
                config.render.mania.gif.SCALE = scale;
                config.render.mania.gif.style.SHOW_SV_LABEL = show_sv_label;
                with_config(Arc::new(config), || {
                    let skin = load_mania_skin_config(4, OutputFormat::Gif);
                    build_layout(&skin, OutputFormat::Gif)
                })
            };
            let hidden = layout_for(false);
            let shown = layout_for(true);
            assert!(shown.sv_info_margin_left > 0);
            assert_eq!(hidden.sv_info_margin_left, 0);
            assert_eq!(shown.image_width, hidden.image_width);
            assert_eq!(shown.image_height, hidden.image_height);
            assert_eq!(shown.column_widths, hidden.column_widths);
        }
    }

    #[test]
    fn video_layout_centers_stage_without_resizing_notes() {
        for scale in [1.0_f64, 2.0] {
            let mut custom = crate::config::CoreConfig::default();
            custom.render.mania.mp4.SCALE = scale;
            crate::config::with_config(std::sync::Arc::new(custom), || {
                for keys in [4, 7] {
                    let skin = crate::render::cpu::modes::mania::skin::load_mania_skin_config(
                        keys,
                        crate::render::geometry::OutputFormat::Mp4,
                    );
                    let content = build_layout(&skin, crate::render::geometry::OutputFormat::Mp4);
                    let video =
                        build_video_layout(&skin, crate::render::geometry::OutputFormat::Mp4);
                    let canvas = crate::render::geometry::video_canvas(content.content);
                    assert_eq!(
                        (video.image_width, video.image_height),
                        (canvas.width as i64, canvas.height as i64)
                    );
                    assert_eq!(
                        (video.content.x, video.content.y),
                        (canvas.origin_x, canvas.origin_y)
                    );
                    assert_eq!(
                        (video.content.width, video.content.height),
                        (content.content.width, content.content.height)
                    );
                    assert_eq!(
                        video.playfield_left,
                        content.playfield_left + canvas.origin_x
                    );
                    assert_eq!(video.playfield_top, content.playfield_top + canvas.origin_y);
                    // 键道宽度、滚动长度与音符头高度都不随画布变化。
                    assert_eq!(video.column_widths, content.column_widths);
                    assert_eq!(video.scroll_length, content.scroll_length);
                    assert_eq!(video.note_head_height, content.note_head_height);
                }
            });
        }
    }
}
