//! 对照 lazer ArgonJudgementPiece 的文字、颜色、环数与动画曲线。
//! 粒子随机量由事件 ID 固定，seek、并行导出和 FPS 改变不会重新抽样。
use super::overlay::OverlayCanvas;
use super::*;
use crate::gameplay::events::{EventKind, HitResult};
use crate::render::text::{render_text_sprite, scaled_bitmap_font_height};

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

pub(super) fn draw(
    image: &mut impl OverlayCanvas,
    context: &RenderContext,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    font: Option<&font::ArgonFont>,
    time: f64,
) {
    // 判定属于 playfield 坐标系，尺寸随当前格式的布局倍率变化。
    let scale = context.frame_layout.scale;
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
            if event.node_id.is_some() && age < 600.0 {
                let object = &prepared.objects[event.object_id];
                let position = object.position_at(event.sample_time);
                let (x, y) = to_frame_point(
                    f64::from(position[0]),
                    f64::from(position[1]),
                    &context.frame_layout,
                );
                let expansion = 1.0 + 0.4 * (1.0 - (age / 150.0).clamp(0.0, 1.0)).powi(2);
                let radius = 6.0 * expansion * scale;
                let size = (radius * 2.0).ceil() as u32 + 2;
                let centre = f64::from(size) / 2.0;
                let mut piece = Img::new(size, size, [0; 4]);
                let color = if event.result == HitResult::IgnoreMiss {
                    [128; 3]
                } else {
                    [237, 17, 33]
                };
                super::super::slider::fill_circle_gradient_aa(
                    &mut piece, centre, centre, radius, color, color,
                );
                image.additive(
                    &piece,
                    (x - centre).round() as i64,
                    (y - centre).round() as i64,
                    1.0 - age / 600.0,
                );
            }
            continue;
        }
        let (text, color, small, large, travel) = match event.result {
            HitResult::Great => ("GREAT", [102, 204, 255], 4, 4, 52.0),
            HitResult::Ok => ("OK", [136, 179, 0], 4, 0, 31.2),
            HitResult::Meh => ("MEH", [255, 204, 34], 3, 0, 15.6),
            HitResult::Miss => ("MISS", [237, 17, 33], 0, 0, 0.0),
            _ => continue,
        };
        let object = &prepared.objects[event.object_id];
        let position = event.position.unwrap_or_else(|| {
            if object.source.hit_type & 8 != 0 {
                [256.0, 192.0]
            } else {
                object.position_at(event.time)
            }
        });
        let (x, mut y) = to_frame_point(
            position[0] as f64,
            position[1] as f64,
            &context.frame_layout,
        );
        let missed = !event.result.is_hit();
        let duration = 800.0;
        if age >= duration {
            continue;
        }
        let parent_alpha = 1.0 - age / duration;
        let expansion = if missed {
            1.6 - 0.6 * (age / 100.0).clamp(0.0, 1.0).powi(2)
        } else {
            1.0 + 0.2 * out_quint(age / 1800.0)
        };
        if small + large > 0 {
            let alpha = (255.0 * parent_alpha * (1.0 - out_quint(age / 1000.0))).round() as u8;
            let mut seed = event.id as u64;
            for index in 0..small + large {
                let angle = random(&mut seed) * std::f64::consts::TAU;
                let distance = travel
                    * (0.5 + random(&mut seed) * 0.5)
                    * (0.3 + 0.7 * out_quint(age / 600.0))
                    * scale;
                // BorderThickness 为 4；环的直径分别是 9 与 14 个逻辑像素。
                let radius = if index < small {
                    4.5 * scale
                } else {
                    7.0 * scale
                };
                let size = (radius * 2.0).ceil().max(1.0) as u32 + 2;
                let mut ring = Img::new(size, size, [0; 4]);
                let centre = f64::from(size) / 2.0;
                super::super::slider::draw_ring_aa(
                    &mut ring,
                    centre,
                    centre,
                    radius,
                    4.0 * scale,
                    [color[0], color[1], color[2], alpha],
                );
                image.additive(
                    &ring,
                    (x + angle.cos() * distance - centre).round() as i64,
                    (y + angle.sin() * distance - centre).round() as i64,
                    1.0,
                );
            }
        }
        let mut sprite = font.map_or_else(
            || {
                render_text_sprite(
                    text,
                    scaled_bitmap_font_height(20, scale),
                    [color[0], color[1], color[2], 255],
                )
            },
            |font| font.sprite(text, 20.0 * scale, 5.0 * scale, color),
        );
        sprite = sprite.resize_bilinear(
            (sprite.w as f64 * expansion).round().max(1.0) as u32,
            (sprite.h as f64 * expansion).round().max(1.0) as u32,
        );
        let alpha = if missed {
            parent_alpha
        } else {
            parent_alpha * out_quint(age / 300.0)
        };
        if missed {
            let rotation = 40.0 * (age / 800.0).powi(5);
            y += 100.0 * scale * (age / 800.0).powi(5);
            // Img 的正旋转与游戏顺时针坐标方向相反。
            sprite = sprite.rotate_expand(-rotation);
        }
        image.additive(
            &sprite,
            (x - sprite.w as f64 / 2.0).round() as i64,
            (y - sprite.h as f64 / 2.0).round() as i64,
            alpha,
        );
    }
}
