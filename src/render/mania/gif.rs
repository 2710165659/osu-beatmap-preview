//! osu!mania GIF 渲染器：多段或单画面下落式音符预览。
//! 移植自 beatmap_preview/mania/gif_renderer.py。

use crate::common::time_selection::{GifRenderOptions, PreviewTimeSelector, TimeAxis};
use crate::core::errors::{PreviewError, Result};
use crate::core::models::{Beatmap, ManiaHitObject, TimingPoint};
use crate::core::mods::ModSettings;
use crate::core::timeout::RequestDeadline;
use crate::parser::round_half_even;
use crate::render::canvas::{Img, Rgba};
use crate::render::composer::save_animated_gif_streamed;
use crate::render::text::{draw_text, render_text_sprite, text_size};
use std::path::Path;

use super::{
    apply_hold_off_mod, apply_inverse_mod, build_sv_changes, darken, format_sv_label,
    is_native_mania, mania_objects, resolve_key_count,
};

use super::skin::load_mania_skin_config;

pub(crate) struct GifLayout {
    pub(crate) segment_count: i64,
    pub(crate) segment_width: i64,
    pub(crate) playfield_height: i64,
    pub(crate) lane_area_width: i64,
    pub(crate) image_width: i64,
    pub(crate) image_height: i64,
    pub(crate) hit_position_y: i64,
    pub(crate) scroll_length: i64,
    pub(crate) note_head_height: i64,
    pub(crate) column_left_offsets: Vec<i64>,
    pub(crate) column_widths: Vec<i64>,
    pub(crate) column_colours: Vec<Rgba>,
}

/// 将谱面时间映射为连续滚动距离，同时处理 BPM 和 SV 变化。
pub(crate) struct ScrollMap {
    starts: Vec<f64>,
    positions: Vec<f64>,
    multipliers: Vec<f64>,
}

impl ScrollMap {
    pub(crate) fn position_at(&self, time: f64) -> f64 {
        let index = self
            .starts
            .partition_point(|s| *s <= time)
            .saturating_sub(1);
        self.positions[index] + (time - self.starts[index]) * self.multipliers[index]
    }
}

