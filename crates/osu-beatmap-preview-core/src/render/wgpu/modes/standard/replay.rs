//! 原生 Standard 回放场景；物件生命周期取判定时间，几何仍走共享 WebGPU 绘制器。
use super::*;
use crate::render::cpu::modes::standard::{
    context::build_replay_context,
    osr::{self, font::ArgonFont, ReplayVisuals},
};
use crate::{gameplay::timeline::GameplayTimeline, preparation::standard::PreparedStandard};

pub(super) struct ReplayScene {
    prepared: Arc<PreparedStandard>,
    pub(super) gameplay: Arc<GameplayTimeline>,
    visuals: ReplayVisuals,
}
pub fn prepare(
    beatmap: &Beatmap,
    prepared: Arc<PreparedStandard>,
    gameplay: Arc<GameplayTimeline>,
    font: Option<ArgonFont>,
    dimensions: (u32, u32),
) -> Result<RealtimeFrameSource> {
    let mut context = build_replay_context(
        beatmap,
        &prepared,
        TimeAxis::new(prepared.objects[0].source.start_time),
        OutputFormat::Mp4,
    );
    let dx = (i64::from(dimensions.0) - context.frame_layout.frame_width) as f64 / 2.0;
    let dy = (i64::from(dimensions.1) - context.frame_layout.frame_height) as f64 / 2.0;
    context.frame_layout.playfield_left += dx;
    context.frame_layout.playfield_top += dy;
    context.frame_layout.frame_width = i64::from(dimensions.0);
    context.frame_layout.frame_height = i64::from(dimensions.1);
    let visuals = ReplayVisuals::new(&context, &gameplay, font, &beatmap.break_periods);
    source_from_context(
        beatmap,
        context,
        Some(ReplayScene {
            prepared,
            gameplay,
            visuals,
        }),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    context: &RenderContext,
    sliders: &[Option<PreparedSlider>],
    digits: &[Arc<Img>],
    breaks: &[PreparedBreak],
    follow: &PreparedFollowPoints,
    replay: &ReplayScene,
    time: i64,
    visible: &[usize],
) -> FrameScene {
    let gameplay = &replay.gameplay;
    let failed = gameplay.failure_time.is_some_and(|end| time as f64 >= end);
    let mut scene = FrameSceneBuilder::new(
        context.frame_layout.frame_width as u32,
        context.frame_layout.frame_height as u32,
        time,
    );
    draw_follow_points(&mut scene, context, follow, time);
    for &id in visible {
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
            crate::render::cpu::modes::standard::spinner::draw_gpu_state(
                &mut scene, context, object, time, rotation,
            );
        } else if failed {
            if object.hit_type & 2 != 0 {
                draw_slider(
                    &mut scene,
                    context,
                    sliders[id].as_ref().unwrap(),
                    digits,
                    id,
                    time,
                );
            } else {
                draw_hit_circle(&mut scene, context, digits, id, time);
            }
        } else {
            let alpha = osr::head_alpha(
                context,
                object.start_time,
                state.head,
                time,
                gameplay.policy.fade_hit_circle_early,
                replay.prepared.od,
            );
            if object.hit_type & 2 != 0 {
                draw_slider_state(
                    &mut scene,
                    context,
                    sliders[id].as_ref().unwrap(),
                    digits,
                    id,
                    time,
                    Some(alpha),
                    state.tracking_at(time as f64),
                );
            } else if !context.settings.traceable {
                let p = replay.prepared.objects[id].position;
                draw_circle_piece(
                    &mut scene,
                    context,
                    digits,
                    to_frame_point(f64::from(p[0]), f64::from(p[1]), &context.frame_layout),
                    context.combo_info[id].color,
                    alpha,
                    context.combo_info[id].number,
                );
            }
        }
    }
    for &id in visible {
        if context.hit_objects[id].hit_type & 8 == 0
            && (failed || gameplay.objects[id].head.is_none_or(|h| h.0 > time as f64))
        {
            draw_approach_circle(&mut scene, context, id, time);
        }
    }
    if let Some(current) = breaks.iter().find(|b| break_alpha(&b.period, time) > 0.0) {
        draw_break(&mut scene, context, current, time);
    }
    osr::draw_scene_overlay(
        &mut scene,
        context,
        &replay.prepared,
        gameplay,
        &replay.visuals,
        time,
    );
    scene.finish()
}
