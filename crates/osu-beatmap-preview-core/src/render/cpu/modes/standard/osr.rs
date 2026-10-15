//! 回放帧消费不可变结果；玩法层和最终背景上的 HUD 查询同一时刻。
mod blend;
mod feedback;
pub mod font;
mod hud;
mod layout;
mod overlay;
use super::{context::*, driver, frame, spinner};
use crate::gameplay::timeline::GameplayTimeline;
use crate::model::BreakPeriod;
use crate::preparation::standard::PreparedStandard;
use crate::render::canvas::Img;

pub(crate) fn replay_layout(format: crate::render::geometry::OutputFormat) -> FrameLayout {
    layout::build(format)
}

/// 每次回放导出只构建一份，worker 共享光标精灵和原版字体。
pub struct ReplayVisuals {
    pub cursor: super::autoplay::AutoplayCursor,
    pub font: Option<font::ArgonFont>,
}
impl ReplayVisuals {
    pub fn new(
        context: &RenderContext,
        gameplay: &GameplayTimeline,
        font: Option<font::ArgonFont>,
    ) -> Self {
        Self {
            cursor: super::autoplay::AutoplayCursor::from_replay(
                context,
                &gameplay.input,
                gameplay.failure_time,
            ),
            font,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    context: &RenderContext,
    cache: &mut RenderCache,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    time: i64,
    breaks: &[BreakPeriod],
    visible: &[usize],
    background: Option<&Img>,
) -> Img {
    cache.prepare_for_context(context);
    let mut image = driver::background_frame(context, background);
    frame::draw_follow_points(&mut image, context, cache, time);
    let failed = gameplay.failure_time.is_some_and(|end| time as f64 >= end);
    for &id in visible {
        let object = &context.hit_objects[id];
        let state = &gameplay.objects[id];
        if failed {
            if object.hit_type & 8 != 0 {
                frame::draw_spinner(&mut image, context, cache, object, time);
            } else if object.hit_type & 2 != 0 {
                frame::draw_slider(&mut image, context, cache, id, time);
            } else {
                frame::draw_hit_circle(&mut image, context, cache, id, time);
            }
        } else if object.hit_type & 8 != 0 {
            let (angle, total) = state.rotation_at(time as f64);
            let (_, previous) = state.rotation_at(time as f64 - 100.0);
            spinner::draw_cpu_state(
                &mut image,
                context,
                object,
                time,
                Some((
                    angle.to_radians() as f64,
                    total as f64 / 360.0,
                    (total - previous) as f64 / 360.0 * 600.0,
                )),
            );
        } else {
            let alpha = head_alpha(
                context,
                object.start_time,
                state.head,
                time,
                gameplay.policy.fade_hit_circle_early,
                prepared.od,
            );
            if object.hit_type & 2 != 0 {
                frame::draw_slider_state(
                    &mut image,
                    context,
                    cache,
                    id,
                    time,
                    Some(alpha),
                    state.tracking_at(time as f64),
                );
            } else {
                let position = prepared.objects[id].position;
                frame::draw_circle_piece(
                    &mut image,
                    context,
                    cache,
                    to_frame_point(
                        position[0] as f64,
                        position[1] as f64,
                        &context.frame_layout,
                    ),
                    context.combo_info[id].color,
                    alpha,
                    &context.combo_info[id].number.to_string(),
                );
            }
        }
    }
    for &id in visible {
        if context.hit_objects[id].hit_type & 8 == 0
            && (failed || gameplay.objects[id].head.is_none_or(|h| h.0 > time as f64))
        {
            frame::draw_approach_circle(
                &mut image,
                context,
                cache,
                id,
                context.combo_info[id].color,
                time,
            );
        }
    }
    if let Some(period) = frame::current_break_period(breaks, time) {
        frame::draw_break_overlay(&mut image, period, time, context);
    }
    image
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
    feedback::draw(
        image,
        context,
        prepared,
        gameplay,
        visuals.font.as_ref(),
        time as f64,
    );
    hud::draw(
        image,
        context,
        gameplay,
        visuals,
        time as f64,
        prepared.last_end.max(gameplay.settlement_time).ceil() as i64,
    );
}

/// 未判定时维持物件，实际命中后才消退；Miss 的掉落和文字由反馈层绘制。
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
        None => super::alpha::normal_object_alpha(start, time.max(start), time, &context.settings),
    };
    if fade_early && judgement.is_none_or(|(hit, result)| hit > time as f64 || !result.is_hit()) {
        // 官方 CL 从 100 窗结束开始淡出，在 50 窗结束时完全透明；命中后的动画独立。
        let windows = crate::calculate::standard::circle::windows(od);
        alpha.min(
            ((start as f64 + windows[2] - time as f64) / (windows[2] - windows[1])).clamp(0.0, 1.0),
        )
    } else {
        alpha
    }
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
    feedback::draw(
        scene,
        context,
        prepared,
        gameplay,
        visuals.font.as_ref(),
        time as f64,
    );
    hud::draw(
        scene,
        context,
        gameplay,
        visuals,
        time as f64,
        prepared.last_end.max(gameplay.settlement_time).ceil() as i64,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// 只在手工验收时导出原始像素；生产构建没有截图或样本入口。
    #[test]
    #[ignore = "需要 OSR_SAMPLE_DIR 和 OSR_SCREENSHOT_DIR 指定手工验收路径"]
    fn official_sample_renders_identically_after_seeking() {
        let root = std::path::PathBuf::from(std::env::var("OSR_SAMPLE_DIR").unwrap());
        let output = std::path::PathBuf::from(std::env::var("OSR_SCREENSHOT_DIR").unwrap());
        let beatmap =
            crate::parse_beatmap_bytes(&std::fs::read(root.join("4903207.osu")).unwrap()).unwrap();
        let replay = Arc::new(
            crate::replay::parse_osr(
                &std::fs::read(root.join("xuan_yuan-osu-6086051920.osr")).unwrap(),
            )
            .unwrap(),
        );
        let prepared = crate::preparation::standard::prepare(&beatmap).unwrap();
        let gameplay = crate::gameplay::engine::compile_standard(&prepared, replay).unwrap();
        let context = build_replay_context(
            &beatmap,
            &prepared,
            crate::processing::timeline::TimeAxis::new(prepared.objects[0].source.start_time),
            crate::render::geometry::OutputFormat::Mp4,
        );
        let times = [31300, 31000, 31300];
        let visuals = ReplayVisuals::new(&context, &gameplay, None);
        let indexes = build_visible_indexes_by_snapshot(
            &context.hit_objects,
            &times,
            context.settings.preempt_ms,
        );
        let mut cache = RenderCache::default();
        let mut first = None;
        for (index, time) in times.into_iter().enumerate() {
            let mut image = render_frame(
                &context,
                &mut cache,
                &prepared,
                &gameplay,
                time,
                &beatmap.break_periods,
                &indexes[index],
                None,
            );
            draw_overlay(&mut image, &context, &prepared, &gameplay, &visuals, time);
            if index == 0 {
                first = Some(image.data.clone());
            }
            if index == 2 {
                assert_eq!(first.as_ref().unwrap(), &image.data);
                std::fs::create_dir_all(&output).unwrap();
                std::fs::write(output.join("osr-preview.rgba"), &image.data).unwrap();
                std::fs::write(
                    output.join("osr-preview.json"),
                    format!(
                        "{{\"width\":{},\"height\":{},\"time_ms\":{time}}}",
                        image.w, image.h
                    ),
                )
                .unwrap();
            }
        }
    }
}
