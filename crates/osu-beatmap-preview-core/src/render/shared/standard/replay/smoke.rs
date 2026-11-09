//! Standard 拉烟：按原始输入的距离采样，纯代码生成烟团；不可变数据支持跳转与并行导出。

use super::overlay::OverlayCanvas;
use crate::{
    gameplay::timeline::GameplayTimeline,
    render::{canvas::Img, cpu::modes::standard::context::*},
    replay::ReplayTimeline,
};
use std::sync::Arc;

// 默认 TextureStore 的 ScaleAdjust=2，64px 资源的 DisplayWidth 为 32；
// DefaultSmokeSegment 用 DisplayWidth * 0.165 作半径，点距为半径的 7/8。
const RADIUS: f64 = 64.0 / 2.0 * 0.165;
const INTERVAL: f64 = RADIUS * 7.0 / 8.0;
const INITIAL_FADE: f64 = 4000.0;
const FINAL_FADE: f64 = 8000.0;

struct Point {
    position: [f64; 2],
    time: f64,
    angle: f64,
    rotation: f64,
}

struct Segment {
    start: f64,
    end: f64,
    points: Vec<Point>,
}

impl Segment {
    fn fade_duration(&self) -> f64 {
        (self.end - self.start).min(INITIAL_FADE)
    }

    fn lifetime_end(&self) -> f64 {
        self.end + FINAL_FADE + self.fade_duration() / 3.0 + self.fade_duration() / 2.0
    }

    fn alpha(&self, point: &Point, time: f64) -> f64 {
        if self.end.is_infinite() {
            let age = time - point.time;
            return if age > 0.0 {
                0.6 * (1.0 - (age / INITIAL_FADE).clamp(0.0, 1.0))
            } else {
                1.0
            };
        }
        let duration = self.fade_duration();
        let first = self.end - duration;
        let final_age = time - duration - first - (point.time - first) / 2.0;
        if final_age > 0.0 && point.time >= first {
            return 1.0 - (final_age / FINAL_FADE).clamp(0.0, 1.0).powi(5);
        }
        let age = time.min(self.end) - point.time;
        let mut alpha = if age > 0.0 {
            0.6 * (1.0 - (age / INITIAL_FADE).clamp(0.0, 1.0))
        } else {
            1.0
        };
        let re_age = time - duration - first - (point.time - first) / 3.0;
        if point.time > first && re_age > 0.0 {
            alpha += out_quint(re_age / 50.0) * (1.0 - alpha);
        }
        alpha
    }
}

pub(crate) struct ReplaySmoke {
    segments: Vec<Segment>,
    sprite: Arc<Img>,
}

impl ReplaySmoke {
    pub(super) fn new(context: &RenderContext, gameplay: &GameplayTimeline) -> Self {
        Self {
            segments: segments(&gameplay.input, gameplay.failure_time),
            sprite: Arc::new(smoke_sprite(context.frame_layout.scale)),
        }
    }

    pub(crate) fn draw(&self, target: &mut impl OverlayCanvas, context: &RenderContext, time: f64) {
        let end = self
            .segments
            .partition_point(|segment| segment.start <= time);
        for segment in &self.segments[..end] {
            if time >= segment.lifetime_end() {
                continue;
            }
            let first = time.min(segment.end) - segment.fade_duration();
            let start = segment.points.partition_point(|point| point.time < first);
            let end = segment.points.partition_point(|point| point.time <= time);
            for point in &segment.points[start..end] {
                let alpha = segment.alpha(point, time);
                if alpha <= 0.0 {
                    continue;
                }
                let age = time - point.time;
                let scale = 0.65 + 0.35 * out_quint(age / 1200.0);
                let rotation = point.angle + point.rotation * out_quint(age / 500.0);
                let (x, y) =
                    to_frame_point(point.position[0], point.position[1], &context.frame_layout);
                target.additive_transformed(
                    &self.sprite,
                    [x, y],
                    scale,
                    rotation.to_degrees(),
                    alpha,
                );
            }
        }
    }
}

