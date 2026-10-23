//! Taiko 判定反馈遵循游戏内 Argon PRO 2022 的可见性与动画节奏。
use crate::gameplay::events::HitResult;
use crate::render::shared::replay::{feedback::FeedbackVisuals, overlay::OverlayCanvas};

pub(super) fn draw_judgement(
    target: &mut impl OverlayCanvas,
    visuals: &FeedbackVisuals,
    result: HitResult,
    event_id: usize,
    position: [f64; 2],
    scale: f64,
    lane_height: i64,
    age: f64,
) -> bool {
    // TaikoArgonSkinTransformer 对 PRO 的 Great 返回空组件，文字和环形粒子一并隐藏。
    // 命中物件的飞行动画由独立绘制流程处理，不受判定组件可见性影响。
    let (label, small, large, travel) = match result {
        HitResult::Ok => (1, 4, 0, 34.8),
        HitResult::Meh => (2, 3, 0, 17.4),
        HitResult::Miss => (3, 0, 0, 0.0),
        _ => return false,
    };
    let duration = 800.0;
    if !(0.0..duration).contains(&age) {
        return false;
    }

    let parent_alpha = 1.0 - out_quint(age / duration);
    if small + large > 0 {
        let alpha = parent_alpha * (1.0 - out_quint(age / 1000.0));
        let mut seed = event_id as u64;
        for index in 0..small + large {
            let angle = random(&mut seed) * std::f64::consts::TAU;
            let distance = travel * (0.5 + random(&mut seed) * 0.5) * scale;
            let offset = distance * (0.6 + 0.4 * out_quint(age / 600.0));
            target.additive_transformed(
                &visuals.rings[label][usize::from(index >= small)],
                [
                    position[0] + angle.cos() * offset,
                    position[1] + angle.sin() * offset,
                ],
                1.0,
                0.0,
                alpha,
            );
        }
    }

    let (text_position, expansion, rotation) = if result == HitResult::Miss {
        let progress = (age / 100.0).clamp(0.0, 1.0);
        (
            [
                position[0],
                position[1] + 100.0 * scale * (age / duration).powi(5),
            ],
            1.6 - 0.6 * progress.powi(2),
            40.0 * (age / duration).powi(5),
        )
    } else {
        let rise = 0.6 + 0.4 * out_quint(age / duration);
        (
            [position[0], position[1] - rise * lane_height as f64],
            1.0 + 0.4 * out_quint(age / duration),
            0.0,
        )
    };
    target.additive_transformed(
        &visuals.labels[label],
        text_position,
        expansion,
        rotation,
        parent_alpha,
    );
    true
}

fn out_quint(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(5)
}

fn random(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_add(0x9e3779b97f4a7c15);
    let mut value = *seed;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((value ^ (value >> 31)) >> 11) as f64 / (1_u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{
        scene::{DrawCommand, FrameSceneBuilder},
        shared::replay::feedback::FeedbackVisuals,
    };
    use std::sync::Arc;

    #[test]
    fn taiko_pro_results_use_game_particle_counts_and_text_motion() {
        let visuals = FeedbackVisuals::new_taiko(1.0, None);
        let position = [120.0, 200.0];
        let lane_height = 100;
        for (result, expected_count) in [
            (HitResult::Ok, 5),
            (HitResult::Meh, 4),
            (HitResult::Miss, 1),
        ] {
            let mut scene = FrameSceneBuilder::new(300, 300, 0);
            assert!(draw_judgement(
                &mut scene,
                &visuals,
                result,
                10,
                position,
                1.0,
                lane_height,
                0.0,
            ));
            let scene = scene.finish();
            assert_eq!(scene.commands.len(), expected_count);
            let label = &visuals.labels[match result {
                HitResult::Ok => 1,
                HitResult::Meh => 2,
                HitResult::Miss => 3,
                _ => unreachable!(),
            }];
            let command = scene
                .commands
                .iter()
                .find(|command| matches!(command, DrawCommand::TransformedSprite { resource, .. } if Arc::ptr_eq(&scene.resources[resource], label)))
                .unwrap();
            let DrawCommand::TransformedSprite {
                position: actual, ..
            } = command
            else {
                unreachable!();
            };
            let expected_y = if result != HitResult::Miss {
                position[1] - lane_height as f64 * 0.6
            } else {
                position[1]
            };
            assert_eq!(*actual, [position[0] as f32, expected_y as f32]);
        }

        let mut scene = FrameSceneBuilder::new(300, 300, 50);
        draw_judgement(
            &mut scene,
            &visuals,
            HitResult::Miss,
            11,
            position,
            1.0,
            lane_height,
            50.0,
        );
        let scene = scene.finish();
        assert!(scene.commands.iter().any(|command| matches!(
            command,
            DrawCommand::TransformedSprite { resource, position: actual, rotation, .. }
                if Arc::ptr_eq(&scene.resources[resource], &visuals.labels[3])
                    && actual[1] > position[1] as f32
                    && *rotation > 0.0
        )));
    }

    #[test]
    fn taiko_pro_great_has_no_text_or_judgement_particles_on_either_target() {
        use crate::render::canvas::Img;

        for scale in [0.75, 1.0, 1.5] {
            let visuals = FeedbackVisuals::new_taiko(scale, None);
            for age in [0.0, 100.0, 400.0, 799.0] {
                let mut scene = FrameSceneBuilder::new(300, 300, age as i64);
                let mut image = Img::new(300, 300, [0; 4]);
                assert!(!draw_judgement(
                    &mut scene,
                    &visuals,
                    HitResult::Great,
                    10,
                    [120.0, 200.0],
                    scale,
                    (100.0 * scale) as i64,
                    age,
                ));
                assert!(!draw_judgement(
                    &mut image,
                    &visuals,
                    HitResult::Great,
                    10,
                    [120.0, 200.0],
                    scale,
                    (100.0 * scale) as i64,
                    age,
                ));
                assert!(scene.finish().commands.is_empty());
                assert!(image.data.iter().all(|byte| *byte == 0));
            }
        }
    }
}
