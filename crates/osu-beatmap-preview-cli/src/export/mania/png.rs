//! osu!mania PNG 静态图导出：计算布局、准备对象，交给 core 绘制后保存。

use crate::export::segment::{resolve_png_window, PngSegment, PngWindow};
use crate::media::image::save_png;
use osu_beatmap_preview_core::model::mods::ModSettings;
use osu_beatmap_preview_core::model::{Beatmap, ManiaHitObject, TimingPoint};
use osu_beatmap_preview_core::processing::parse::round_half_even;
use osu_beatmap_preview_core::processing::timeline::TimeAxis;
use osu_beatmap_preview_core::render::cpu::modes::mania::png::{
    render_mania_png_scene, ManiaPngLayout, TimingLine,
};
use osu_beatmap_preview_core::support::error::Result;
use osu_beatmap_preview_core::support::timeout::RequestDeadline;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{
    apply_hold_off_mod, apply_inverse_mod, build_sv_changes, is_native_mania, mania_objects,
    resolve_key_count,
};

pub(crate) fn render_mania_grid(
    beatmap: &Beatmap,
    output_path: &Path,
    mods: Option<&ModSettings>,
    time_axis: TimeAxis,
    segment: Option<PngSegment>,
    deadline: &RequestDeadline,
) -> Result<PathBuf> {
    deadline.check()?;
    // 键数直接来自谱面 CS（模组不会改变原生 mania 轨道数）。
    let key_count = resolve_key_count(beatmap)?;
    let palette = super::lane_palette(key_count);

    let mut hit_objects = mania_objects(beatmap);
    if mods.is_some_and(|m| m.inverse) {
        hit_objects = apply_inverse_mod(&hit_objects, &beatmap.timing_points);
    }
    if mods.is_some_and(|m| m.hold_off) {
        hit_objects = apply_hold_off_mod(&hit_objects);
    }

    let cs_mode = mods.is_some_and(|m| m.cs_override);
    let native_mania = is_native_mania(beatmap);

    // 裁剪开头静音：若第一个音符在 5 秒之后，则从其前 1 秒开始，
    // 并对齐到红线节拍网格。
    let first_note_time = hit_objects
        .iter()
        .map(|ho| ho.start_time)
        .min()
        .unwrap_or(0);
    let chart_start_time = if first_note_time >= 5000 {
        osu_beatmap_preview_core::processing::timeline::snap_to_beat_grid(
            first_note_time - 1000,
            &beatmap.timing_points,
        )
    } else {
        0
    };

    // 整谱范围以最后一个物件结尾；底部留白只是布局样式，不参与选段。
    let chart_content_end = hit_objects.iter().map(|ho| ho.end_time).max().unwrap_or(0);
    // 区间段（--time-points + --duration-time）：窗口规则与 MP4 一致（见 resolve_png_window）。
    let window = resolve_png_window(chart_start_time, chart_content_end, segment)?;
    let chart_start_time = window.start_ms;
    // 只保留窗口内的物件并平移到窗口起点；长按跨界按窗口截断。
    // 整谱模式窗口覆盖全部物件，行为不变。
    window_hit_objects(&mut hit_objects, window);

    let beatmap_duration = window.duration_ms();
    let chart_end_time = beatmap_duration
        + crate::config::current()
            .render
            .mania
            .png
            .style
            .BOTTOM_PADDING_MS;
    let timing_points_for_render: Vec<TimingPoint> = beatmap
        .timing_points
        .iter()
        .map(|tp| {
            let mut tp = *tp;
            tp.time -= chart_start_time as f64;
            tp
        })
        .collect();
    let timing_lines = build_timing_lines(
        &timing_points_for_render,
        chart_end_time,
        beatmap.beat_divisor,
        hit_objects.iter().map(|ho| ho.start_time).min().unwrap_or(0),
    );
    let sv_changes = if cs_mode
        || !native_mania
        || !crate::config::current()
            .render
            .mania
            .png
            .style
            .SHOW_SV_LABEL
    {
        Vec::new()
    } else {
        build_sv_changes(&timing_points_for_render, chart_end_time)
    };
    let layout = build_png_layout(
        key_count,
        beatmap_duration,
        chart_end_time,
        chart_start_time,
    )?;
    deadline.check()?;

    let image = render_mania_png_scene(
        &layout,
        key_count,
        &palette,
        &hit_objects,
        &timing_lines,
        &sv_changes,
        time_axis,
    );

    save_png(&image, output_path, deadline)?;
    Ok(output_path.to_path_buf())
}

