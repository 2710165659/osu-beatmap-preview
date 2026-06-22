//! Alpha / timing helpers for osu!standard renderer.

use crate::models::StandardHitObject;

use super::constants::*;
use super::context::RenderSettings;

pub(crate) fn object_alpha(
    start_time: i64,
    end_time: i64,
    snapshot_time: i64,
    settings: &RenderSettings,
) -> f64 {
    if settings.hidden && !settings.traceable {
        hidden_object_alpha(start_time, end_time, snapshot_time, settings)
    } else {
        normal_object_alpha(start_time, end_time, snapshot_time, settings)
    }
}

pub(crate) fn normal_object_alpha(
    start_time: i64,
    end_time: i64,
    snapshot_time: i64,
    settings: &RenderSettings,
) -> f64 {
    if snapshot_time < start_time {
        let fade_start = start_time - settings.preempt_ms;
        return ((snapshot_time - fade_start) as f64 / settings.fade_in_ms).clamp(0.0, 1.0);
    }
    if snapshot_time <= end_time {
        return 1.0;
    }
    (1.0 - (snapshot_time - end_time) as f64 / SLIDER_FADE_OUT_MS as f64).max(0.0)
}

pub(crate) fn hidden_object_alpha(
    start_time: i64,
    end_time: i64,
    snapshot_time: i64,
    settings: &RenderSettings,
) -> f64 {
    if end_time > start_time {
        return hidden_slider_body_alpha(start_time, end_time, snapshot_time, settings);
    }

    let fade_start = (start_time - settings.preempt_ms) as f64;
    let fade_in_end = fade_start + settings.fade_in_ms;
    if snapshot_time < start_time {
        let fade_in_alpha =
            ((snapshot_time as f64 - fade_start) / settings.fade_in_ms.max(1.0)).clamp(0.0, 1.0);
        let fade_out_end = fade_in_end + settings.preempt_ms as f64 * 0.3;
        let fade_out_alpha = (1.0
            - (snapshot_time as f64 - fade_in_end) / (fade_out_end - fade_in_end).max(1.0))
        .clamp(0.0, 1.0);
        return fade_in_alpha.min(fade_out_alpha);
    }
    0.0
}

pub(crate) fn slider_body_alpha(
    hit_object: &StandardHitObject,
    snapshot_time: i64,
    settings: &RenderSettings,
) -> f64 {
    if settings.hidden && !settings.traceable {
        hidden_slider_body_alpha(
            hit_object.start_time,
            hit_object.end_time,
            snapshot_time,
            settings,
        )
    } else {
        normal_object_alpha(
            hit_object.start_time,
            hit_object.end_time,
            snapshot_time,
            settings,
        )
    }
}

pub(crate) fn hidden_slider_body_alpha(
    start_time: i64,
    end_time: i64,
    snapshot_time: i64,
    settings: &RenderSettings,
) -> f64 {
    let fade_start = (start_time - settings.preempt_ms) as f64;
    let fade_in_end = fade_start + settings.fade_in_ms;
    let t = snapshot_time as f64;
    if t < fade_in_end {
        return ((t - fade_start) / settings.fade_in_ms.max(1.0)).clamp(0.0, 1.0);
    }
    if snapshot_time <= end_time {
        return (1.0 - (t - fade_in_end) / (end_time as f64 - fade_in_end).max(1.0))
            .clamp(0.0, 1.0);
    }
    (1.0 - (snapshot_time - end_time) as f64 / SLIDER_FADE_OUT_MS as f64).max(0.0)
}

pub(crate) fn spinner_alpha(
    hit_object: &StandardHitObject,
    snapshot_time: i64,
    settings: &RenderSettings,
) -> f64 {
    if !settings.hidden || settings.traceable {
        return normal_object_alpha(
            hit_object.start_time,
            hit_object.end_time,
            snapshot_time,
            settings,
        );
    }
    let fade_start = hit_object.start_time as f64 - settings.fade_in_ms;
    if snapshot_time < hit_object.start_time {
        return ((snapshot_time as f64 - fade_start) / settings.fade_in_ms.max(1.0))
            .clamp(0.0, 1.0);
    }
    if snapshot_time <= hit_object.end_time {
        return 1.0;
    }
    (1.0
        - (snapshot_time - hit_object.end_time) as f64
            / (settings.preempt_ms as f64 * 0.3).max(1.0))
    .max(0.0)
}

pub(crate) fn slider_head_alpha(
    hit_object: &StandardHitObject,
    snapshot_time: i64,
    settings: &RenderSettings,
    snaked_start: f64,
    snaked_end: f64,
) -> f64 {
    if snaked_start > 0.001 || snaked_end <= 0.001 {
        return 0.0;
    }
    if snapshot_time < hit_object.start_time {
        return object_alpha(
            hit_object.start_time,
            hit_object.start_time,
            snapshot_time,
            settings,
        );
    }
    if settings.hidden && !settings.traceable {
        return 0.0;
    }
    if snapshot_time <= hit_object.start_time + POST_HIT_FADE_MS {
        return 1.0 - (snapshot_time - hit_object.start_time) as f64 / POST_HIT_FADE_MS as f64;
    }
    0.0
}

pub(crate) fn slider_path_progress(span_count: i64, completion: f64) -> f64 {
    let span = ((completion * span_count as f64) as i64).min(span_count - 1);
    let mut progress = (completion * span_count as f64).fract();
    if completion >= 1.0 {
        progress = 1.0;
    }
    if span % 2 == 1 {
        progress = 1.0 - progress;
    }
    progress
}
