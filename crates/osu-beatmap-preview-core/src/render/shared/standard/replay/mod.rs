//! 回放帧消费不可变结果；玩法层和最终背景上的 HUD 查询同一时刻。
mod blend;
mod feedback;
mod flashlight;
pub mod font;
mod frame;
mod hud;
mod layout;
mod overlay;
mod slider;
use crate::gameplay::timeline::GameplayTimeline;
use crate::model::BreakPeriod;
use crate::render::canvas::Img;
use crate::render::cpu::modes::standard::{autoplay, context::*};
use crate::replay::preparation::standard::PreparedStandard;
#[cfg(test)]
use frame::head_alpha;
pub(crate) use frame::ReplayDrawTarget;
pub use frame::ReplayRenderPlan;
pub(crate) use slider::data as slider_data;
use std::sync::Arc;

pub(crate) fn replay_layout(format: crate::render::geometry::OutputFormat) -> FrameLayout {
    layout::build(format)
}

/// 每次回放导出只构建一份，worker 共享光标精灵和原版字体。
pub struct ReplayVisuals {
    pub cursor: autoplay::AutoplayCursor,
    pub font: Option<font::ArgonFont>,
    /// 判定条颜色轴只与 OD 和当前格式 SCALE 有关，准备阶段生成一次。
    hit_error_axis: Arc<Img>,
    health: hud::HealthAnimation,
    feedback: feedback::FeedbackVisuals,
    flashlight: Option<flashlight::ReplayFlashlight>,
}
impl ReplayVisuals {
    pub fn new(
        context: &RenderContext,
        gameplay: &GameplayTimeline,
        font: Option<font::ArgonFont>,
        breaks: &[BreakPeriod],
        od: f64,
    ) -> Self {
        let cursor =
            autoplay::AutoplayCursor::from_replay(context, &gameplay.input, gameplay.failure_time);
        let mods = crate::replay::mods::decode(&gameplay.document).expect("已校验的回放 Mod");
        let flashlight = mods
            .settings
            .flashlight
            .then(|| flashlight::ReplayFlashlight::new(gameplay, &cursor, &mods, breaks));
        let scale = context.frame_layout.output_scale();
        Self {
            flashlight,
            feedback: feedback::FeedbackVisuals::new(context, font.as_ref()),
            hit_error_axis: hud::build_hit_error_axis(scale, od),
            health: hud::HealthAnimation::new(gameplay),
            cursor,
            font,
        }
    }
}

/// 最终背景合成之后调用，确保 Argon 的加法文字和粒子不会被 src-over 再次暗化。
pub fn draw_overlay(
    image: &mut Img,
    context: &RenderContext,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    visuals: &ReplayVisuals,
    time: i64,
) {
    draw_overlay_to(image, context, prepared, gameplay, visuals, time);
}

/// 原生场景共享 CPU 回放 HUD 与 Argon 反馈参数，不上传整帧位图。
pub fn draw_scene_overlay(
    scene: &mut crate::render::scene::FrameSceneBuilder,
    context: &RenderContext,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    visuals: &ReplayVisuals,
    time: i64,
) {
    draw_overlay_to(scene, context, prepared, gameplay, visuals, time);
}

fn draw_overlay_to(
    target: &mut impl overlay::OverlayCanvas,
    context: &RenderContext,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    visuals: &ReplayVisuals,
    time: i64,
) {
    feedback::draw(
        target,
        context,
        prepared,
        gameplay,
        &visuals.feedback,
        time as f64,
    );
    if let Some(flashlight) = &visuals.flashlight {
        target.flashlight(
            &flashlight.at(context, time),
            crate::render::geometry::PixelRect {
                x: 0,
                y: 0,
                width: context.frame_layout.frame_width,
                height: context.frame_layout.frame_height,
            },
        );
    }
    hud::draw(
        target,
        context,
        gameplay,
        visuals,
        time as f64,
        prepared.last_end.max(gameplay.settlement_time).ceil() as i64,
        prepared.od,
    );
}

#[cfg(test)]
mod tests;