fn segments(input: &ReplayTimeline, failure: Option<f64>) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut current: Option<Segment> = None;
    let mut last = [0.0; 2];
    let mut distance = 0.0;
    let mut index = 0;
    for frame in input.frames.iter() {
        let time = frame.time_ms as f64;
        if failure.is_some_and(|end| time >= end) {
            break;
        }
        if frame.keys & 16 == 0 {
            if let Some(mut segment) = current.take() {
                segment.end = time;
                segments.push(segment);
            }
            continue;
        }
        let position = frame.cursor.map(f64::from);
        // 无效坐标不参与距离插值，避免跨过坏帧生成无限点列。
        if !position.iter().all(|v| v.is_finite()) {
            continue;
        }
        if current.is_none() {
            let mut segment = Segment {
                start: time,
                end: f64::INFINITY,
                points: Vec::new(),
            };
            add_point(&mut segment, position, time, &mut index);
            current = Some(segment);
            distance = 0.0;
            last = position;
            continue;
        }
        let segment = current.as_mut().unwrap();
        let delta = [position[0] - last[0], position[1] - last[1]];
        let length = delta[0].hypot(delta[1]);
        if length > 0.0 {
            let mut offset = INTERVAL - distance;
            while offset <= length {
                let factor = offset / length;
                add_point(
                    segment,
                    [last[0] + delta[0] * factor, last[1] + delta[1] * factor],
                    time,
                    &mut index,
                );
                offset += INTERVAL;
            }
            distance = (distance + length) % INTERVAL;
        }
        last = position;
    }
    if let Some(mut segment) = current {
        // 失败等价于松键；正常回放最后一帧仍按住时保持烟段，不虚构松键事件。
        if let Some(end) = failure {
            segment.end = end;
        }
        segments.push(segment);
    }
    segments
}

fn add_point(segment: &mut Segment, position: [f64; 2], time: f64, index: &mut u32) {
    segment.points.push(Point {
        position,
        time,
        angle: random(*index, 0) * std::f64::consts::TAU,
        rotation: (random(*index, 1) * 2.0 - 1.0) * 0.25,
    });
    *index += 1;
}

fn random(index: u32, salt: u32) -> f64 {
    let mut value = index
        .wrapping_add(salt.wrapping_mul(0x9e3779b9))
        .wrapping_add(0x85ebca6b);
    value = (value ^ (value >> 16)).wrapping_mul(0x7feb352d);
    value = (value ^ (value >> 15)).wrapping_mul(0x846ca68b);
    f64::from(value ^ (value >> 16)) / f64::from(u32::MAX)
}

fn out_quint(value: f64) -> f64 {
    1.0 - (1.0 - value.clamp(0.0, 1.0)).powi(5)
}

fn smoke_sprite(scale: f64) -> Img {
    let radius = RADIUS * scale;
    let side = (radius * 2.0).ceil().max(1.0) as u32 + 2;
    let center = f64::from(side) / 2.0;
    let mut image = Img::new(side, side, [0; 4]);
    // 默认烟图的平均 alpha 约 0.10，内部也有透明空隙，不能以实心白色轮廓代替。
    // 多频连续噪声形成烟絮，径向包络软化边缘；子像素积分保留小倍率下的稀疏度。
    for y in 0..side {
        for x in 0..side {
            let mut coverage = 0.0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let dx = (f64::from(x) + (f64::from(sx) + 0.5) / 4.0 - center) / radius;
                    let dy = (f64::from(y) + (f64::from(sy) + 0.5) / 4.0 - center) / radius;
                    let r2 = dx * dx + dy * dy;
                    if r2 >= 1.0 {
                        continue;
                    }
                    let u = (dx + 1.0) / 2.0;
                    let v = (dy + 1.0) / 2.0;
                    let noise = 0.55 * smoke_noise(u * 8.0, v * 8.0)
                        + 0.30 * smoke_noise(u * 16.0, v * 16.0)
                        + 0.15 * smoke_noise(u * 32.0, v * 32.0);
                    coverage +=
                        (1.8 * noise.powi(2) * (-3.5 * r2).exp() * (1.0 - r2.powi(4))).min(0.72);
                }
            }
            let alpha = (coverage / 16.0 * 255.0).round() as u8;
            image.put(x, y, [255, 255, 255, alpha]);
        }
    }
    image
}

