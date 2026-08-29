//! osu!mania MP4 渲染器：完整谱面连续播放（无四段预览）。
//! 在单列布局中复用 GIF 的逐音符绘制逻辑；省略底部段落标签，
//! 全局右上角标签由 `video::save_mp4_streamed` 绘制。
//!
//! 时间范围由 `--time-points` 和 `--duration-time` 控制。

use crate::common::time_selection::TimeAxis;
use crate::core::errors::{PreviewError, Result};
use crate::core::models::Beatmap;
use crate::core::mods::ModSettings;
use crate::core::timeout::RequestDeadline;
use crate::core::validate::TimePoint;
use crate::parser::round_half_even;
use crate::render::canvas::{Img, Rgba};
use crate::render::video::audio::AudioSourceJob;
use crate::render::video::{resolve_video_time_range, save_mp4_streamed};
use std::path::Path;

use super::gif::{
    build_column_left_offsets, build_scroll_map, compute_time_range, draw_gif_hit_object,
    draw_gif_sv_indicators, draw_segment_background, segment_left, visible_pos_window, GifLayout,
};
use super::skin::load_mania_skin_config;
use super::{
    apply_hold_off_mod, apply_inverse_mod, build_sv_changes, darken, is_native_mania,
    mania_objects, resolve_key_count,
};

pub(crate) fn render_mania_video(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    start_time: Option<TimePoint>,
    duration_time: Option<f64>,
    output_path: &Path,
    background: Option<Img>,
    audio_job: AudioSourceJob,
    time_axis: TimeAxis,
    deadline: &RequestDeadline,
) -> Result<()> {
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
    let cs_mode = mods.is_some_and(|m| m.cs_override);
    if hit_objects.is_empty() {
        return Err(PreviewError::render("mania beatmap has no hit objects"));
    }

    let speed = mods.map_or(1.0, |m| m.speed_multiplier);
    let first = original_objects
        .iter()
        .map(|h| h.start_time)
        .min()
        .unwrap_or(0);
    let last = original_objects
        .iter()
        .map(|h| h.end_time)
        .max()
        .unwrap_or(0);
    let range = resolve_video_time_range(beatmap, first, last, start_time, duration_time, speed)?;
    let (start, end) = (range.start, range.end);
    let total_ms = end - start;
    let fps = crate::config::current().layout.mania.mp4.FPS as u32;
    let frame_count = ((total_ms as f64 * fps as f64 / (1000.0 * speed)).round() as usize).max(1);

    let skin_config = load_mania_skin_config(key_count);
    let layout = build_video_layout(&skin_config);
    let native_mania = is_native_mania(beatmap);
    let scroll_map = build_scroll_map(beatmap, &original_objects, cs_mode, native_mania);
    let time_range = compute_time_range(speed, skin_config.hit_position);
    let pixels_per_scroll_unit = layout.scroll_length as f64 / time_range;
    let sv_changes = if cs_mode || !native_mania {
        Vec::new()
    } else {
        build_sv_changes(&beatmap.timing_points, end + round_half_even(time_range))
    };
    let sv_positions: Vec<f64> = sv_changes
        .iter()
        .map(|&(time, _)| scroll_map.position_at(time as f64))
        .collect();
    let hold_colors: Vec<Rgba> = palette.iter().map(|&c| darken(c, 0.5)).collect();

    // 预计算滚动距离位置，供排序后的二分查找裁剪使用。
    // 按距离而不是谱面时间裁剪，在可变 SV 下仍然正确（时间与位置不是线性关系）；
    // 详见 `visible_pos_window`。
    let pos_start: Vec<f64> = hit_objects
        .iter()
        .map(|ho| scroll_map.position_at(ho.start_time as f64))
        .collect();
    let pos_end: Vec<f64> = hit_objects
        .iter()
        .map(|ho| scroll_map.position_at(ho.end_time as f64))
        .collect();
    let max_hold_position: f64 = pos_start
        .iter()
        .zip(&pos_end)
        .map(|(&start, &end)| (end - start).max(0.0))
        .fold(0.0_f64, f64::max);

    // 单段静态背景：一列背景和判定线，不绘制段间分隔线。
    let static_bg = {
        // 背景图在最终视频画布上统一处理；这里仅绘制 Mania 轨道和侧板。
        let mut bg = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            if background.is_some() {
                [0, 0, 0, 0]
            } else {
                crate::config::current().layout.mania.mp4.IMAGE_BACKGROUND
            },
        );
        draw_segment_background(&mut bg, segment_left(0, &layout), &layout);
        bg
    };

    let render = move |frame_index: usize| -> (Img, i64) {
        let snapshot_time =
            start + round_half_even(frame_index as f64 * 1000.0 * speed / fps as f64);
        let snapshot_pos = scroll_map.position_at(snapshot_time as f64);
        let mut canvas = static_bg.clone();
        let seg_left = segment_left(0, &layout);
        draw_gif_sv_indicators(
            &mut canvas,
            &sv_changes,
            &sv_positions,
            seg_left,
            snapshot_pos,
            &layout,
            pixels_per_scroll_unit,
        );
        // 对预计算的滚动距离位置执行二分查找。按距离而不是谱面时间裁剪，
        // 在可变 SV 下仍然正确。
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
                &mut canvas,
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
        background,
        time_axis,
        deadline,
    )
}

/// MP4 的单段布局：单列宽度、无段间间隔、无底部标签区域（全局右上角标签由编码器绘制）。
fn build_video_layout(skin_config: &super::skin::ManiaSkinConfig) -> GifLayout {
    let column_left_offsets =
        build_column_left_offsets(&skin_config.column_widths, &skin_config.column_line_widths);
    let lane_area_width: i64 = skin_config.column_widths.iter().sum::<i64>()
        + skin_config.column_line_widths.iter().sum::<i64>();
    let segment_width = crate::render::mania::constants::LEFT_PANEL_WIDTH * 2 + lane_area_width;
    let playfield_height = crate::render::mania::constants::FRAME_HEIGHT;
    let hit_position_y = round_half_even(playfield_height as f64 - skin_config.hit_position);
    let scroll_length =
        (hit_position_y - crate::render::mania::constants::STAGE_TOP_PADDING).max(1);
    let average_column_width = skin_config.column_widths.iter().sum::<i64>() as f64
        / skin_config.column_widths.len() as f64;
    let note_head_height = round_half_even(
        crate::render::mania::constants::NOTE_HEAD_HEIGHT as f64 * average_column_width
            / crate::render::mania::constants::LANE_WIDTH as f64,
    )
    .max(1);
    let image_width = crate::render::mania::constants::PAGE_MARGIN_X * 2 + segment_width;
    let image_height = crate::render::mania::constants::PAGE_MARGIN_Y * 2 + playfield_height;
    GifLayout {
        segment_count: 1,
        segment_width,
        playfield_height,
        lane_area_width,
        image_width,
        image_height,
        hit_position_y,
        scroll_length,
        note_head_height,
        column_left_offsets,
        column_widths: skin_config.column_widths.clone(),
        column_colours: skin_config.column_colours.clone(),
    }
}
