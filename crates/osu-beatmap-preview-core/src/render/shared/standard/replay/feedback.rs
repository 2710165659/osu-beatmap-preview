//! 对照 lazer ArgonJudgementPiece 的文字、颜色、环数与动画曲线。
//! 粒子随机量由事件 ID 固定，seek、并行导出和 FPS 改变不会重新抽样。
use super::overlay::OverlayCanvas;
use super::*;
use crate::gameplay::events::{EventKind, HitResult};
use crate::render::shared::replay::feedback::{self, FeedbackVisuals};

/// 官方 DrawableOsuJudgement 使用物件中心和 CS 缩放，主滑条判定固定在尾端。
fn judgement_position(
    object: &crate::replay::preparation::standard::PreparedObject,
    sample_time: f64,
    nested: bool,
) -> [f32; 2] {
    if object.source.hit_type & 8 != 0 {
        [256.0, 192.0]
    } else if nested || sample_time <= object.source.start_time as f64 {
        object.position_at(sample_time)
    } else {
        object.position_at(object.end)
    }
}

pub(super) fn draw(
    image: &mut impl OverlayCanvas,
    context: &RenderContext,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    visuals: &FeedbackVisuals,
    time: f64,
) {
    // 判定属于 playfield 坐标系，尺寸随当前格式的布局倍率变化。
    let scale = context.frame_layout.scale * f64::from(prepared.radius) / 64.0;
    for event in gameplay.events_between(time - 1000.0, time) {
        if event.kind != EventKind::Judgement || event.object_id >= prepared.objects.len() {
            continue;
        }
        let age = time - event.time;
        // Argon 的滑条嵌套漏判是直径 12 的实心圆，不能套用主物件的 MISS 文字。
        if matches!(
            event.result,
            HitResult::LargeTickMiss | HitResult::SmallTickMiss | HitResult::IgnoreMiss
        ) {
            let object = &prepared.objects[event.object_id];
            // CL 滑条头没有 node_id，但同样使用漏点圆；转盘奖励漏判不生成视觉反馈。
            if object.path.is_some()
                && (event.node_id.is_some() || event.result == HitResult::LargeTickMiss)
                && age < 600.0
            {
                let position = judgement_position(object, event.sample_time, true);
                let (x, y) = to_frame_point(
                    f64::from(position[0]),
                    f64::from(position[1]),
                    &context.frame_layout,
                );
                let expansion = 1.0 + 0.4 * (1.0 - (age / 150.0).clamp(0.0, 1.0)).powi(2);
                image.additive_transformed(
                    &visuals.tick_misses[usize::from(event.result != HitResult::IgnoreMiss)],
                    [x, y],
                    expansion,
                    0.0,
                    1.0 - age / 600.0,
                );
            }
            continue;
        }
        let object = &prepared.objects[event.object_id];
        let position = judgement_position(object, event.sample_time, event.node_id.is_some());
        let (x, y) = to_frame_point(
            position[0] as f64,
            position[1] as f64,
            &context.frame_layout,
        );
        feedback::draw_judgement(image, visuals, event.result, event.id, [x, y], scale, age);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processing::timeline::TimeAxis;
    use crate::render::{
        geometry::OutputFormat,
        scene::{DrawCommand, FrameSceneBuilder},
    };
    use std::sync::Arc;

    #[test]
    fn slider_head_nodes_and_parent_use_their_object_positions() {
        let (_, prepared) = fixture(5);
        let object = &prepared.objects[0];
        let start = object.source.start_time as f64;
        assert_eq!(judgement_position(object, start, false), object.position);
        assert_eq!(
            judgement_position(object, start + object.span / 2.0, true),
            object.position_at(start + object.span / 2.0)
        );
        assert_eq!(
            judgement_position(object, object.end, false),
            object.position_at(object.end)
        );
        assert_ne!(object.position, object.position_at(object.end));
    }

    #[test]
    fn gpu_feedback_reuses_textures_for_scale_rotation_fade_and_seek() {
        let (context, _) = fixture(5);
        let visuals = FeedbackVisuals::new(
            context.frame_layout.scale * context.settings.object_scale,
            None,
        );
        let sprite = &visuals.labels[3];
        for (scale, rotation, alpha) in [(1.6, 0.0, 1.0), (1.0, 20.0, 0.5), (1.6, 0.0, 1.0)] {
            let mut scene = FrameSceneBuilder::new(100, 100, 0);
            scene.additive_transformed(sprite, [40.0, 50.0], scale, rotation, alpha);
            let scene = scene.finish();
            assert_eq!(scene.resources.len(), 1);
            assert!(Arc::ptr_eq(
                scene.resources.values().next().unwrap(),
                sprite
            ));
            let DrawCommand::TransformedSprite {
                position,
                origin,
                size,
                rotation: actual,
                additive,
                ..
            } = scene.commands[0]
            else {
                panic!("判定动画必须使用 GPU 变换");
            };
            assert_eq!(position, [40.0, 50.0]);
            assert_eq!(origin, [0.5; 2]);
            assert_eq!(
                size,
                [
                    (sprite.w as f64 * scale) as f32,
                    (sprite.h as f64 * scale) as f32
                ]
            );
            assert_eq!(actual, rotation.to_radians() as f32);
            assert!(additive);
        }
    }

    #[test]
    fn feedback_size_tracks_cs_and_current_output_scale() {
        let (context, _) = fixture(5);
        let normal = FeedbackVisuals::new(
            context.frame_layout.scale * context.settings.object_scale,
            None,
        );
        let (small_context, _) = fixture(8);
        let small = FeedbackVisuals::new(
            small_context.frame_layout.scale * small_context.settings.object_scale,
            None,
        );
        assert!(small.labels[0].h < normal.labels[0].h);
        let mut context = context;
        context.frame_layout.scale *= 2.0;
        let double = FeedbackVisuals::new(
            context.frame_layout.scale * context.settings.object_scale,
            None,
        );
        assert!(double.rings[0][0].w > normal.rings[0][0].w);
    }

    fn fixture(cs: u8) -> (RenderContext, PreparedStandard) {
        let raw = format!("osu file format v14\n[General]\nMode:0\n[Difficulty]\nCircleSize:{cs}\nSliderMultiplier:1.4\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n100,100,1000,2,0,L|300:100,1,200\n");
        let beatmap = crate::parse_beatmap_bytes(raw.as_bytes()).unwrap();
        let prepared = crate::replay::preparation::standard::prepare(&beatmap).unwrap();
        let context =
            build_replay_context(&beatmap, &prepared, TimeAxis::new(1000), OutputFormat::Mp4);
        (context, prepared)
    }
}
