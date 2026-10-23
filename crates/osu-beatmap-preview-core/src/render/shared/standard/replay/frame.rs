//! 回放帧的有序绘制计划；索引预计算，逐帧借用查询，不分配物件列表。

use crate::gameplay::timeline::GameplayTimeline;
use crate::model::BreakPeriod;
use crate::render::cpu::modes::standard::{alpha, constants::SLIDER_FADE_OUT_MS, context::*};
use crate::replay::preparation::standard::PreparedStandard;

pub struct ReplayRenderPlan {
    visibility: super::super::visibility::ObjectVisibility,
}

/// 后端接收已解析的显示状态，不读取判定时间线或自行决定物件层序。
pub(crate) trait ReplayDrawTarget {
    fn follow_points(&mut self);
    fn circle(&mut self, id: usize, position: (f64, f64), opacity: f64);
    fn slider(&mut self, id: usize, head_opacity: Option<f64>, tracking: bool);
    fn spinner(&mut self, id: usize, rotation: Option<(f64, f64, f64)>);
    fn approach_circle(&mut self, id: usize);
    fn break_overlay(&mut self, index: usize, period: &BreakPeriod);
}

impl ReplayRenderPlan {
    pub fn new(
        context: &RenderContext,
        prepared: &PreparedStandard,
        gameplay: &GameplayTimeline,
    ) -> Self {
        let intervals = context.hit_objects.iter().enumerate().map(|(id, object)| {
            // 裁剪是绘制寿命的上界；真实判定可把圆圈或滑条头保留到名义结束之后。
            let nominal = (prepared.objects[id].end.ceil() as i64).max(object.start_time)
                + SLIDER_FADE_OUT_MS;
            let end = gameplay.objects[id].head.map_or(nominal, |(time, _)| {
                nominal.max(time.ceil() as i64 + SLIDER_FADE_OUT_MS)
            });
            (object.start_time - context.settings.preempt_ms, end, id)
        });
        Self {
            visibility: super::super::visibility::ObjectVisibility::new(intervals),
        }
    }

    pub(crate) fn frame<'a>(
        &'a self,
        context: &'a RenderContext,
        prepared: &'a PreparedStandard,
        gameplay: &'a GameplayTimeline,
        time: i64,
        breaks: &'a [BreakPeriod],
    ) -> ReplayFrame<'a> {
        ReplayFrame {
            plan: self,
            context,
            prepared,
            gameplay,
            time,
            breaks,
        }
    }
}

pub(crate) struct ReplayFrame<'a> {
    plan: &'a ReplayRenderPlan,
    context: &'a RenderContext,
    prepared: &'a PreparedStandard,
    gameplay: &'a GameplayTimeline,
    time: i64,
    breaks: &'a [BreakPeriod],
}

impl ReplayFrame<'_> {
    pub(crate) fn draw(&self, target: &mut impl ReplayDrawTarget) {
        let Self {
            context,
            prepared,
            gameplay,
            time,
            ..
        } = *self;
        let failed = gameplay.failure_time.is_some_and(|end| time as f64 >= end);
        target.follow_points();
        for id in self.plan.visibility.at(time) {
            let object = &context.hit_objects[id];
            let state = &gameplay.objects[id];
            if object.hit_type & 8 != 0 {
                let rotation = (!failed).then(|| {
                    let (angle, total) = state.rotation_at(time as f64);
                    let (_, previous) = state.rotation_at(time as f64 - 100.0);
                    (
                        f64::from(angle).to_radians(),
                        f64::from(total) / 360.0,
                        f64::from(total - previous) / 360.0 * 600.0,
                    )
                });
                target.spinner(id, rotation);
            } else if object.hit_type & 2 != 0 {
                super::slider::data(context, prepared, id);
                let opacity = (!failed).then(|| {
                    head_alpha(
                        context,
                        object.start_time,
                        state.head,
                        time,
                        gameplay.standard_policy().fade_hit_circle_early,
                        prepared.od,
                    )
                });
                target.slider(id, opacity, failed || state.tracking_at(time as f64));
            } else if !context.settings.traceable {
                let (position, opacity) = if failed {
                    (
                        stacked_position(object, &context.settings),
                        alpha::object_alpha(
                            object.start_time,
                            object.start_time,
                            time,
                            &context.settings,
                        ),
                    )
                } else {
                    let p = prepared.objects[id].position;
                    (
                        (f64::from(p[0]), f64::from(p[1])),
                        head_alpha(
                            context,
                            object.start_time,
                            state.head,
                            time,
                            gameplay.standard_policy().fade_hit_circle_early,
                            prepared.od,
                        ),
                    )
                };
                target.circle(id, position, opacity);
            }
        }
        for id in self.plan.visibility.at(time) {
            if context.hit_objects[id].hit_type & 8 == 0
                && (failed || gameplay.objects[id].head.is_none_or(|h| h.0 > time as f64))
            {
                target.approach_circle(id);
            }
        }
        if let Some((index, period)) = self
            .breaks
            .iter()
            .enumerate()
            .find(|(_, period)| super::super::animation::break_alpha(period, time) > 0.0)
        {
            target.break_overlay(index, period);
        }
    }
}

/// 回放判定控制消退时间，物件主体仍受 Mod 可见性约束；Miss 的掉落和文字由反馈层绘制。
pub(crate) fn head_alpha(
    context: &RenderContext,
    start: i64,
    judgement: Option<(f64, crate::gameplay::events::HitResult)>,
    time: i64,
    fade_early: bool,
    od: f64,
) -> f64 {
    let alpha = match judgement.filter(|j| j.0 <= time as f64) {
        Some((hit, result)) => (1.0
            - (time as f64 - hit) / if result.is_hit() { 120.0 } else { 100.0 })
        .clamp(0.0, 1.0),
        None => alpha::normal_object_alpha(start, time.max(start), time, &context.settings),
    };
    // 命中或漏判的消退动画不能重置 HD 透明度；圆圈与滑条头继续使用普通预览的消隐时间轴。
    // 仅隐藏接近圈的 HD 设置和 TC 不隐藏物件主体，仍保留原有判定淡出。
    let alpha = if context.settings.hidden
        && !context.settings.traceable
        && !context.settings.hidden_only_approach
    {
        alpha.min(alpha::object_alpha(start, start, time, &context.settings))
    } else {
        alpha
    };
    if fade_early && judgement.is_none_or(|(hit, result)| hit > time as f64 || !result.is_hit()) {
        // 官方 CL 从 100 窗结束开始淡出，在 50 窗结束时完全透明；命中后的动画独立。
        let windows = crate::replay::calculate::standard::circle::windows(od);
        alpha.min(
            ((start as f64 + windows[2] - time as f64) / (windows[2] - windows[1])).clamp(0.0, 1.0),
        )
    } else {
        alpha
    }
}
