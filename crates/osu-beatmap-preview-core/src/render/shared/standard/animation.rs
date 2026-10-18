//! CPU 与场景绘制共用的接近圈、折返箭头和休息段动画。

use crate::model::{BreakPeriod, StandardHitObject};
use crate::render::cpu::modes::standard::{constants::*, context::*};

pub(crate) struct ApproachCircle {
    pub center: (f64, f64),
    pub diameter: f64,
    pub opacity: f64,
}

pub(crate) fn approach_circle(
    context: &RenderContext,
    id: usize,
    time: i64,
) -> Option<ApproachCircle> {
    let object = &context.hit_objects[id];
    if context.settings.hidden || time >= object.start_time {
        return None;
    }
    let elapsed = (time - (object.start_time - context.settings.preempt_ms)) as f64;
    let progress = (elapsed / context.settings.preempt_ms as f64).clamp(0.0, 1.0);
    let opacity = 0.9 * (elapsed / (context.settings.fade_in_ms * 2.0).max(1.0)).min(1.0);
    if opacity <= 0.0 {
        return None;
    }
    let world = stacked_position(object, &context.settings);
    Some(ApproachCircle {
        center: to_frame_point(world.0, world.1, &context.frame_layout),
        diameter: context.frame_circle_diameter as f64 * (4.0 - 3.0 * progress),
        opacity,
    })
}

pub(crate) fn reverse_arrow_opacity(
    object: &StandardHitObject,
    time: i64,
    repeat: i64,
    snake_start: f64,
    snake_end: f64,
    opacity: f64,
) -> f64 {
    let position = if repeat % 2 == 1 { 1.0 } else { 0.0 };
    if !(snake_start - 0.001 <= position && position <= snake_end + 0.001) {
        return 0.0;
    }
    let duration = (object.end_time - object.start_time) as f64;
    let spans = object.slider_repeats.max(1) as f64;
    let fade = 300.0_f64.min(duration / spans) / duration.max(1.0);
    let factor = if time < object.start_time {
        if repeat == 1 {
            1.0
        } else {
            0.0
        }
    } else {
        let traversal = (time - object.start_time) as f64
            / (object.end_time - object.start_time).max(1) as f64
            * spans;
        if traversal < (repeat - 1) as f64 || traversal >= repeat as f64 {
            0.0
        } else if traversal > repeat as f64 - fade {
            ((repeat as f64 - traversal) / fade).max(0.0)
        } else {
            1.0
        }
    };
    opacity * factor
}

pub(crate) fn break_alpha(period: &BreakPeriod, time: i64) -> f64 {
    if period.end_time - period.start_time < BREAK_MIN_DURATION_MS
        || !(period.start_time..=period.end_time).contains(&time)
    {
        return 0.0;
    }
    if time < period.start_time + BREAK_FADE_DURATION_MS {
        return (time - period.start_time) as f64 / BREAK_FADE_DURATION_MS as f64;
    }
    if time > period.end_time - BREAK_FADE_DURATION_MS {
        return (period.end_time - time) as f64 / BREAK_FADE_DURATION_MS as f64;
    }
    1.0
}

pub(crate) fn break_remaining_ratio(period: &BreakPeriod, time: i64) -> f64 {
    let duration = period.end_time - BREAK_FADE_DURATION_MS - period.start_time;
    if duration <= 0 {
        return 0.0;
    }
    ((period.end_time - BREAK_FADE_DURATION_MS - time) as f64 / duration as f64).clamp(0.0, 1.0)
}