/// 将物件裁剪进渲染窗口并平移到窗口起点。
///
/// 窗口外的圆点直接丢弃，跨界长按按窗口截断，避免窗口外的物件被挤进最后一列；
/// 整谱模式窗口覆盖全部物件，结果与不裁剪一致。
fn window_hit_objects(hit_objects: &mut Vec<ManiaHitObject>, window: PngWindow) {
    let duration = window.duration_ms();
    hit_objects.retain(|ho| ho.end_time >= window.start_ms && ho.start_time <= window.end_ms);
    for ho in hit_objects.iter_mut() {
        ho.start_time = (ho.start_time - window.start_ms).clamp(0, duration);
        ho.end_time = (ho.end_time - window.start_ms).clamp(ho.start_time, duration);
    }
}

fn build_png_layout(
    key_count: i32,
    beatmap_duration: i64,
    chart_end_time: i64,
    chart_start_time: i64,
) -> Result<ManiaPngLayout> {
    let skin_config =
        super::skin::load_mania_skin_config(key_count, crate::export::geometry::OutputFormat::Png);
    let output_scale = crate::export::geometry::output_scale(
        crate::export::geometry::GameMode::Mania,
        crate::export::geometry::OutputFormat::Png,
    );
    let (lane_widths, lane_left_offsets, skin_lane_area_width) =
        super::animation::build_scaled_columns(
            &skin_config.column_widths,
            &skin_config.column_line_widths,
            output_scale,
        );
    let logical_pixels_per_ms = crate::config::current()
        .render
        .mania
        .png
        .sizing
        .PIXELS_PER_MS
        / output_scale;
    let logical_chart_height = (chart_end_time as f64 * logical_pixels_per_ms).ceil() as i64;
    let total_chart_height =
        crate::export::geometry::scale_px(logical_chart_height as f64, output_scale).max(1);
    let column_count = calculate_column_count(beatmap_duration, total_chart_height)?;
    let time_per_column = ceil_div(chart_end_time, column_count);
    let logical_column_height = (time_per_column as f64 * logical_pixels_per_ms).ceil() as i64;
    let column_height =
        crate::export::geometry::scale_px(logical_column_height as f64, output_scale).max(1);
    let top_buffer = crate::export::geometry::scale_px(
        crate::export::geometry::scale_px(crate::export::mania::constants::TOP_BUFFER as f64, 0.5)
            as f64,
        output_scale,
    );
    let total_column_height = top_buffer + column_height;
    let lane_area_width = skin_lane_area_width
        + (key_count as i64 - 1) * crate::config::current().render.mania.png.sizing.LANE_GAP;
    let column_width = crate::config::current()
        .render
        .mania
        .png
        .sizing
        .LEFT_PANEL_WIDTH
        + lane_area_width;
    let image_width = crate::config::current()
        .render
        .mania
        .png
        .sizing
        .PAGE_MARGIN_LEFT
        + crate::config::current()
            .render
            .mania
            .png
            .sizing
            .PAGE_MARGIN_RIGHT
        + column_count
            * (crate::config::current()
                .render
                .mania
                .png
                .sizing
                .INFO_MARGIN_LEFT
                + column_width
                + crate::config::current()
                    .render
                    .mania
                    .png
                    .sizing
                    .INFO_MARGIN_RIGHT)
        + (column_count - 1) * crate::config::current().render.mania.png.sizing.COLUMN_GAP;
    let image_height = crate::config::current()
        .render
        .mania
        .png
        .sizing
        .PAGE_MARGIN_TOP
        + crate::config::current()
            .render
            .mania
            .png
            .sizing
            .PAGE_MARGIN_BOTTOM
        + crate::config::current()
            .render
            .mania
            .png
            .sizing
            .INFO_MARGIN_TOP
        + total_column_height
        + crate::config::current()
            .render
            .mania
            .png
            .sizing
            .INFO_MARGIN_BOTTOM;
    Ok(ManiaPngLayout {
        column_count,
        time_per_column,
        column_height,
        total_column_height,
        lane_area_width,
        column_width,
        lane_widths,
        lane_left_offsets,
        top_buffer,
        image_width,
        image_height,
        chart_start_time,
    })
}

