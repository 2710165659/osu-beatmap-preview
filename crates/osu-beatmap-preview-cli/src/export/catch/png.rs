//! osu!catch PNG 静态图导出：计算布局、准备渲染对象，交给 core 绘制后保存。

use crate::media::image::save_png;
use osu_beatmap_preview_core::model::mods::ModSettings;
use osu_beatmap_preview_core::model::{Beatmap, TimingPoint};
use osu_beatmap_preview_core::processing::parse::round_half_even;
use osu_beatmap_preview_core::processing::timeline::TimeAxis;
use osu_beatmap_preview_core::render::cpu::modes::catch::png::{
    ceil_div, predominant_measure_aligned_height, render_catch_png_scene, CatchPngLayout,
    RedlineSection, TimingLine,
};
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
use osu_beatmap_preview_core::support::timeout::RequestDeadline;
use std::path::{Path, PathBuf};

use super::objects::{build_catch_render_objects, effective_difficulty};

pub(crate) fn rhe(v: f64) -> i64 {
    round_half_even(v)
}

// ─── 布局 ───

fn pixels_per_ms_for_ar(approach_rate: f64, playfield_scale: f64) -> f64 {
    let time_range = super::objects::catch_time_range(approach_rate);
    let visible_fall_height = (crate::export::catch::constants::STABLE_CATCHER_Y
        - crate::export::catch::constants::STABLE_FRUIT_START_Y)
        * playfield_scale;
    visible_fall_height / time_range
}

fn resolve_max_area_height(beatmap_duration: i64) -> i64 {
    if beatmap_duration < 60_000 {
        crate::config::current()
            .render
            .catch
            .png
            .sizing
            .MAX_AREA_HEIGHT_0_TO_1_MINUTES
    } else if beatmap_duration < 2 * 60_000 {
        crate::config::current()
            .render
            .catch
            .png
            .sizing
            .MAX_AREA_HEIGHT_1_TO_2_MINUTES
    } else if beatmap_duration < 3 * 60_000 {
        crate::config::current()
            .render
            .catch
            .png
            .sizing
            .MAX_AREA_HEIGHT_2_TO_3_MINUTES
    } else if beatmap_duration < 4 * 60_000 {
        crate::config::current()
            .render
            .catch
            .png
            .sizing
            .MAX_AREA_HEIGHT_3_TO_4_MINUTES
    } else if beatmap_duration < 5 * 60_000 {
        crate::config::current()
            .render
            .catch
            .png
            .sizing
            .MAX_AREA_HEIGHT_4_TO_5_MINUTES
    } else {
        crate::config::current()
            .render
            .catch
            .png
            .sizing
            .MAX_AREA_HEIGHT_5_TO_6_MINUTES
    }
}

fn build_layout(
    beatmap_duration: i64,
    circle_size: f64,
    approach_rate: f64,
    chart_start_time: i64,
    timing_lines: &[TimingLine],
) -> Result<CatchPngLayout> {
    if beatmap_duration
        >= crate::config::current()
            .render
            .catch
            .png
            .style
            .MAX_SUPPORTED_DURATION_MS
    {
        return Err(PreviewError::render(
            "songs longer than 10 minutes are not supported",
        ));
    }
    let render_scale = crate::export::geometry::output_scale(
        crate::export::geometry::GameMode::Catch,
        crate::export::geometry::OutputFormat::Png,
    );
    let visible_playfield_width = crate::export::geometry::scale_px(
        crate::export::catch::constants::PLAYFIELD_DISPLAY_WIDTH as f64,
        render_scale,
    );
    let playfield_scale =
        visible_playfield_width as f64 / crate::export::catch::constants::PLAYFIELD_WIDTH;
    let object_scale = super::objects::circle_scale(circle_size);

    // 纵向密度上限：限制谱面总像素高度，防止高 AR + 长曲导致内存爆炸
    let mut pixels_per_ms = pixels_per_ms_for_ar(approach_rate, playfield_scale);
    let natural_height = beatmap_duration as f64 * pixels_per_ms;
    if natural_height
        > crate::config::current()
            .render
            .catch
            .png
            .sizing
            .MAX_TOTAL_CHART_HEIGHT as f64
    {
        pixels_per_ms *= crate::config::current()
            .render
            .catch
            .png
            .sizing
            .MAX_TOTAL_CHART_HEIGHT as f64
            / natural_height;
    }

    let total_chart_height = rhe(beatmap_duration as f64 * pixels_per_ms).max(1);
    let max_area_height = resolve_max_area_height(beatmap_duration);
    let aligned_height =
        predominant_measure_aligned_height(timing_lines, pixels_per_ms, max_area_height)
            .unwrap_or(max_area_height);
    let total_column_height = total_chart_height.min(aligned_height).max(1);
    let column_count = ceil_div(total_chart_height, total_column_height).max(1);
    let config = &crate::config::current().render.catch.png;
    let unit_width = config.sizing.INFO_MARGIN_LEFT
        + config.sizing.COLUMN_WIDTH
        + config.sizing.INFO_MARGIN_RIGHT;
    let image_width = config.sizing.PAGE_MARGIN_LEFT
        + config.sizing.PAGE_MARGIN_RIGHT
        + column_count * unit_width
        + (column_count - 1) * config.sizing.COLUMN_GAP;
    let image_height = config.sizing.PAGE_MARGIN_TOP
        + config.sizing.PAGE_MARGIN_BOTTOM
        + config.sizing.INFO_MARGIN_TOP
        + total_column_height
        + config.sizing.INFO_MARGIN_BOTTOM;
    Ok(CatchPngLayout {
        column_count,
        total_column_height,
        visible_playfield_width,
        image_width,
        image_height,
        playfield_scale,
        object_scale,
        pixels_per_ms,
        chart_start_time,
    })
}

