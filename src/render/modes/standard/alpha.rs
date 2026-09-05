//! osu!standard 渲染器的 alpha 与时间辅助函数。

use crate::domain::models::StandardHitObject;

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
    (1.0 - (snapshot_time - end_time) as f64
        / crate::render::modes::standard::constants::SLIDER_FADE_OUT_MS as f64)
        .max(0.0)
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
    (1.0 - (snapshot_time - end_time) as f64
        / crate::render::modes::standard::constants::SLIDER_FADE_OUT_MS as f64)
        .max(0.0)
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
    (1.0 - (snapshot_time - hit_object.end_time) as f64
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
    if snapshot_time
        <= hit_object.start_time + crate::render::modes::standard::constants::POST_HIT_FADE_MS
    {
        return 1.0
            - (snapshot_time - hit_object.start_time) as f64
                / crate::render::modes::standard::constants::POST_HIT_FADE_MS as f64;
    }
    0.0
}

/// 计算单个 SliderTick 的生命周期透明度。
///
/// 普通模式沿用 DrawableSliderTick 的 150ms 淡入和判定后的 150ms 淡出；
/// Hidden 模式则使用 osu! 对 tick 的短窗口淡出规则。
pub(crate) fn slider_tick_alpha(
    tick_time: f64,
    time_preempt: f64,
    snapshot_time: i64,
    settings: &RenderSettings,
) -> f64 {
    let t = snapshot_time as f64;
    if settings.hidden && !settings.traceable {
        let spawn_time = tick_time - time_preempt;
        if t < spawn_time {
            return 0.0;
        }

        // 初始变换仍会先执行 150ms 淡入，Hidden 的特殊淡出紧接其后开始。
        if t < spawn_time + 150.0 {
            return ((t - spawn_time) / 150.0).clamp(0.0, 1.0);
        }

        let duration = (time_preempt - 150.0).clamp(0.0, 1000.0);
        if duration <= 0.0 {
            return 0.0;
        }
        return ((tick_time - t) / duration).clamp(0.0, 1.0);
    }

    let spawn_time = tick_time - time_preempt;
    if t < spawn_time {
        return 0.0;
    }
    if t < spawn_time + 150.0 {
        return ((t - spawn_time) / 150.0).clamp(0.0, 1.0);
    }
    if t <= tick_time {
        return 1.0;
    }
    // DrawableSliderTick 命中后使用 Easing.OutQuint 淡出：动画开始阶段
    // 快速降低透明度，尾部逐渐收敛到零。预览没有判定状态，因此始终走此成功路径。
    let progress = ((t - tick_time) / 150.0).clamp(0.0, 1.0);
    (1.0 - progress).powi(5)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(hidden: bool, traceable: bool) -> RenderSettings {
        RenderSettings {
            circle_diameter: 128,
            object_scale: 1.0,
            preempt_ms: 800,
            fade_in_ms: 400.0,
            hidden,
            traceable,
        }
    }

    #[test]
    fn slider_tick_alpha_fades_in_and_fades_after_tick() {
        let s = settings(false, false);
        assert_eq!(slider_tick_alpha(1000.0, 400.0, 599, &s), 0.0);
        assert!((slider_tick_alpha(1000.0, 400.0, 700, &s) - (100.0 / 150.0)).abs() < 1e-9);
        assert_eq!(slider_tick_alpha(1000.0, 400.0, 1000, &s), 1.0);
        assert!((slider_tick_alpha(1000.0, 400.0, 1075, &s) - 0.03125).abs() < 1e-9);
        assert_eq!(slider_tick_alpha(1000.0, 400.0, 1150, &s), 0.0);
    }

    #[test]
    fn hidden_slider_tick_fades_to_zero_at_tick_time() {
        let s = settings(true, false);
        assert_eq!(slider_tick_alpha(1000.0, 500.0, 499, &s), 0.0);
        assert_eq!(slider_tick_alpha(1000.0, 500.0, 575, &s), 0.5);
        assert_eq!(slider_tick_alpha(1000.0, 500.0, 650, &s), 1.0);
        assert!((slider_tick_alpha(1000.0, 500.0, 825, &s) - 0.5).abs() < 1e-9);
        assert_eq!(slider_tick_alpha(1000.0, 500.0, 1000, &s), 0.0);
    }

    #[test]
    fn traceable_uses_normal_slider_tick_lifecycle() {
        let s = settings(true, true);
        assert!(slider_tick_alpha(1000.0, 400.0, 700, &s) > 0.0);
    }
}