fn smoke_noise(x: f64, y: f64) -> f64 {
    let ix = x.floor() as u32;
    let iy = y.floor() as u32;
    let smooth = |v: f64| v * v * (3.0 - 2.0 * v);
    let fx = smooth(x.fract());
    let fy = smooth(y.fract());
    let top = random(ix, iy) * (1.0 - fx) + random(ix + 1, iy) * fx;
    let bottom = random(ix, iy + 1) * (1.0 - fx) + random(ix + 1, iy + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::ReplayFrame;

    fn input(frames: &[(i64, f32, u32)]) -> ReplayTimeline {
        ReplayTimeline::new(
            frames
                .iter()
                .enumerate()
                .map(|(sequence, &(time_ms, x, keys))| ReplayFrame {
                    delta_ms: 0,
                    time_ms,
                    cursor: [x, 100.0],
                    keys,
                    sequence,
                })
                .collect(),
        )
    }

    #[test]
    fn distance_sampling_preserves_stationary_points_and_separates_presses() {
        let input = input(&[
            (0, 0.0, 16),
            (10, 0.0, 16),
            (20, 30.0, 16),
            (30, 100.0, 0),
            (40, 200.0, 16),
            (50, 200.0, 0),
        ]);
        let smoke = segments(&input, None);
        assert_eq!(smoke.len(), 2);
        assert_eq!(smoke[0].points.len(), 7);
        assert_eq!(smoke[0].end, 30.0);
        assert_eq!(smoke[1].points.len(), 1);
        assert_eq!(smoke[1].points[0].position, [200.0, 100.0]);
        for (i, point) in smoke[0].points.iter().enumerate() {
            assert!((point.position[0] - i as f64 * INTERVAL).abs() < 1e-6);
        }
        let failed = segments(&input, Some(15.0));
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].end, 15.0);
        assert_eq!(failed[0].points.len(), 1);
        assert!(segments(&self::input(&[(0, 0.0, 15), (20, 100.0, 0)]), None).is_empty());
    }

    #[test]
    fn smoke_remains_translucent_and_wispy_at_different_scales() {
        for scale in [0.75, 1.0, 2.0, 6.0] {
            let sprite = smoke_sprite(scale);
            let alpha: Vec<_> = sprite.data.chunks_exact(4).map(|pixel| pixel[3]).collect();
            let mass = alpha.iter().map(|&a| f64::from(a) / 255.0).sum::<f64>();
            let mean = mass / (2.0 * RADIUS * scale).powi(2);
            assert!(
                (0.06..0.14).contains(&mean),
                "烟团平均 alpha 超出参考范围：{scale}/{mean}"
            );
            assert!(alpha.iter().all(|&a| a <= 184), "不能重新变成实心白色粒子");
        }
    }

    #[test]
    fn fade_matches_hold_relight_and_final_disappearance() {
        let mut segment = Segment {
            start: 0.0,
            end: 1000.0,
            points: Vec::new(),
        };
        let mut index = 0;
        add_point(&mut segment, [0.0; 2], 500.0, &mut index);
        let point = &segment.points[0];
        assert!((segment.alpha(point, 750.0) - 0.5625).abs() < 1e-9);
        assert!(segment.alpha(point, 1220.0) > segment.alpha(point, 1000.0));
        assert!(segment.alpha(point, 2000.0) > 0.99);
        assert_eq!(segment.alpha(point, segment.lifetime_end()), 0.0);
        segment.end = f64::INFINITY;
        assert_eq!(segment.alpha(&segment.points[0], 4500.0), 0.0);
    }

    #[test]
    fn both_targets_keep_smoke_when_seeking_with_fractional_layout_scales() {
        use crate::{
            processing::timeline::TimeAxis,
            render::{
                geometry::OutputFormat,
                scene::{DrawCommand, FrameSceneBuilder},
            },
        };
        let raw = b"osu file format v14\n[General]\nMode:0\n[Difficulty]\nCircleSize:5\nSliderMultiplier:1.4\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n100,100,1000,1,0\n";
        let beatmap = crate::parse_beatmap_bytes(raw).unwrap();
        let prepared = crate::replay::preparation::standard::prepare(&beatmap).unwrap();
        let input = input(&[(0, 100.0, 16), (100, 130.0, 16), (200, 130.0, 0)]);
        for format in [OutputFormat::Png, OutputFormat::Gif, OutputFormat::Mp4] {
            let mut context =
                build_replay_context(&beatmap, &prepared, TimeAxis::new(1000), format);
            context.frame_layout.scale *= 0.75;
            let smoke = ReplaySmoke {
                segments: segments(&input, None),
                sprite: Arc::new(smoke_sprite(context.frame_layout.scale)),
            };
            let width = context.frame_layout.frame_width as u32;
            let height = context.frame_layout.frame_height as u32;
            let mut expected = None;
            for time in [500.0, 12000.0, -1.0, 500.0] {
                let mut image = Img::new(width, height, [0; 4]);
                smoke.draw(&mut image, &context, time);
                let mut scene = FrameSceneBuilder::new(width, height, time as i64);
                smoke.draw(&mut scene, &context, time);
                let scene = scene.finish();
                if time != 500.0 {
                    assert!(image.data.iter().all(|byte| *byte == 0));
                    assert!(scene.commands.is_empty());
                    continue;
                }
                assert!(image.data.chunks_exact(4).any(|pixel| pixel[3] > 0));
                let center = to_frame_point(100.0, 100.0, &context.frame_layout);
                assert!(matches!(scene.commands[0], DrawCommand::TransformedSprite {
                    position, additive: true, ..
                } if position == [center.0 as f32, center.1 as f32]));
                assert_eq!(
                    scene.resources.len(),
                    1,
                    "烟团在 GPU 上共享同一个程序生成资源"
                );
                if let Some(expected) = &expected {
                    assert_eq!(&image.data, expected);
                } else {
                    expected = Some(image.data);
                }
            }
        }
    }
}
