//! osu!taiko PNG 静态图导出：计算行布局并交给 core 绘制，然后编码保存。

use osu_beatmap_preview_core::render::cpu::modes::taiko::constants as taiko_constants;

use crate::export::segment::{resolve_png_window, PngSegment, PngWindow};
use crate::media::image;
use osu_beatmap_preview_core::model::mods::ModSettings;
use osu_beatmap_preview_core::model::{Beatmap, TaikoHitObject};
use osu_beatmap_preview_core::processing::parse::round_half_even;
use osu_beatmap_preview_core::processing::timeline::TimeAxis;
use osu_beatmap_preview_core::render::cpu::modes::taiko::png::{
    render_taiko_png_scene, TaikoPngLayout,
};
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
use osu_beatmap_preview_core::support::timeout::RequestDeadline;
use std::path::{Path, PathBuf};

use super::timing::*;

#[inline]
fn pyround(v: f64) -> i64 {
    round_half_even(v)
}

// ─── PNG 布局 ───

pub(crate) fn render_taiko_grid(
    beatmap: &Beatmap,
    output_path: &Path,
    mods: Option<&ModSettings>,
    time_axis: TimeAxis,
    segment: Option<PngSegment>,
    deadline: &RequestDeadline,
) -> Result<PathBuf> {
    deadline.check()?;
    let mut hit_objects = apply_taiko_object_mods(taiko_hit_objects(beatmap), mods);
    if hit_objects.is_empty() {
        return Err(PreviewError::render("taiko beatmap has no hit objects"));
    }

    let chart_end_time = hit_objects.iter().map(|h| h.end_time).max().unwrap();

    // 始终裁剪开头的静音，直接从第一个音符开始。
    let first_note_time = hit_objects.iter().map(|h| h.start_time).min().unwrap_or(0);
    let chart_start_time = osu_beatmap_preview_core::processing::timeline::snap_to_beat_grid(
        first_note_time,
        &beatmap.timing_points,
    );

    // 区间段（--time-points + --duration-time）：窗口规则与 MP4 一致（见 resolve_png_window）。
    let window = resolve_png_window(chart_start_time, chart_end_time, segment)?;
    let chart_start_time = window.start_ms;
    let effective_chart_end_time = window.duration_ms();

    // 10 分钟上限：整谱模式沿用谱面末尾绝对时间的旧判断；
    // 区间段模式只看实际渲染时长，长谱面的片段仍可出图。
    let duration_for_limit = if segment.is_some() {
        effective_chart_end_time
    } else {
        chart_end_time
    };
    if duration_for_limit
        >= crate::config::current()
            .render
            .taiko
            .png
            .style
            .MAX_SUPPORTED_DURATION_MS
    {
        return Err(PreviewError::render(
            "songs longer than 10 minutes are not supported",
        ));
    }

    // 只保留窗口内的物件并平移到窗口起点；整谱模式窗口覆盖全部物件，行为不变。
    window_hit_objects(&mut hit_objects, window);

    let slider_multiplier = effective_slider_multiplier(beatmap, mods)?;
    let mut timing_points = effective_timing_points(beatmap, mods);
    for tp in &mut timing_points {
        tp.time -= chart_start_time as f64;
    }
    // 静态图的 note 间距只跟随红线 BPM（绿线 SV 不影响排版）
    let spacing_timing_points = spacing_timing_points_for_png(&timing_points);
    let render_scale = crate::export::geometry::output_scale(
        crate::export::geometry::GameMode::Taiko,
        crate::export::geometry::OutputFormat::Png,
    );
    let mapper = build_scroll_mapper(
        &spacing_timing_points,
        effective_chart_end_time,
        slider_multiplier,
        crate::config::current()
            .render
            .taiko
            .png
            .style
            .SPACING_PER_BPM,
        render_scale,
    );
    let redline_sections = build_redline_sections(&timing_points, effective_chart_end_time);
    let kiai_sections = build_kiai_sections(&timing_points, effective_chart_end_time);
    let first_note_time = hit_objects.iter().map(|h| h.start_time).min().unwrap_or(0);
    let timing_lines = build_timing_lines(
        &redline_sections,
        &mapper,
        crate::export::taiko::constants::MIN_BEAT_LINE_SPACING * render_scale,
        &kiai_sections,
        first_note_time,
        time_axis.to_display(chart_start_time),
    );
    let beat_positions = png_beat_positions(&redline_sections, &mapper);
    let mut layout = build_png_layout(
        mapper.end_position(),
        &timing_lines,
        &beat_positions,
        chart_start_time,
    );
    layout.overall_difficulty = effective_overall_difficulty(beatmap, mods);
    let sv_changes = build_sv_changes(&timing_points, effective_chart_end_time, &mapper);
    deadline.check()?;

    let image = render_taiko_png_scene(
        &layout,
        &hit_objects,
        &mapper,
        &timing_lines,
        &sv_changes,
        time_axis,
    );

    image::save_png(&image, output_path, deadline)?;
    Ok(output_path.to_path_buf())
}

