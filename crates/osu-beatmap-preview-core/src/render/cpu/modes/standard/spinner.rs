//! Argon 转盘的共享几何与 Autoplay 时间轴；任意时间取帧不依赖上一帧状态。

use super::context::{to_frame_point, RenderContext};
use super::slider::draw_ring_aa;
use crate::model::StandardHitObject;
use crate::render::canvas::{Img, Rgba};
use crate::render::scene::{FrameSceneBuilder, SceneRect};
use crate::render::text::{render_text_sprite, scaled_bitmap_font_height};
use std::f64::consts::TAU;
use std::sync::Arc;

enum Piece {
    Circle(f64, Rgba),
    Ring(f64, f64, Rgba),
    Line((f64, f64), (f64, f64), f64, Rgba),
    Text(String, f64, u32, f64, Rgba),
}

fn out_quint(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(5)
}
fn difficulty(od: f64, low: f64, mid: f64, high: f64) -> f64 {
    if od <= 5.0 {
        low + (mid - low) * od / 5.0
    } else {
        mid + (high - mid) * (od - 5.0) / 5.0
    }
}

/// 游戏 Autoplay 每真实毫秒转动 0.05 rad；倍速只换算时间，不能重复加速旋转。
pub fn autoplay_rotation(elapsed_ms: f64, rate: f64) -> f64 {
    elapsed_ms.max(0.0) * 0.05 / rate
}

fn pieces(context: &RenderContext, object: &StandardHitObject, time: i64) -> Vec<Piece> {
    let preempt = context.settings.preempt_ms.max(1) as f64;
    let elapsed = (time - object.start_time) as f64;
    let duration = (object.end_time - object.start_time).max(1) as f64;
    let after = (elapsed - duration).max(0.0);
    let alpha = if elapsed < 0.0 {
        ((elapsed + preempt) / context.settings.fade_in_ms.max(1.0)).clamp(0.0, 1.0)
    } else {
        (1.0 - after / 240.0).clamp(0.0, 1.0)
    };
    if alpha <= 0.0 {
        return Vec::new();
    }
    let scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Standard,
        context.output_format,
    );
    let entry = out_quint((elapsed + preempt / 2.0) / (preempt / 4.0));
    let grow = out_quint(elapsed / (preempt / 2.0));
    let disc_scale = 0.2 * entry + 0.8 * grow;
    let radius =
        context.frame_layout.scale * 192.0 * disc_scale * (1.0 + 0.2 * out_quint(after / 320.0));
    if radius <= 0.0 {
        return Vec::new();
    }
    let rotation = autoplay_rotation(elapsed.clamp(0.0, duration), context.spinner_rate);
    let spins = rotation / TAU;
    let required =
        (difficulty(context.spinner_od, 90.0, 150.0, 225.0) / 60000.0 * duration + 0.0001).floor();
    let progress = if required <= 0.0 {
        1.0
    } else {
        (spins / required).clamp(0.0, 1.0)
    };
    let complete = progress >= 1.0;
    let rgba = |rgb: [u8; 3], opacity: f64| {
        [
            rgb[0],
            rgb[1],
            rgb[2],
            (255.0 * opacity * alpha).round() as u8,
        ]
    };
    let white = rgba([255; 3], 1.0);
    let pink = [252, 97, 143];
    let fill_radius = (radius - 8.0 * scale).max(0.0) * (0.1 + 0.88 * progress);
    let mut result = Vec::new();
    // 多层低透明度圆环近似 Argon 的柔和阴影，不改变后端的混合契约。
    for layer in (1..=8).rev() {
        result.push(Piece::Ring(
            fill_radius + layer as f64 * 4.0 * scale,
            8.0 * scale,
            rgba(pink, 0.015 * (9 - layer) as f64),
        ));
    }
    let pulse = if complete {
        (1.0 - (rotation.rem_euclid(TAU) / 0.05 * context.spinner_rate) / 250.0).max(0.0)
    } else {
        0.0
    };
    result.push(Piece::Circle(fill_radius, rgba(pink, 0.4 + 0.2 * pulse)));
    let mut arc = |angle: f64, sweep: f64, r: f64, thickness: f64, color: Rgba| {
        let segments = (sweep.abs() * r / (3.0 * scale)).ceil().max(1.0) as usize;
        for i in 0..segments {
            let a = angle + sweep * i as f64 / segments as f64;
            let b = angle + sweep * (i + 1) as f64 / segments as f64;
            result.push(Piece::Line(
                (a.sin() * r, -a.cos() * r),
                (b.sin() * r, -b.cos() * r),
                thickness,
                color,
            ));
        }
    };
    let ring_sweep = TAU * if complete { 0.5 } else { 0.31 };
    for angle in [0.0, std::f64::consts::PI] {
        arc(
            angle - ring_sweep / 2.0,
            ring_sweep,
            (radius - 8.0 * scale).max(0.0),
            radius * if complete { 0.044 } else { 0.02 },
            white,
        );
    }
    if !complete {
        for angle in [std::f64::consts::FRAC_PI_2, -std::f64::consts::FRAC_PI_2] {
            arc(
                angle - TAU * 0.15 / 2.0,
                TAU * 0.15,
                radius * 0.94,
                radius * 0.12,
                rgba([255; 3], 0.25),
            );
            if progress > 0.0 {
                arc(
                    angle - TAU * 0.15 * progress / 2.0,
                    TAU * 0.15 * progress,
                    radius * 0.94,
                    radius * 0.12,
                    rgba([171, 255, 255], 1.0),
                );
            }
        }
    }
    let ambient = (elapsed + preempt / 2.0).max(0.0) * (25.0_f64.to_radians() * duration / 2000.0)
        / (preempt + duration);
    for i in 0..25 {
        let angle = i as f64 / 25.0 * TAU
            + rotation
            + ambient
            + std::f64::consts::PI * out_quint(after / 320.0);
        let x = angle.sin() * radius * 0.75;
        let y = -angle.cos() * radius * 0.75;
        let tilt = angle + 120.0_f64.to_radians();
        let dx = tilt.cos() * 15.0 * scale * disc_scale;
        let dy = tilt.sin() * 15.0 * scale * disc_scale;
        result.push(Piece::Line(
            (x - dx, y - dy),
            (x + dx, y + dy),
            5.0 * scale * disc_scale,
            white,
        ));
    }
    let tracking = (elapsed.max(0.0) / 150.0).clamp(0.0, 1.0);
    let centre_radius = (40.0 - 20.0 * tracking) * scale * (0.3 * entry + 0.5 * grow);
    result.push(Piece::Ring(centre_radius * 0.8, 10.0 * scale, white));
    result.push(Piece::Ring(centre_radius, 3.0 * scale, white));
    if elapsed >= 0.0 {
        result.push(Piece::Text("477".into(), 60.0 * scale, 28, 1.0, white));
        result.push(Piece::Text(
            "SPINS PER MINUTE".into(),
            90.0 * scale,
            16,
            1.0,
            white,
        ));
        let maximum = ((difficulty(context.spinner_od, 250.0, 380.0, 430.0) / 60000.0 * duration
            + 0.0001)
            .floor()
            - required
            - 2.0)
            .max(0.0);
        let bonus = (spins.floor() - required - 2.0).clamp(0.0, maximum);
        if bonus > 0.0 {
            let max = bonus >= maximum;
            let age = (spins - (required + 2.0 + bonus)) * TAU / 0.05 * context.spinner_rate;
            let opacity = (1.0 - age / if max { 500.0 } else { 1500.0 }).clamp(0.0, 1.0);
            let size = if max {
                1.5 + 1.3 * out_quint(age / 1000.0)
            } else {
                1.5 - 0.5 * out_quint(age / 1000.0)
            };
            result.push(Piece::Text(
                if max {
                    "MAX".into()
                } else {
                    format!("{}", (bonus * 100.0) as u32)
                },
                -100.0 * scale,
                28,
                size,
                rgba(if max { pink } else { [255; 3] }, opacity),
            ));
        }
    }
    result
}

