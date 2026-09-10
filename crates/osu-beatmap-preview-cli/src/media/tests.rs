#![cfg(test)]
use super::*;
use osu_beatmap_preview_core::model::{HitObjects, KvSection};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn beatmap_with_preview(preview_time: Option<&str>, lead_in: Option<&str>) -> Beatmap {
    let mut general = KvSection::default();
    if let Some(value) = preview_time {
        general.insert("PreviewTime", value.to_string());
    }
    if let Some(value) = lead_in {
        general.insert("AudioLeadIn", value.to_string());
    }
    Beatmap {
        metadata: KvSection::default(),
        difficulty: KvSection::default(),
        general,
        timing_points: Vec::new(),
        hit_objects: HitObjects::Standard(Vec::new()),
        break_periods: Vec::new(),
        background_filename: None,
        combo_colors: Vec::new(),
        beat_divisor: 0,
    }
}

#[test]
fn preview_start_uses_preview_time_and_duration() {
    let beatmap = beatmap_with_preview(Some("45000"), None);
    let range = resolve_video_time_range(
        &beatmap,
        10_000,
        100_000,
        Some(TimePoint::Preview),
        Some(30.0),
        1.0,
    )
    .unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 45_000,
            end: 75_000
        }
    );
}

#[test]
fn default_start_uses_requested_duration_on_a_long_chart() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(&beatmap, 10_000, 100_000, None, Some(60.0), 1.0).unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 10_000,
            end: 70_000
        }
    );
}

#[test]
fn numeric_start_shifts_backward_when_it_runs_past_chart_tail() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(
        &beatmap,
        10_000,
        100_000,
        Some(TimePoint::Seconds(50.0)),
        Some(60.0),
        1.0,
    )
    .unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 42_000,
            end: 102_000
        }
    );
}

#[test]
fn short_chart_returns_full_playable_range_instead_of_padding_to_duration() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(&beatmap, 10_000, 20_000, None, Some(60.0), 1.0).unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 8_000,
            end: 22_000
        }
    );
}

#[test]
fn short_chart_uses_full_range_even_with_a_negative_requested_start() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(
        &beatmap,
        10_000,
        20_000,
        Some(TimePoint::Seconds(-20.0)),
        Some(60.0),
        1.0,
    )
    .unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 8_000,
            end: 22_000
        }
    );
}

#[test]
fn negative_start_is_preserved_when_tail_adjustment_is_not_needed() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(
        &beatmap,
        10_000,
        100_000,
        Some(TimePoint::Seconds(-20.0)),
        Some(60.0),
        1.0,
    )
    .unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: -10_000,
            end: 50_000
        }
    );
}

#[test]
fn speed_multiplier_scales_chart_span() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(&beatmap, 10_000, 100_000, None, Some(30.0), 1.5).unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 10_000,
            end: 55_000
        }
    );
}

#[test]
fn numeric_start_is_relative_to_first_object() {
    let beatmap = beatmap_with_preview(None, None);
    let range = resolve_video_time_range(
        &beatmap,
        10_000,
        100_000,
        Some(TimePoint::Seconds(-2.0)),
        Some(10.0),
        1.5,
    )
    .unwrap();
    assert_eq!(
        range,
        VideoTimeRange {
            start: 8_000,
            end: 23_000
        }
    );
}

#[test]
fn progress_label_uses_current_skin_time_and_full_playable_duration() {
    let time_axis = TimeAxis::new(12_500);
    let total_ms = time_axis.to_display(102_500);

    assert_eq!(total_ms, 90_000);
    assert_eq!(
        format_progress_label(time_axis.to_display(12_000), total_ms),
        "-0:01/1:30"
    );
    assert_eq!(
        format_progress_label(time_axis.to_display(92_500), total_ms),
        "1:20/1:30"
    );
}

#[test]
fn video_background_fits_entire_image_and_keeps_black_borders() {
    let mut source = Img::new(4, 2, [255, 100, 0, 255]);
    for y in 0..2 {
        for x in 2..4 {
            source.put(x, y, [0, 100, 255, 255]);
        }
    }
    let background = prepare_video_background(
        &source,
        4,
        4,
        video_style(crate::export::geometry::GameMode::Standard),
    );
    assert_eq!((background.w, background.h), (4, 4));
    assert_eq!(background.get(0, 0), [0, 0, 0, 255]);
    assert_eq!(background.get(0, 1), [77, 30, 0, 255]);
    assert_eq!(background.get(0, 2), [77, 30, 0, 255]);
    assert_eq!(background.get(3, 1), [0, 30, 77, 255]);
    assert_eq!(background.get(0, 3), [0, 0, 0, 255]);
}

#[test]
fn dropping_audio_task_cancels_and_joins_worker() {
    let deadline = RequestDeadline::new(Instant::now(), "mp4", Duration::from_secs(300));
    let worker_deadline = deadline.clone();
    let finished = Arc::new(AtomicBool::new(false));
    let worker_finished = finished.clone();
    let task = JoinedAudioTask::new(
        std::thread::spawn(move || loop {
            if let Err(error) = worker_deadline.check() {
                worker_finished.store(true, Ordering::Relaxed);
                return Err(error);
            }
            std::thread::yield_now();
        }),
        deadline,
    );

    drop(task);
    assert!(finished.load(Ordering::Relaxed));
}