/// 将物件裁剪进渲染窗口并平移到窗口起点。
///
/// 窗口外的圆点直接丢弃，跨界长条（连打/大连打）按窗口截断，
/// 避免窗口外的物件被挤到边界上；整谱模式窗口覆盖全部物件，结果与不裁剪一致。
fn window_hit_objects(hit_objects: &mut Vec<TaikoHitObject>, window: PngWindow) {
    let duration = window.duration_ms();
    hit_objects.retain(|ho| ho.end_time >= window.start_ms && ho.start_time <= window.end_ms);
    for ho in hit_objects.iter_mut() {
        ho.start_time = (ho.start_time - window.start_ms).clamp(0, duration);
        ho.end_time = (ho.end_time - window.start_ms).clamp(ho.start_time, duration);
    }
}

/// 常规每行四个完整小节；长拍号提前换行，单小节超宽时按拍线续行。
/// 全部换行沿用原始坐标，不缩放行内音符。
fn compute_row_start_positions(
    measure_positions: &[f64],
    chart_width: f64,
    target_width: f64,
    beat_positions: &[f64],
) -> Vec<f64> {
    let mut starts = vec![0.0];
    let mut previous = 0.0;
    let mut count = 0;
    let mut continuing_long_measure = false;
    for end in measure_positions
        .iter()
        .copied()
        .chain(std::iter::once(chart_width))
    {
        if !end.is_finite() || end <= previous || end > chart_width {
            continue;
        }
        if target_width.is_finite() && target_width > 0.0 && end - previous > target_width {
            // 普通小节后单独开始续行；相邻长小节或红线切段不强制清空剩余行宽。
            if !continuing_long_measure && previous > *starts.last().unwrap() {
                starts.push(previous);
            }
            let mut start = *starts.last().unwrap();
            // 连续累加拍长会有微小浮点误差，不能因此漏掉恰好位于行尾的拍线。
            let tolerance = target_width * 1e-9;
            while end - start > target_width + tolerance {
                let limit = start + target_width;
                let index =
                    beat_positions.partition_point(|&position| position <= limit + tolerance);
                let next = index
                    .checked_sub(1)
                    .map(|i| beat_positions[i])
                    .filter(|&position| position.is_finite() && position > start)
                    // 单拍也超宽或没有合法拍线时按行宽续行。
                    .unwrap_or(limit);
                starts.push(next);
                start = next;
            }
            continuing_long_measure = true;
            count = 0;
        } else {
            if continuing_long_measure {
                count = 1;
                continuing_long_measure = false;
            }
            // 长拍号即使单小节未超宽，多个相邻小节也不能一起撑宽整图。
            let start = *starts.last().unwrap();
            if target_width.is_finite()
                && target_width > 0.0
                && end - start > target_width + target_width * 1e-9
                && previous > start
            {
                starts.push(previous);
                count = 0;
            }
            count += 1;
            if count == 4 {
                if end < chart_width {
                    starts.push(end);
                }
                count = 0;
            }
        }
        previous = end;
    }
    starts
}

/// 单独生成换行拍线，避免将激昂/SV 标记误当拍线，也不受绘制用的最小拍线间距影响。
fn png_beat_positions(sections: &[RedlineSection], mapper: &ScrollPositionMapper) -> Vec<f64> {
    let mut positions = Vec::new();
    for section in sections {
        if !section.beat_length.is_finite() || section.beat_length <= 0.0 {
            continue;
        }
        let first = ((-section.start_time as f64 / section.beat_length).ceil()).max(0.0) as i64;
        let last =
            ((section.end_time - section.start_time) as f64 / section.beat_length).floor() as i64;
        for beat in first..=last {
            let position =
                mapper.position_at(section.start_time as f64 + beat as f64 * section.beat_length);
            if positions.last().is_none_or(|&previous| position > previous) {
                positions.push(position);
            }
        }
    }
    positions
}