pub fn draw_cpu(frame: &mut Img, context: &RenderContext, object: &StandardHitObject, time: i64) {
    let (x, y) = to_frame_point(256.0, 192.0, &context.frame_layout);
    let scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Standard,
        context.output_format,
    );
    for piece in pieces(context, object, time) {
        match piece {
            Piece::Circle(r, c) => frame.fill_circle_aa(x, y, r, c),
            Piece::Ring(r, w, c) => draw_ring_aa(frame, x, y, r, w, c),
            Piece::Line(a, b, w, c) => frame.draw_line(x + a.0, y + a.1, x + b.0, y + b.1, w, c),
            Piece::Text(text, offset, size, zoom, c) => {
                let sprite =
                    render_text_sprite(&text, scaled_bitmap_font_height(size, scale * zoom), c);
                frame.alpha_composite(
                    &sprite,
                    (x - sprite.w as f64 / 2.0).round() as i64,
                    (y + offset).round() as i64,
                );
            }
        }
    }
}

pub fn draw_gpu(
    scene: &mut FrameSceneBuilder,
    context: &RenderContext,
    object: &StandardHitObject,
    time: i64,
) {
    let (x, y) = to_frame_point(256.0, 192.0, &context.frame_layout);
    let scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Standard,
        context.output_format,
    );
    for piece in pieces(context, object, time) {
        match piece {
            Piece::Circle(r, c) => scene.circle([x as f32, y as f32], r as f32, c),
            Piece::Ring(r, w, c) => scene.ring([x as f32, y as f32], r as f32, w as f32, c),
            Piece::Line(a, b, w, c) => scene.line(
                [(x + a.0) as f32, (y + a.1) as f32],
                [(x + b.0) as f32, (y + b.1) as f32],
                w as f32,
                c,
            ),
            Piece::Text(text, offset, size, zoom, c) => {
                let sprite = Arc::new(render_text_sprite(
                    &text,
                    scaled_bitmap_font_height(size, scale * zoom),
                    [255; 4],
                ));
                let rect = SceneRect {
                    x: (x - sprite.w as f64 / 2.0) as f32,
                    y: (y + offset) as f32,
                    width: sprite.w as f32,
                    height: sprite.h as f32,
                };
                scene.glyph(sprite, rect, c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn autoplay_uses_real_time_at_all_rates() {
        for rate in [0.75, 1.0, 1.5] {
            assert!((autoplay_rotation(1000.0 * rate, rate) - 50.0).abs() < 1e-10);
        }
        assert_eq!(autoplay_rotation(-10.0, 1.0), 0.0);
    }
}