/// 让每列高度尽量成为主要小节间隔的整数倍，使各列的主小节线纵向对齐。
fn build_timing_lines(timing_points: &[TimingPoint], chart_end_time: i64) -> Vec<TimingLine> {
    let red_lines: Vec<&TimingPoint> = timing_points
        .iter()
        .filter(|p| p.uninherited && p.beat_length.is_finite() && p.beat_length > 0.0)
        .collect();
    if red_lines.is_empty() {
        return Vec::new();
    }

    // 切分红线区段（首段从 0 或首条红线之前开始，沿用首条红线参数）
    let mut sections: Vec<RedlineSection> = Vec::new();
    for (index, point) in red_lines.iter().enumerate() {
        let start = if index == 0 {
            point.time.min(0.0)
        } else {
            point.time
        };
        let end = if index + 1 < red_lines.len() {
            red_lines[index + 1].time
        } else {
            chart_end_time as f64
        };
        if end <= start {
            continue;
        }
        sections.push(RedlineSection {
            start_time: if index == 0 { point.time } else { start },
            end_time: end,
            beat_length: point.beat_length.max(1.0),
            meter: point.meter.max(1),
        });
    }

    let mut lines: Vec<TimingLine> = Vec::new();
    let mut last_bpm: Option<f64> = None;
    for section in &sections {
        let bpm = 60_000.0 / section.beat_length;
        let show_bpm = last_bpm.is_none_or(|last| (last - bpm).abs() > 0.01);
        last_bpm = Some(bpm);
        let mut beat_index: i64 = 0;
        loop {
            let time = section.start_time + beat_index as f64 * section.beat_length;
            if time > section.end_time + 0.001 || time > chart_end_time as f64 {
                break;
            }
            if time >= 0.0 {
                lines.push(TimingLine {
                    time: rhe(time),
                    is_measure: beat_index % section.meter as i64 == 0,
                    show_label: true,
                    bpm: (show_bpm && beat_index == 0).then_some(bpm),
                });
            }
            beat_index += 1;
        }
    }
    if let Some(first_visible) = lines.iter_mut().find(|line| line.show_label) {
        if first_visible.bpm.is_none() {
            first_visible.bpm = crate::export::timing::bpm_at(timing_points, first_visible.time);
        }
    }
    lines
}

pub(crate) fn render_catch_grid(
    beatmap: &Beatmap,
    output_path: &Path,
    mods: Option<&ModSettings>,
    time_axis: TimeAxis,
    deadline: &RequestDeadline,
) -> Result<PathBuf> {
    deadline.check()?;
    let hit_objects = match beatmap.hit_objects.as_catch() {
        Some(v) if !v.is_empty() => v,
        _ => return Err(PreviewError::render("catch beatmap has no hit objects")),
    };

    let difficulty = effective_difficulty(beatmap, mods);
    let mut render_objects = build_catch_render_objects(
        beatmap,
        hit_objects,
        mods,
        &difficulty,
        crate::config::current()
            .render
            .catch
            .png
            .style
            .SHOW_BANANA_ROUTE,
    )?;
    deadline.check()?;
    let chart_end_time = hit_objects.iter().map(|h| h.end_time).max().unwrap().max(1);

    // 裁剪开头静音：若第一个音符在 5 秒之后，则从其前 1 秒开始，
    // 并对齐到红线节拍网格。
    let first_note_time = hit_objects.iter().map(|h| h.start_time).min().unwrap_or(0);
    let chart_start_time = if first_note_time >= 5000 {
        osu_beatmap_preview_core::processing::timeline::snap_to_beat_grid(
            first_note_time - 1000,
            &beatmap.timing_points,
        )
    } else {
        0
    };

    let (effective_chart_end_time, timing_points_for_render): (i64, Vec<TimingPoint>) =
        if chart_start_time > 0 {
            for ro in &mut render_objects {
                ro.start_time = (ro.start_time - chart_start_time).max(0);
                if let Some(ref mut et) = ro.event_time {
                    *et = (*et - chart_start_time as f64).max(0.0);
                }
            }
            let tp = beatmap
                .timing_points
                .iter()
                .map(|tp| {
                    let mut tp = *tp;
                    tp.time -= chart_start_time as f64;
                    tp
                })
                .collect();
            ((chart_end_time - chart_start_time).max(0), tp)
        } else {
            (chart_end_time, beatmap.timing_points.clone())
        };

    let timing_lines = build_timing_lines(&timing_points_for_render, effective_chart_end_time);
    let layout = build_layout(
        effective_chart_end_time,
        difficulty.cs,
        difficulty.ar,
        chart_start_time,
        &timing_lines,
    )?;

    let image = render_catch_png_scene(&layout, &render_objects, &timing_lines, time_axis);

    save_png(&image, output_path, deadline)?;
    Ok(output_path.to_path_buf())
}