fn ceil_div(a: i64, b: i64) -> i64 {
    (a + b - 1).div_euclid(b)
}

fn calculate_column_count(beatmap_duration: i64, total_chart_height: i64) -> Result<i64> {
    if beatmap_duration
        >= crate::config::current()
            .render
            .mania
            .png
            .style
            .MAX_SUPPORTED_DURATION_MS
    {
        return Err(
            osu_beatmap_preview_core::support::error::PreviewError::render(
                "songs longer than 10 minutes are not supported",
            ),
        );
    }
    if beatmap_duration >= 6 * 60 * 1000 {
        return Ok(crate::config::current()
            .render
            .mania
            .png
            .structure
            .FIXED_COLUMN_COUNT_6_TO_10_MINUTES);
    }
    let max_area_height = resolve_max_area_height(beatmap_duration);
    Ok(ceil_div(total_chart_height, max_area_height).max(1))
}

fn resolve_max_area_height(beatmap_duration: i64) -> i64 {
    if beatmap_duration < 60 * 1000 {
        crate::config::current()
            .render
            .mania
            .png
            .sizing
            .MAX_AREA_HEIGHT_0_TO_1_MINUTES
    } else if beatmap_duration < 2 * 60 * 1000 {
        crate::config::current()
            .render
            .mania
            .png
            .sizing
            .MAX_AREA_HEIGHT_1_TO_2_MINUTES
    } else if beatmap_duration < 3 * 60 * 1000 {
        crate::config::current()
            .render
            .mania
            .png
            .sizing
            .MAX_AREA_HEIGHT_2_TO_3_MINUTES
    } else if beatmap_duration < 4 * 60 * 1000 {
        crate::config::current()
            .render
            .mania
            .png
            .sizing
            .MAX_AREA_HEIGHT_3_TO_4_MINUTES
    } else if beatmap_duration < 5 * 60 * 1000 {
        crate::config::current()
            .render
            .mania
            .png
            .sizing
            .MAX_AREA_HEIGHT_4_TO_5_MINUTES
    } else {
        crate::config::current()
            .render
            .mania
            .png
            .sizing
            .MAX_AREA_HEIGHT_5_TO_6_MINUTES
    }
}

