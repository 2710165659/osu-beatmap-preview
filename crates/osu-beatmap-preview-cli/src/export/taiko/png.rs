//! osu!taiko PNG 静态图导出：计算行布局并交给 core 绘制，然后编码保存。

use crate::media::image;
use osu_beatmap_preview_core::model::mods::ModSettings;
use osu_beatmap_preview_core::model::Beatmap;
use osu_beatmap_preview_core::processing::parse::round_half_even;
use osu_beatmap_preview_core::processing::timeline::TimeAxis;
use osu_beatmap_preview_core::render::cpu::modes::taiko::png::{
    render_taiko_png_scene, TaikoPngLayout,
};
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
use osu_beatmap_preview_core::support::timeout::RequestDeadline;
use std::collections::HashMap;
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
    deadline: &RequestDeadline,
) -> Result<PathBuf> {
    deadline.check()?;
    let mut hit_objects = apply_taiko_object_mods(taiko_hit_objects(beatmap), mods);
    if hit_objects.is_empty() {
        return Err(PreviewError::render("taiko beatmap has no hit objects"));
    }

    let chart_end_time = hit_objects.iter().map(|h| h.end_time).max().unwrap();
    if chart_end_time
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

    // 始终裁剪开头的静音，直接从第一个音符开始。
    let first_note_time = hit_objects.iter().map(|h| h.start_time).min().unwrap_or(0);
    let chart_start_time = osu_beatmap_preview_core::processing::timeline::snap_to_beat_grid(
        first_note_time,
        &beatmap.timing_points,
    );

    let effective_chart_end_time: i64;
    if chart_start_time > 0 {
        for ho in &mut hit_objects {
            ho.start_time = (ho.start_time - chart_start_time).max(0);
            ho.end_time = (ho.end_time - chart_start_time).max(ho.start_time);
        }
        effective_chart_end_time = (chart_end_time - chart_start_time).max(0);
    } else {
        effective_chart_end_time = chart_end_time;
    }

    let slider_multiplier = effective_slider_multiplier(beatmap, mods)?;
    let mut timing_points = effective_timing_points(beatmap, mods);
    if chart_start_time > 0 {
        for tp in &mut timing_points {
            tp.time -= chart_start_time as f64;
        }
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
    let layout = build_png_layout(
        effective_chart_end_time,
        mapper.end_position(),
        &redline_sections,
        &timing_lines,
        chart_start_time,
    );
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
/// 计算每行的起始滚动位置，使行首对齐到小节线。
///
/// 从位置 0 开始，每行最多容纳 `max_row_width` 像素；下一行的起点取
/// 「不超过 当前行起点 + max_row_width 的最后一条小节线」。如果该范围内
/// 没有小节线（极端情况），则退化为定宽切分以保证推进。
pub(crate) fn compute_row_start_positions(
    measure_positions: &[f64],
    chart_width: f64,
    max_row_width: i64,
) -> Vec<f64> {
    let mut starts = vec![0.0f64];
    let width = max_row_width as f64;
    loop {
        let current = *starts.last().unwrap();
        if current + width >= chart_width {
            break;
        }
        // 在 (current, current+width] 范围内找最后一条小节线作为下一行起点
        let next = measure_positions
            .iter()
            .copied()
            .filter(|&p| p > current + 1.0 && p <= current + width)
            .fold(f64::NEG_INFINITY, f64::max);
        if next.is_finite() {
            starts.push(next);
        } else {
            starts.push(current + width);
        }
    }
    starts
}

fn build_png_layout(
    beatmap_duration: i64,
    chart_width: f64,
    redline_sections: &[RedlineSection],
    timing_lines: &[TimingLine],
    chart_start_time: i64,
) -> TaikoPngLayout {
    let base_row_width = resolve_base_row_width(beatmap_duration);
    let bpm_width_multiplier = if crate::config::current()
        .render
        .taiko
        .png
        .style
        .SPACING_PER_BPM
        > 0.0
    {
        1.0
    } else {
        resolve_row_width_bpm_multiplier(redline_sections)
    };
    let max_row_width = pyround(base_row_width as f64 * bpm_width_multiplier);

    // 行首对齐：以小节线位置作为换行锚点
    let measure_positions: Vec<f64> = timing_lines
        .iter()
        .filter(|l| l.is_measure)
        .map(|l| l.position)
        .collect();
    let row_start_positions =
        compute_row_start_positions(&measure_positions, chart_width, max_row_width);
    let row_count = row_start_positions.len() as i64;

    // 实际使用的最大行宽：小节对齐后每行都比 max_row_width 短，
    // 画布宽度按真实内容收缩，避免右侧留下整片空白。
    let mut used_row_width = 0.0f64;
    for (i, &start) in row_start_positions.iter().enumerate() {
        let end = if i + 1 < row_start_positions.len() {
            row_start_positions[i + 1]
        } else {
            chart_width
        };
        used_row_width = used_row_width.max(end - start);
    }
    let used_row_width = (used_row_width.ceil() as i64).clamp(1, max_row_width);

    let content_width = crate::config::current()
        .render
        .taiko
        .png
        .sizing
        .ROW_INNER_PADDING_X
        * 2
        + used_row_width;
    let image_width = crate::config::current()
        .render
        .taiko
        .png
        .sizing
        .PAGE_MARGIN_LEFT
        + crate::config::current()
            .render
            .taiko
            .png
            .sizing
            .PAGE_MARGIN_RIGHT
        + content_width;
    let config = &crate::config::current().render.taiko.png;
    let image_height = config.sizing.PAGE_MARGIN_TOP
        + config.sizing.PAGE_MARGIN_BOTTOM
        + config.sizing.INFO_MARGIN_TOP
        + row_count * (config.sizing.ROW_HEIGHT + config.sizing.INFO_MARGIN_BOTTOM)
        + (row_count - 1).max(0) * config.sizing.ROW_GAP;
    let normal_note_diameter = pyround(
        crate::config::current().render.taiko.png.sizing.ROW_HEIGHT as f64
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
    }
}

fn resolve_base_row_width(beatmap_duration: i64) -> i64 {
    if beatmap_duration < 60_000 {
        return crate::config::current()
            .render
            .taiko
            .png
            .sizing
            .BASE_ROW_WIDTH_0_TO_1_MINUTES;
    }
    if beatmap_duration < 2 * 60_000 {
        return crate::config::current()
            .render
            .taiko
            .png
            .sizing
            .BASE_ROW_WIDTH_1_TO_2_MINUTES;
    }
    if beatmap_duration < 3 * 60_000 {
        return crate::config::current()
            .render
            .taiko
            .png
            .sizing
            .BASE_ROW_WIDTH_2_TO_3_MINUTES;
    }
    if beatmap_duration < 4 * 60_000 {
        return crate::config::current()
            .render
            .taiko
            .png
            .sizing
            .BASE_ROW_WIDTH_3_TO_4_MINUTES;
    }
    if beatmap_duration < 5 * 60_000 {
        return crate::config::current()
            .render
            .taiko
            .png
            .sizing
            .BASE_ROW_WIDTH_4_TO_5_MINUTES;
    }
    if beatmap_duration < 6 * 60_000 {
        return crate::config::current()
            .render
            .taiko
            .png
            .sizing
            .BASE_ROW_WIDTH_5_TO_6_MINUTES;
    }
    crate::config::current()
        .render
        .taiko
        .png
        .sizing
        .BASE_ROW_WIDTH_6_TO_10_MINUTES
}

fn resolve_row_width_bpm_multiplier(redline_sections: &[RedlineSection]) -> f64 {
    let main_bpm = resolve_main_bpm(redline_sections);
    if main_bpm < 180.0 {
        return crate::config::current()
            .render
            .taiko
            .png
            .style
            .ROW_WIDTH_MULTIPLIER_BPM_0_TO_180;
    }
    if main_bpm < 240.0 {
        return crate::config::current()
            .render
            .taiko
            .png
            .style
            .ROW_WIDTH_MULTIPLIER_BPM_180_TO_240;
    }
    if main_bpm < 300.0 {
        return crate::config::current()
            .render
            .taiko
            .png
            .style
            .ROW_WIDTH_MULTIPLIER_BPM_240_TO_300;
    }
    crate::config::current()
        .render
        .taiko
        .png
        .style
        .ROW_WIDTH_MULTIPLIER_BPM_300_PLUS
}

fn resolve_main_bpm(redline_sections: &[RedlineSection]) -> f64 {
    // 按区段时长对四舍五入后的 BPM 加权，选择占比最大的值。
    // 相同权重时取先插入的值，匹配 Python 按插入顺序执行 max 的行为。
    let mut order: Vec<i64> = Vec::new();
    let mut weighted: HashMap<i64, i64> = HashMap::new();
    for section in redline_sections {
        let bpm = pyround(60_000.0 / section.beat_length);
        let duration = (section.end_time - section.start_time).max(0);
        if !weighted.contains_key(&bpm) {
            order.push(bpm);
        }
        *weighted.entry(bpm).or_insert(0) += duration;
    }

    if order.is_empty() {
        return 120.0;
    }
    let mut best_bpm = order[0];
    let mut best_duration = weighted[&order[0]];
    for &bpm in &order[1..] {
        if weighted[&bpm] > best_duration {
            best_duration = weighted[&bpm];
            best_bpm = bpm;
        }
    }
    best_bpm as f64
}

#[cfg(test)]
mod tests {
    use super::compute_row_start_positions;

    #[test]
    fn row_break_measure_anchors_are_scale_invariant_for_all_bpm_tiers() {
        let logical_measures = [0.0, 600.0, 1_200.0, 1_800.0, 2_400.0, 3_000.0, 3_600.0];
        let logical_chart_width = 3_900.0;

        for multiplier in [1.0, 1.15, 1.3, 1.45] {
            let logical_row_width = 1_300.0 * multiplier;
            let baseline = compute_row_start_positions(
                &logical_measures,
                logical_chart_width,
                osu_beatmap_preview_core::processing::parse::round_half_even(logical_row_width),
            );

            for scale in [0.5, 1.0, 1.5, 2.0] {
                let measures: Vec<f64> = logical_measures
                    .iter()
                    .map(|position| position * scale)
                    .collect();
                let starts = compute_row_start_positions(
                    &measures,
                    logical_chart_width * scale,
                    osu_beatmap_preview_core::processing::parse::round_half_even(
                        logical_row_width * scale,
                    ),
                );

                assert_eq!(starts.len(), baseline.len());
                for (actual, expected) in starts.iter().zip(&baseline) {
                    assert!((actual / scale - expected).abs() <= 1.0);
                }
            }
        }
    }
}