/// 用四小节分组宽度的上四分位数作为目标，避免少数长拍号撑宽整图。
/// 坐标来自同一滚动映射，不对行内音符做二次缩放。
fn typical_row_width(measure_positions: &[f64], chart_width: f64) -> f64 {
    let mut widths = Vec::new();
    let mut start = 0.0;
    let mut previous = 0.0;
    let mut count = 0;
    for &position in measure_positions {
        if !position.is_finite() || position <= previous || position > chart_width {
            continue;
        }
        previous = position;
        count += 1;
        if count == 4 {
            widths.push(position - start);
            start = position;
            count = 0;
        }
    }
    if widths.is_empty() {
        return chart_width;
    }
    widths.sort_by(f64::total_cmp);
    widths[(widths.len() - 1) * 3 / 4]
}

fn build_png_layout(
    chart_width: f64,
    timing_lines: &[TimingLine],
    beat_positions: &[f64],
    chart_start_time: i64,
) -> TaikoPngLayout {
    let taiko_png_sizing = taiko_constants::png::sizing();
    let measure_positions: Vec<f64> = timing_lines
        .iter()
        .filter(|l| l.is_measure)
        .map(|l| l.position)
        .collect();
    let target_width = typical_row_width(&measure_positions, chart_width);
    let row_start_positions = compute_row_start_positions(
        &measure_positions,
        chart_width,
        target_width,
        beat_positions,
    );
    let row_count = row_start_positions.len() as i64;

    // 超长小节已按拍线续行，画布按实际最长行分配，不压缩音符间距。
    let mut used_row_width = 0.0f64;
    for (i, &start) in row_start_positions.iter().enumerate() {
        let end = if i + 1 < row_start_positions.len() {
            row_start_positions[i + 1]
        } else {
            chart_width
        };
        used_row_width = used_row_width.max(end - start);
    }
    let used_row_width = (used_row_width.ceil() as i64).max(1);
    let max_row_width = used_row_width;

    let content_width = taiko_png_sizing.ROW_INNER_PADDING_X * 2 + used_row_width;
    let image_width =
        taiko_png_sizing.PAGE_MARGIN_LEFT + taiko_png_sizing.PAGE_MARGIN_RIGHT + content_width;
    let image_height = taiko_png_sizing.PAGE_MARGIN_TOP
        + taiko_png_sizing.PAGE_MARGIN_BOTTOM
        + taiko_png_sizing.INFO_MARGIN_TOP
        + row_count * (taiko_png_sizing.ROW_HEIGHT + taiko_png_sizing.INFO_MARGIN_BOTTOM)
        + (row_count - 1).max(0) * taiko_png_sizing.ROW_GAP;
    let normal_note_diameter = pyround(
        taiko_png_sizing.ROW_HEIGHT as f64
            * crate::export::taiko::constants::NORMAL_NOTE_SIZE_RATIO,
    );
    let big_note_diameter =
        pyround(normal_note_diameter as f64 * crate::export::taiko::constants::BIG_NOTE_SCALE);
    TaikoPngLayout {
        row_count,
        max_row_width,
        content_width,
        image_width,
        image_height,
        normal_note_diameter,
        big_note_diameter,
        row_start_positions,
        chart_start_time,
        overall_difficulty: 5.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_measures_keep_four_per_row() {
        let measures: Vec<_> = (0..=10).map(|i| i as f64 * 100.0).collect();
        let width = typical_row_width(&measures, 950.0);
        assert_eq!(width, 400.0);
        assert_eq!(
            compute_row_start_positions(&measures, 950.0, width, &[]),
            vec![0.0, 400.0, 800.0]
        );
        assert_eq!(
            compute_row_start_positions(&measures, 800.0, width, &[]),
            vec![0.0, 400.0]
        );
    }

    #[test]
    fn variable_width_measures_keep_four_per_row_at_every_scale() {
        let measures = [0.0, 100.0, 100.0, 250.0, 400.0, 700.0, 800.0];
        for scale in [0.5, 1.0, 2.0] {
            let scaled: Vec<_> = measures.iter().map(|p| p * scale).collect();
            assert_eq!(
                compute_row_start_positions(&scaled, 900.0 * scale, 800.0 * scale, &[]),
                vec![0.0, 700.0 * scale]
            );
        }
        assert_eq!(
            compute_row_start_positions(&[0.0, 600.0, 700.0], 800.0, 400.0, &[0.0, 300.0, 600.0]),
            vec![0.0, 300.0, 700.0]
        );
        assert_eq!(
            compute_row_start_positions(&[], 100.0, 400.0, &[]),
            vec![0.0]
        );
    }

    #[test]
    fn exceptional_group_does_not_set_target_width() {
        let mut measures: Vec<_> = (0..=16).map(|i| i as f64 * 100.0).collect();
        measures.extend([2000.0, 2400.0, 2800.0, 3200.0]);
        assert_eq!(typical_row_width(&measures, 3200.0), 400.0);
    }
    #[test]
    fn long_measure_keeps_beat_on_row_edge_despite_roundoff() {
        let beats: Vec<_> = (0..=64).map(|beat| beat as f64 * (1.0 + 1e-12)).collect();
        let starts = compute_row_start_positions(&[0.0], 64.0, 16.0, &beats);
        assert_eq!(starts.len(), 4);
        for (index, start) in starts.iter().enumerate() {
            assert!((start - index as f64 * 16.0).abs() < 1e-8);
        }
    }

    #[test]
    fn long_measure_continues_across_redline_without_short_row() {
        // 对应末尾同 BPM 红线：30 拍后重新起算小节，但续行仍应填满 16 拍。
        let beats: Vec<_> = (0..=62).map(|beat| beat as f64).collect();
        assert_eq!(
            compute_row_start_positions(&[0.0, 30.0], 62.0, 16.0, &beats),
            vec![0.0, 16.0, 32.0, 48.0]
        );
    }

    #[test]
    fn adjacent_long_measures_do_not_expand_every_row() {
        assert_eq!(
            compute_row_start_positions(&[0.0, 400.0, 800.0, 1200.0], 1200.0, 400.0, &[]),
            vec![0.0, 400.0, 800.0]
        );
    }

    #[test]
    fn oversized_measure_does_not_expand_every_row() {
        let beats: Vec<_> = (0..=288).map(|i| i as f64 * 25.0).collect();
        let starts = compute_row_start_positions(&[0.0, 400.0, 6800.0], 7200.0, 400.0, &beats);
        assert!(starts.iter().all(|start| beats.contains(start)));
        for pair in starts
            .iter()
            .copied()
            .chain(std::iter::once(7200.0))
            .collect::<Vec<_>>()
            .windows(2)
        {
            assert!(pair[1] - pair[0] <= 400.0, "oversized row: {pair:?}");
        }
    }
    fn taiko_object(start_time: i64, end_time: i64) -> TaikoHitObject {
        TaikoHitObject {
            start_time,
            end_time,
            hit_type: 0,
            hitsound: 0,
            samples: Vec::new(),
        }
    }

    #[test]
    fn window_keeps_overlapping_objects_and_clips_spans() {
        let window = PngWindow {
            start_ms: 1_000,
            end_ms: 3_000,
        };
        let mut hit_objects = vec![
            taiko_object(0, 500),       // 窗口前：丢弃
            taiko_object(2_000, 2_000), // 窗口内圆点：平移到0基准
            taiko_object(0, 2_500),     // 跨窗口起点：头部截到窗口起点
            taiko_object(2_500, 4_000), // 跨窗口终点：尾部截到窗口终点
            taiko_object(3_500, 3_500), // 窗口后：丢弃
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
            taiko_object(500, 500),
            taiko_object(1_000, 2_000),
            taiko_object(4_500, 5_000),
        ];

        window_hit_objects(&mut hit_objects, window);

        let times: Vec<(i64, i64)> = hit_objects
            .iter()
            .map(|ho| (ho.start_time, ho.end_time))
            .collect();
        assert_eq!(times, vec![(0, 0), (500, 1_500), (4_000, 4_500)]);
    }
}