fn build_timing_lines(
    timing_points: &[TimingPoint],
    chart_end_time: i64,
    beat_divisor: i32,
    first_note_time: i64,
) -> Vec<TimingLine> {
    let base_points: Vec<&TimingPoint> = timing_points.iter().filter(|p| p.uninherited).collect();
    if base_points.is_empty() {
        return Vec::new();
    }

    let mut ordered_unique: BTreeMap<i64, TimingLine> = BTreeMap::new();
    for (index, point) in base_points.iter().enumerate() {
        let segment_end = if index + 1 < base_points.len() {
            base_points[index + 1].time.trunc() as i64 as f64
        } else {
            chart_end_time as f64
        };

        let beat_pixels = point.beat_length
            * crate::config::current()
                .render
                .mania
                .png
                .sizing
                .PIXELS_PER_MS;
        let subdivision: i64 = if beat_divisor > 0 {
            (beat_divisor as i64).max(1)
        } else if beat_pixels >= 72.0 {
            4
        } else if beat_pixels >= 28.0 {
            2
        } else {
            1
        };
        let step = point.beat_length / subdivision as f64;
        // NaN 步长直接继续（与 Python 一样只输出一条线）；零或负值另行处理。
        // 该步长会造成无限循环，因此跳过。
        if step <= 0.0 {
            continue;
        }
        let bar_modulo = (subdivision * point.meter as i64).max(1);
        let mut step_index: i64 = 0;
        let mut current = point.time;

        while current <= segment_end + 0.001 {
            if current >= 0.0 {
                let is_bar = step_index % bar_modulo == 0;
                let is_beat = step_index % subdivision == 0;
                ordered_unique.insert(
                    round_half_even(current),
                    TimingLine {
                        time: round_half_even(current),
                        color: if is_bar {
                            crate::config::current()
                                .render
                                .mania
                                .png
                                .style
                                .MEASURE_LINE_COLOR
                        } else if is_beat {
                            crate::config::current()
                                .render
                                .mania
                                .png
                                .style
                                .BEAT_LINE_COLOR
                        } else {
                            crate::config::current()
                                .render
                                .mania
                                .png
                                .style
                                .SUBDIVISION_LINE
                        },
                        show_label: is_bar || is_beat,
                        bpm_label: None,
                    },
                );
            }
            step_index += 1;
            current = point.time + step_index as f64 * step;
        }
    }
    // 添加 BPM 标签：BPM 变化时标在每条红线的第一条小节线，
    // 并在最接近首个音符的小节线上标记。
    if !ordered_unique.is_empty() {
        let mut last_bpm: Option<f64> = None;
        for point in &base_points {
            let bpm = 60_000.0 / point.beat_length;
            let bpm_changed = last_bpm.is_none_or(|prev| (bpm - prev).abs() > 0.01);
            last_bpm = Some(bpm);

            if bpm_changed {
                // 查找该红线时间点或之后的第一条小节线。
                let rounded = round_half_even(point.time);
                let key = ordered_unique
                    .range(rounded..)
                    .next()
                    .map(|(&k, _)| k)
                    .unwrap_or(rounded);
                if let Some(line) = ordered_unique.get_mut(&key) {
                    line.bpm_label = Some(format!("{:.0}BPM", bpm.round()));
                }
            }
        }

        // 首个音符 BPM：使用 first_note_time 时生效的 BPM。
        if first_note_time > 0 {
            let bpm = 60_000.0
                / base_points
                    .iter()
                    .rfind(|p| p.time <= first_note_time as f64)
                    .map_or(base_points[0].beat_length, |p| p.beat_length);
            let key = ordered_unique
                .range(first_note_time..)
                .next()
                .map(|(&k, _)| k);
            if let Some(k) = key {
                if let Some(line) = ordered_unique.get_mut(&k) {
                    // 仅当同一时刻尚未因 BPM 变化添加标签时才附加。
                    if line.bpm_label.is_none() {
                        line.bpm_label = Some(format!("{:.0}BPM", bpm.round()));
                    }
                }
            }
        }
    }

    ordered_unique.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mania_note(start_time: i64, end_time: i64) -> ManiaHitObject {
        ManiaHitObject {
            lane: 0,
            start_time,
            end_time,
            is_long_note: end_time > start_time,
            samples: Vec::new(),
        }
    }

    #[test]
    fn window_keeps_overlapping_objects_and_clips_holds() {
        let window = PngWindow {
            start_ms: 1_000,
            end_ms: 3_000,
        };
        let mut hit_objects = vec![
            mania_note(0, 500),       // 窗口前：丢弃
            mania_note(2_000, 2_000), // 窗口内圆点：平移到 0 基准
            mania_note(500, 2_500),   // 跨窗口起点：长按头部截到窗口起点
            mania_note(2_500, 4_000), // 跨窗口终点：长按尾部截到窗口终点
            mania_note(3_500, 3_500), // 窗口后：丢弃
        ];

        window_hit_objects(&mut hit_objects, window);

        let times: Vec<(i64, i64)> = hit_objects
            .iter()
            .map(|ho| (ho.start_time, ho.end_time))
            .collect();
        assert_eq!(times, vec![(1_000, 1_000), (0, 1_500), (1_500, 2_000)]);
    }

    #[test]
    fn full_chart_window_shifts_every_object_without_clipping() {
        let window = PngWindow {
            start_ms: 500,
            end_ms: 5_000,
        };
        let mut hit_objects = vec![
            mania_note(500, 500),
            mania_note(1_000, 2_000),
            mania_note(4_500, 5_000),
        ];

        window_hit_objects(&mut hit_objects, window);

        let times: Vec<(i64, i64)> = hit_objects
            .iter()
            .map(|ho| (ho.start_time, ho.end_time))
            .collect();
        assert_eq!(times, vec![(0, 0), (500, 1_500), (4_000, 4_500)]);
    }
}
