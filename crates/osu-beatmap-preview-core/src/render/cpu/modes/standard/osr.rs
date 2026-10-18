//! 回放 CPU 绘制适配；消费共享计划，保留原有滑条和精灵缓存。

use super::{context::*, driver, frame, spinner};
use crate::gameplay::timeline::GameplayTimeline;
use crate::model::BreakPeriod;
use crate::render::canvas::Img;
use crate::render::shared::standard::replay::ReplayDrawTarget;
use crate::replay::preparation::standard::PreparedStandard;

pub(crate) use crate::render::shared::standard::replay::replay_layout;
pub use crate::render::shared::standard::replay::{
    draw_overlay, draw_scene_overlay, font, ReplayRenderPlan, ReplayVisuals,
};

#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    context: &RenderContext,
    cache: &mut RenderCache,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    time: i64,
    breaks: &[BreakPeriod],
    plan: &ReplayRenderPlan,
    background: Option<&Img>,
) -> Img {
    cache.prepare_for_context(context);
    let mut image = driver::background_frame(context, background);
    plan.frame(context, prepared, gameplay, time, breaks)
        .draw(&mut CpuReplayTarget {
            image: &mut image,
            context,
            cache,
            time,
        });
    image
}

struct CpuReplayTarget<'a> {
    image: &'a mut Img,
    context: &'a RenderContext,
    cache: &'a mut RenderCache,
    time: i64,
}

impl ReplayDrawTarget for CpuReplayTarget<'_> {
    fn follow_points(&mut self) {
        frame::draw_follow_points(self.image, self.context, self.cache, self.time);
    }

    fn circle(&mut self, id: usize, position: (f64, f64), opacity: f64) {
        let combo = self.context.combo_info[id];
        frame::draw_circle_piece(
            self.image,
            self.context,
            self.cache,
            to_frame_point(position.0, position.1, &self.context.frame_layout),
            combo.color,
            opacity,
            &combo.number.to_string(),
        );
    }

    fn slider(&mut self, id: usize, head_opacity: Option<f64>, tracking: bool) {
        frame::draw_slider_state(
            self.image,
            self.context,
            self.cache,
            id,
            self.time,
            head_opacity,
            tracking,
        );
    }

    fn spinner(&mut self, id: usize, rotation: Option<(f64, f64, f64)>) {
        spinner::draw_cpu_state(
            self.image,
            self.context,
            &self.context.hit_objects[id],
            self.time,
            rotation,
        );
    }

    fn approach_circle(&mut self, id: usize) {
        frame::draw_approach_circle(
            self.image,
            self.context,
            self.cache,
            id,
            self.context.combo_info[id].color,
            self.time,
        );
    }

    fn break_overlay(&mut self, _index: usize, period: &BreakPeriod) {
        frame::draw_break_overlay(self.image, period, self.time, self.context);
    }
}