pub(crate) fn render_mania_gif(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    options: GifRenderOptions,
    output_path: &Path,
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

    // DT/HT 只改变谱面时间推进速度；GIF 每段仍播放 10 秒。
    let speed_multiplier = mods.map_or(1.0, |m| m.speed_multiplier);
    let (segment_timings, segment_duration, frame_count, show_time_label, time_axis) = match options
    {
        GifRenderOptions::Segments {
            times_ms,
            time_axis,
        } => {
            let gameplay_segment_duration = round_half_even(
                crate::config::current().layout.mania.gif.DURATION_MS as f64 * speed_multiplier,
            );
            let spans: Vec<(i64, i64)> = hit_objects
                .iter()
                .map(|ho| (ho.start_time, ho.end_time))
                .collect();
            let segment_timings = PreviewTimeSelector::new(
                beatmap,
                spans,
                crate::config::current().layout.mania.gif.IMAGES_PER_ROW as usize,
                gameplay_segment_duration,
                times_ms,
            )?
            .choose()?;
            let frame_count = round_half_even(
                (crate::config::current().layout.mania.gif.DURATION_MS
                    * crate::config::current().layout.mania.gif.FPS) as f64
                    / 1000.0,
            )
            .max(1);
            (
                segment_timings,
                gameplay_segment_duration,
                frame_count,
                crate::config::current().layout.mania.gif.SHOW_TIME_LABEL,
                time_axis,
            )
        }
    };

    let skin_config = load_mania_skin_config(key_count);
    let layout = build_gif_layout(&skin_config, segment_timings.len() as i64, show_time_label);
    let native_mania = is_native_mania(beatmap);
    // CS 是恒定滚动：保留 33 速时间窗口，但跳过 SV 倍率。
    let scroll_map = build_scroll_map(beatmap, &original_objects, cs_mode, native_mania);
    // time_range 是 33 速下从判定线到顶部可见的谱面时间跨度。
    let time_range = compute_time_range(speed_multiplier, skin_config.hit_position);
    let pixels_per_scroll_unit = layout.scroll_length as f64 / time_range;
    let frame_duration_ms =
        round_half_even(1000.0 / crate::config::current().layout.mania.gif.FPS as f64).max(1);
    let max_segment_end = segment_timings
        .iter()
        .map(|t| t.start_time + segment_duration)
        .max()
        .unwrap_or(0);
    let sv_changes = if cs_mode || !native_mania {
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
                        + round_half_even(
                            frame_index as f64 * 1000.0 * speed_multiplier
                                / crate::config::current().layout.mania.gif.FPS as f64,
                        )
                })
                .collect()
        })
        .collect();

    let hold_colors: Vec<Rgba> = palette.iter().map(|&c| darken(c, 0.5)).collect();

    // 预计算每个音符的滚动距离位置，供排序后的二分查找裁剪使用。
    // position_at 随时间严格单调（所有滚动倍率都大于 0），hit_objects 又按
    // start_time 排序，因此 `pos_start` 为升序。
    //
    // 可变 SV 下必须按滚动距离而不是谱面时间裁剪：时间与位置并非线性关系，
    // 慢 SV 会把很长的谱面时间压缩到少量屏幕像素，按时间窗口会误删可见音符。
    // 距离通过常量 `pixels_per_scroll_unit` 映射到 y，因此任意 SV 下都保持精确。
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

    // 静态背景（段分隔线、列/轨道背景、判定线）只预渲染一次并逐帧克隆，
    // 避免 150 帧内重复绘制约 600 次相同像素。
    let static_bg = {
        let mut bg = Img::new(
            layout.image_width as u32,
            layout.image_height as u32,
            crate::config::current().layout.mania.gif.IMAGE_BACKGROUND,
        );
        draw_segment_separators(&mut bg, &layout);
        for segment_index in 0..layout.segment_count {
            draw_segment_background(&mut bg, segment_left(segment_index, &layout), &layout);
        }
        bg
    };

    // 每段的时间标签（及 "Preview Time" 提示）只预渲染一次为精灵图。
    // 段内文字恒定，可避免每段 150 次 format!、text_size、draw_text 调用；
    // 每帧只需合成预构建精灵图。
    let label_y = crate::render::mania::constants::PAGE_MARGIN_Y
        + layout.playfield_height
        + crate::config::current().layout.mania.gif.TIME_LABEL_TOP_GAP;
    let pre_labels: Vec<PreLabel> = if show_time_label {
        segment_timings
            .iter()
            .enumerate()
            .map(|(si, st)| {
                let seg_left = segment_left(si as i64, &layout);
                build_pre_label(st, segment_duration, &layout, seg_left, label_y, time_axis)
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
                    crate::config::current().layout.mania.gif.SV_TEXT_FONT_SIZE,
                    crate::config::current().layout.mania.gif.SV_TEXT_COLOR,
                ),
            )
        })
        .collect();

    let render_frame = |frame_index: usize| -> Img {
        let mut canvas = static_bg.clone();

        for (segment_index, _segment_timing) in segment_timings.iter().enumerate() {
            let seg_left = segment_left(segment_index as i64, &layout);
            let snapshot_time = segment_snapshot_times[segment_index][frame_index];
            let snapshot_pos = scroll_map.position_at(snapshot_time as f64);
            draw_gif_sv_indicators_fast(
                &mut canvas,
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

    save_animated_gif_streamed(
        frame_count as usize,
        render_frame,
        output_path,
        frame_duration_ms as u32,
        deadline,
    )
}

fn build_gif_layout(
    skin_config: &super::skin::ManiaSkinConfig,
    segment_count: i64,
    show_time_label: bool,
) -> GifLayout {
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
    // PNG 使用 38px 轨道和 15px 音符；GIF 音符高度随皮肤缩放。
    // 随列宽缩放，避免宽列中的音符显得被压扁。
    let note_head_height = round_half_even(
        crate::render::mania::constants::NOTE_HEAD_HEIGHT as f64 * average_column_width
            / crate::render::mania::constants::LANE_WIDTH as f64,
    )
    .max(1);
    let image_width = crate::render::mania::constants::PAGE_MARGIN_X * 2
        + segment_count * segment_width
        + (segment_count - 1) * crate::config::current().layout.mania.gif.GRID_GAP;
    let label_height = if show_time_label {
        crate::config::current().layout.mania.gif.TIME_LABEL_TOP_GAP
            + crate::config::current().layout.mania.gif.TIME_LABEL_HEIGHT
    } else {
        0
    };
    let image_height =
        crate::render::mania::constants::PAGE_MARGIN_Y * 2 + playfield_height + label_height;
    GifLayout {
        segment_count,
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

pub(crate) fn build_column_left_offsets(
    column_widths: &[i64],
    column_line_widths: &[i64],
) -> Vec<i64> {
    // ColumnLineWidth 包含 keys + 1 项：最左侧、列之间、最右侧。
    let mut offsets = Vec::with_capacity(column_widths.len());
    let mut cursor = column_line_widths.first().copied().unwrap_or(0);
    for (index, width) in column_widths.iter().enumerate() {
        offsets.push(cursor);
        cursor += width;
        if index + 1 < column_line_widths.len() {
            cursor += column_line_widths[index + 1];
        }
    }
    offsets
}

/// 对应 DrawableManiaRuleset.updateTimeRange()：根据 HitPosition 调整 33 速基础窗口。
pub(crate) fn compute_time_range(speed_multiplier: f64, hit_position: f64) -> f64 {
    let hit_position_scale = (crate::render::mania::constants::FRAME_HEIGHT as f64 - hit_position)
        / (crate::render::mania::constants::FRAME_HEIGHT as f64
            - crate::render::mania::constants::DEFAULT_HIT_POSITION_FROM_BOTTOM);
    (crate::render::mania::constants::BASE_TIME_RANGE_MS
        / crate::config::current().layout.mania.gif.SCROLL_SPEED
        * hit_position_scale
        * speed_multiplier)
        .max(1.0)
}

pub(crate) fn build_scroll_map(
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

pub(crate) fn segment_left(segment_index: i64, layout: &GifLayout) -> i64 {
    crate::render::mania::constants::PAGE_MARGIN_X
        + segment_index
            * (layout.segment_width + crate::config::current().layout.mania.gif.GRID_GAP)
}

fn draw_segment_separators(canvas: &mut Img, layout: &GifLayout) {
    let playfield_top = crate::render::mania::constants::PAGE_MARGIN_Y;
    let playfield_bottom = playfield_top + layout.playfield_height;
    for segment_index in 0..layout.segment_count - 1 {
        let left_segment_right = segment_left(segment_index, layout) + layout.segment_width;
        let separator_left = left_segment_right
            + (crate::config::current().layout.mania.gif.GRID_GAP
                - crate::config::current().layout.mania.gif.SEPARATOR_WIDTH)
                / 2;
        canvas.set_rect(
            separator_left,
            playfield_top,
            separator_left + crate::config::current().layout.mania.gif.SEPARATOR_WIDTH,
            playfield_bottom,
            crate::config::current()
                .layout
                .mania
                .gif
                .SEPARATOR_BACKGROUND,
        );
    }
}

pub(crate) fn draw_segment_background(canvas: &mut Img, seg_left: i64, layout: &GifLayout) {
    // GIF 不绘制小节线、节拍线和轨道分隔线，只保留灰色侧板与判定线。
    let playfield_top = crate::render::mania::constants::PAGE_MARGIN_Y;
    let playfield_bottom = playfield_top + layout.playfield_height;
    let lane_area_left = seg_left + crate::render::mania::constants::LEFT_PANEL_WIDTH;
    let lane_area_right = lane_area_left + layout.lane_area_width;

    canvas.set_rect(
        seg_left,
        playfield_top,
        seg_left + layout.segment_width,
        playfield_bottom,
        crate::config::current().layout.mania.gif.LANE_BACKGROUND,
    );
    canvas.set_rect(
        seg_left,
        playfield_top,
        lane_area_left,
        playfield_bottom,
        crate::config::current()
            .layout
            .mania
            .gif
            .LEFT_PANEL_BACKGROUND,
    );
    canvas.set_rect(
        lane_area_right,
        playfield_top,
        seg_left + layout.segment_width,
        playfield_bottom,
        crate::config::current()
            .layout
            .mania
            .gif
            .LEFT_PANEL_BACKGROUND,
    );

    for (lane_index, &lane_width) in layout.column_widths.iter().enumerate() {
        let lane_left = lane_area_left + layout.column_left_offsets[lane_index];
        canvas.set_rect(
            lane_left,
            playfield_top,
            lane_left + lane_width,
            playfield_bottom,
            layout.column_colours[lane_index],
        );
    }

    let judgement_y = playfield_top + layout.hit_position_y;
    canvas.draw_line(
        seg_left as f64,
        judgement_y as f64,
        (seg_left + layout.segment_width) as f64,
        judgement_y as f64,
        2.0,
        crate::config::current()
            .layout
            .mania
            .gif
            .JUDGEMENT_LINE_COLOR,
    );
}

pub(crate) fn draw_gif_sv_indicators(
    canvas: &mut Img,
    sv_changes: &[(i64, f64)],
    sv_positions: &[f64],
    seg_left: i64,
    snapshot_pos: f64,
    layout: &GifLayout,
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
        if y < crate::render::mania::constants::PAGE_MARGIN_Y
            || y > crate::render::mania::constants::PAGE_MARGIN_Y + layout.playfield_height
        {
            continue;
        }
        let label = format_sv_label(sv);
        let (label_w, label_h) = text_size(
            &label,
            crate::config::current().layout.mania.gif.SV_TEXT_FONT_SIZE,
        );
        let x = (seg_left + crate::render::mania::constants::LEFT_PANEL_WIDTH - label_w as i64 - 3)
            .max(0);
        let label_y = (y as f64 - label_h as f64 / 2.0).floor() as i64;
        draw_text(
            canvas,
            x,
            label_y,
            &label,
            crate::config::current().layout.mania.gif.SV_TEXT_FONT_SIZE,
            crate::config::current().layout.mania.gif.SV_TEXT_COLOR,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_gif_hit_object(
    canvas: &mut Img,
    hit_object: &ManiaHitObject,
    palette: &[Rgba],
    hold_colors: &[Rgba],
    seg_left: i64,
    start_pos: f64,
    end_pos: f64,
    snapshot_pos: f64,
    layout: &GifLayout,
    pixels_per_scroll_unit: f64,
) {
    let y_start = y_at_position(start_pos, snapshot_pos, layout, pixels_per_scroll_unit);
    let y_end = y_at_position(end_pos, snapshot_pos, layout, pixels_per_scroll_unit);
    let playfield_top = crate::render::mania::constants::PAGE_MARGIN_Y;
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
        + crate::render::mania::constants::LEFT_PANEL_WIDTH
        + layout.column_left_offsets[lane]
        + crate::render::mania::constants::NOTE_SIDE_PADDING;
    let lane_right = lane_left + layout.column_widths[lane]
        - crate::render::mania::constants::NOTE_SIDE_PADDING * 2;

    if hit_object.is_long_note {
        let body_top = playfield_top.max(y_end.min(y_start - layout.note_head_height));
        let body_bottom = playfield_bottom.min(y_start);
        if body_top < body_bottom {
            canvas.set_rect(lane_left, body_top, lane_right, body_bottom, hold_color);
        }
    }

    let head_top = playfield_top.max(y_start - layout.note_head_height);
    let head_bottom = playfield_bottom.min(y_start);
    if head_top < head_bottom {
        canvas.set_rect(lane_left, head_top, lane_right, head_bottom, lane_color);
    }
}

/// 与 `y_at_time` 相同，但接收预计算的 `snapshot_pos`，避免每次调用都对
/// `position_at(snapshot_time)` 二分查找。一帧包含数千音符时可消除数千次重复查找。
#[inline]
fn y_at_position(
    object_pos: f64,
    snapshot_pos: f64,
    layout: &GifLayout,
    pixels_per_scroll_unit: f64,
) -> i64 {
    let distance = object_pos - snapshot_pos;
    crate::render::mania::constants::PAGE_MARGIN_Y + layout.hit_position_y
        - round_half_even(distance * pixels_per_scroll_unit)
}

/// 滚动距离窗口 `[lo, hi]`，窗口外不可能有可见音符。
/// 用它二分查找预计算的 `pos_start` 数组，避免每帧扫描全部音符。
///
/// 窗口单位是滚动距离（position_at），不是谱面时间。可变 SV 使时间与位置非线性：
/// 慢 SV 会把很长的谱面时间压缩到少量屏幕像素，基于时间的窗口会丢弃屏幕内音符。
/// 距离通过 `pixels_per_scroll_unit` 映射到屏幕 y，因此任意 SV 下都保持精确。
/// `lo`/`hi` 覆盖游戏区域并额外留出 `note_head_height`，避免头部/主体在边缘突现。
///
/// 从下界减去 `max_hold_position`（距离空间中最宽的长按主体），防止长按被截断：
/// `end_time` 仍在屏幕内的长按，其 `start_time` 在距离上可能早得多。
/// 由于 `pos_start >= pos_end - max_hold_position` 且
/// `pos_end >= snapshot_pos - past_dist`，可得 `pos_start >= lo`，
/// partition_point 会保留该音符。`draw_gif_hit_object` 内部仍执行 y 裁剪以保证像素精度。
#[inline]
pub(crate) fn visible_pos_window(
    snapshot_pos: f64,
    layout: &GifLayout,
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
    timing: &crate::common::time_selection::PreviewSegmentTiming,
    duration_ms: i64,
    layout: &GifLayout,
    seg_left: i64,
    y: i64,
    time_axis: TimeAxis,
) -> PreLabel {
    let label = format!(
        "{} - {}",
        crate::render::text::format_mmss_floor(time_axis.to_display(timing.start_time)),
        crate::render::text::format_mmss_floor(
            time_axis.to_display(timing.start_time + duration_ms)
        )
    );
    let color = if timing.is_preview {
        crate::config::current()
            .layout
            .mania
            .gif
            .PREVIEW_TIME_LABEL_COLOR
    } else {
        crate::config::current().layout.mania.gif.TIME_LABEL_COLOR
    };
    let note_color = if timing.is_preview {
        crate::config::current()
            .layout
            .mania
            .gif
            .PREVIEW_TIME_LABEL_COLOR
    } else {
        crate::config::current()
            .layout
            .mania
            .gif
            .TIME_LABEL_NOTE_COLOR
    };
    let (label_w, label_h) = text_size(
        &label,
        crate::config::current()
            .layout
            .mania
            .gif
            .TIME_LABEL_FONT_SIZE,
    );
    let sprite = render_text_sprite(
        &label,
        crate::config::current()
            .layout
            .mania
            .gif
            .TIME_LABEL_FONT_SIZE,
        color,
    );
    let x = seg_left + (layout.segment_width - label_w as i64).div_euclid(2);

    let note = if timing.is_preview {
        let note_text = "Preview Time";
        let (note_w, _) = text_size(
            note_text,
            crate::config::current()
                .layout
                .mania
                .gif
                .TIME_LABEL_NOTE_FONT_SIZE,
        );
        let note_sprite = render_text_sprite(
            note_text,
            crate::config::current()
                .layout
                .mania
                .gif
                .TIME_LABEL_NOTE_FONT_SIZE,
            note_color,
        );
        let note_x = seg_left + (layout.segment_width - note_w as i64).div_euclid(2);
        Some(Box::new(PreLabel {
            sprite: note_sprite,
            x: note_x,
            y: y + label_h as i64 + 4,
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
    layout: &GifLayout,
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
        if y < crate::render::mania::constants::PAGE_MARGIN_Y
            || y > crate::render::mania::constants::PAGE_MARGIN_Y + layout.playfield_height
        {
            continue;
        }
        let label_h = sprite.h as i64;
        let x =
            (seg_left + crate::render::mania::constants::LEFT_PANEL_WIDTH - sprite.w as i64 - 3)
                .max(0);
        let label_y = (y as f64 - label_h as f64 / 2.0).floor() as i64;
        canvas.alpha_composite(sprite, x, label_y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 下落音符模式下，未来音符位于判定线以上，过去音符位于判定线以下。
    fn y_at_time(
        time: f64,
        snapshot_time: i64,
        layout: &GifLayout,
        scroll_map: &ScrollMap,
        pixels_per_scroll_unit: f64,
    ) -> i64 {
        let distance = scroll_map.position_at(time) - scroll_map.position_at(snapshot_time as f64);
        crate::render::mania::constants::PAGE_MARGIN_Y + layout.hit_position_y
            - round_half_even(distance * pixels_per_scroll_unit)
    }

    fn test_layout() -> GifLayout {
        GifLayout {
            segment_count: 1,
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
            column_colours: vec![[0, 0, 0, 255]],
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
                    (y >= crate::render::mania::constants::PAGE_MARGIN_Y
                        && y <= crate::render::mania::constants::PAGE_MARGIN_Y
                            + layout.playfield_height)
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
                    y >= crate::render::mania::constants::PAGE_MARGIN_Y
                        && y <= crate::render::mania::constants::PAGE_MARGIN_Y
                            + layout.playfield_height
                })
                .collect();

            assert_eq!(expected, actual, "snapshot={snapshot}");
        }
    }
}
