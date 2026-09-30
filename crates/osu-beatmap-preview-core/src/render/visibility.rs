//! HD/FL 的共享可见性规则。没有回放输入时，轨迹和连击来自谱面的自动预览。

use std::sync::Arc;

use crate::domain::models::{BreakPeriod, TimingPoint};
use crate::render::canvas::Img;
use crate::render::geometry::PixelRect;
use crate::render::scene::{FrameSceneBuilder, SceneRect};

#[derive(Clone, Copy)]
pub enum FlashlightKind {
    Standard,
    Taiko,
    Catch,
    Mania,
}

/// 满连击时间轴独立于帧索引；分段、seek、并行和乱序出帧都得到相同结果。
pub struct VisibilityTimeline {
    hit_times: Vec<f64>,
    breaks: Vec<BreakPeriod>,
}

impl VisibilityTimeline {
    pub fn new(hit_times: impl IntoIterator<Item = f64>, breaks: &[BreakPeriod]) -> Self {
        let mut hit_times: Vec<_> = hit_times
            .into_iter()
            .filter(|time| time.is_finite())
            .collect();
        hit_times.sort_by(f64::total_cmp);
        Self {
            hit_times,
            breaks: breaks.to_vec(),
        }
    }

    pub fn combo_at(&self, time: i64) -> usize {
        self.hit_times.partition_point(|&hit| hit <= time as f64)
    }

    pub fn is_break(&self, time: i64) -> bool {
        self.breaks
            .iter()
            .any(|period| period.start_time <= time && time < period.end_time)
    }

    /// 对齐 ModFlashlight/CatchModFlashlight 的 100/200 combo 尺寸与休息段扩张。
    /// 动画按谱面时间计算，不能按导出帧累积，否则 DT/HT 或随机 seek 会改变遮罩。
    pub fn flashlight_scale(&self, kind: FlashlightKind, time: i64) -> f64 {
        let mut changes = Vec::with_capacity(2 + self.breaks.len() * 2);
        if !matches!(kind, FlashlightKind::Mania) {
            for threshold in [100, 200] {
                if let Some(&hit) = self.hit_times.get(threshold - 1) {
                    changes.push(hit);
                }
            }
        }
        for period in &self.breaks {
            changes.push(period.start_time as f64);
            changes.push(period.end_time as f64);
        }
        changes.sort_by(f64::total_cmp);
        changes.dedup();
        let mut previous_time = f64::NEG_INFINITY;
        let mut from = 1.0;
        let mut target = 1.0;
        let mut duration = 0.0;
        for change in changes
            .into_iter()
            .take_while(|&change| change <= time as f64)
        {
            let current = interpolate(from, target, change - previous_time, duration);
            let in_break = self.breaks.iter().any(|period| {
                period.start_time as f64 <= change && change < period.end_time as f64
            });
            let combo = self.hit_times.partition_point(|&hit| hit <= change);
            let next = if in_break {
                if matches!(kind, FlashlightKind::Catch) {
                    1.538
                } else {
                    2.5
                }
            } else {
                match (kind, combo) {
                    (FlashlightKind::Mania, _) => 1.0,
                    (FlashlightKind::Catch, 200..) => 0.770,
                    (FlashlightKind::Catch, 100..) => 0.885,
                    (_, 200..) => 0.625,
                    (_, 100..) => 0.8125,
                    _ => 1.0,
                }
            };
            from = current;
            target = next;
            duration = match kind {
                FlashlightKind::Mania => 800.0,
                FlashlightKind::Catch => (from - target).abs() / 0.001154,
                _ => (from - target).abs() / 0.001875,
            };
            previous_time = change;
        }
        interpolate(from, target, time as f64 - previous_time, duration)
    }
}

fn interpolate(from: f64, to: f64, elapsed: f64, duration: f64) -> f64 {
    if duration <= 0.0 {
        return to;
    }
    from + (to - from) * (elapsed / duration).clamp(0.0, 1.0)
}

/// 红线和绿线都可以切换 Kiai；按绝对谱面时间查询效果状态。
/// 状态不依赖 BPM、出帧顺序或倍速，区段内的可见性不会随节拍闪烁。
pub struct KiaiTimeline {
    effects: Vec<(f64, bool)>,
}

impl KiaiTimeline {
    pub fn new(points: &[TimingPoint]) -> Self {
        let mut effects: Vec<_> = points
            .iter()
            .filter(|point| point.time.is_finite())
            .map(|point| (point.time, point.kiai_mode))
            .collect();
        effects.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { effects }
    }

    pub fn is_active_at(&self, time: i64) -> bool {
        let end = self.effects.partition_point(|point| point.0 <= time as f64);
        end.checked_sub(1)
            .is_some_and(|index| self.effects[index].1)
    }
}

/// 物件之间线性移动；滑条可加入路径节点。可选的连续阻尼近似 FL 光圈跟随延迟。
/// 在每个节点保存解析解，查询不会依赖上次渲染时间或 fps。
pub struct AutoplayPath {
    points: Vec<(f64, [f64; 2])>,
    followed: Vec<[f64; 2]>,
    follow_time_constant: f64,
}

impl AutoplayPath {
    pub fn new(mut points: Vec<(f64, [f64; 2])>, follow_time_constant: f64) -> Self {
        points.retain(|(time, point)| time.is_finite() && point.iter().all(|v| v.is_finite()));
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        if points.is_empty() {
            points.push((0.0, [256.0, 192.0]));
        }
        let mut followed = vec![points[0].1];
        for pair in points.windows(2) {
            followed.push(follow_segment(
                pair[0],
                pair[1],
                *followed.last().unwrap(),
                pair[1].0,
                follow_time_constant,
            ));
        }
        Self {
            points,
            followed,
            follow_time_constant,
        }
    }

    pub fn position_at(&self, time: i64) -> [f64; 2] {
        let time = time as f64;
        let index = self
            .points
            .partition_point(|point| point.0 <= time)
            .saturating_sub(1);
        let from = self.points[index];
        let to = self
            .points
            .get(index + 1)
            .copied()
            .unwrap_or((time.max(from.0), from.1));
        follow_segment(
            from,
            to,
            self.followed[index],
            time.max(from.0),
            self.follow_time_constant,
        )
    }
}

fn follow_segment(
    from: (f64, [f64; 2]),
    to: (f64, [f64; 2]),
    followed: [f64; 2],
    time: f64,
    tau: f64,
) -> [f64; 2] {
    let elapsed = (time - from.0).max(0.0);
    let duration = to.0 - from.0;
    if tau <= 0.0 {
        let t = if duration > 0.0 {
            (elapsed / duration).clamp(0.0, 1.0)
        } else {
            1.0
        };
        return [
            from.1[0] + (to.1[0] - from.1[0]) * t,
            from.1[1] + (to.1[1] - from.1[1]) * t,
        ];
    }
    let decay = (-elapsed / tau).exp();
    std::array::from_fn(|axis| {
        let velocity = if duration > 0.0 {
            (to.1[axis] - from.1[axis]) / duration
        } else {
            0.0
        };
        from.1[axis]
            + velocity * (elapsed - tau)
            + (followed[axis] - from.1[axis] + velocity * tau) * decay
    })
}

#[derive(Clone, Copy)]
pub struct Flashlight {
    pub center: [f64; 2],
    pub radius: f64,
    /// Mania 是横向可视带，其余模式是圆形光圈。
    pub band: bool,
    /// Standard 跟踪滑条时可视部分额外暗化 80%。
    pub dim: f64,
}

impl Flashlight {
    pub fn opacity_at(&self, x: f64, y: f64) -> u8 {
        let distance = if self.band {
            (y - self.center[1]).abs()
        } else {
            (x - self.center[0]).hypot(y - self.center[1])
        };
        let smoothness = if self.band { 1.1 } else { 1.4 };
        // 对齐 osu-resources 的 CircularFlashlight/RectangularFlashlight smoothstep。
        let t = ((distance - self.radius) / (self.radius * (smoothness - 1.0)).max(f64::EPSILON))
            .clamp(0.0, 1.0);
        let alpha = t * t * (3.0 - 2.0 * t);
        ((self.dim + (1.0 - self.dim) * alpha).clamp(0.0, 1.0) * 255.0).round() as u8
    }

    pub fn apply(&self, image: &mut Img, rect: PixelRect) {
        for y in rect.y.max(0)..(rect.y + rect.height).min(image.h as i64) {
            for x in rect.x.max(0)..(rect.x + rect.width).min(image.w as i64) {
                image.blend_px(
                    x,
                    y,
                    [0, 0, 0, self.opacity_at(x as f64 + 0.5, y as f64 + 0.5)],
                );
            }
        }
    }

    /// 场景后端复用同一遮罩，避免 CPU 导出和 WGPU 预览出现不同的渐变边缘。
    pub fn draw_scene(&self, scene: &mut FrameSceneBuilder, rect: PixelRect) {
        let mut overlay = Img::new(rect.width as u32, rect.height as u32, [0, 0, 0, 0]);
        for y in 0..overlay.h {
            for x in 0..overlay.w {
                let index = overlay.idx(x, y) + 3;
                overlay.data[index] = self.opacity_at(
                    (rect.x + x as i64) as f64 + 0.5,
                    (rect.y + y as i64) as f64 + 0.5,
                );
            }
        }
        scene.sprite(
            Arc::new(overlay),
            SceneRect {
                x: rect.x as f32,
                y: rect.y as f32,
                width: rect.width as f32,
                height: rect.height as f32,
            },
            1.0,
        );
    }
}

#[derive(Clone, Copy, Default)]
pub struct ManiaHidden {
    fade_start: f64,
    covered_top: f64,
    enabled: bool,
}

impl ManiaHidden {
    /// ManiaModHidden：768 高坐标下覆盖 160..400 px，每 combo 增长 0.5 px。
    /// 遮罩只改变音符 alpha；判定线、键道和 SV 提示不受 HD 影响。
    pub fn new(
        timeline: &VisibilityTimeline,
        time: i64,
        top: i64,
        hit_y: i64,
        reference_scale: f64,
    ) -> Self {
        if timeline.is_break(time) {
            return Self::default();
        }
        let coverage = (160.0 + timeline.combo_at(time) as f64 * 0.5).min(400.0) * reference_scale;
        let covered_top = hit_y as f64 - coverage;
        Self {
            fade_start: covered_top - (hit_y - top) as f64 * 0.25,
            covered_top,
            enabled: true,
        }
    }

    pub fn alpha_at(&self, y: i64) -> u8 {
        if !self.enabled {
            return 255;
        }
        (((self.covered_top - (y as f64 + 0.5))
            / (self.covered_top - self.fade_start).max(f64::EPSILON))
        .clamp(0.0, 1.0)
            * 255.0)
            .round() as u8
    }

    pub fn apply(&self, notes: &mut Img, rect: PixelRect) {
        if !self.enabled {
            return;
        }
        let left = rect.x.max(0);
        let right = (rect.x + rect.width).min(notes.w as i64);
        for y in rect.y.max(0)..(rect.y + rect.height).min(notes.h as i64) {
            let alpha = self.alpha_at(y) as u32;
            for x in left..right {
                let index = notes.idx(x as u32, y as u32) + 3;
                notes.data[index] = ((notes.data[index] as u32 * alpha + 127) / 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kiai_switches_at_effect_boundaries_without_waiting_for_beats() {
        let beatmap = crate::parse_beatmap_bytes(
            b"osu file format v14\n[General]\nMode:2\n[Difficulty]\nCircleSize:5\nApproachRate:5\n[TimingPoints]\n0,500,4,1,0,100,1,0\n750,-100,4,1,0,100,0,1\n1200,-50,4,1,0,100,0,1\n1300,-100,4,1,0,100,0,0\n2000,250,4,1,0,100,1,1\n2500,-100,4,1,0,100,0,0\n[HitObjects]\n256,192,3000,1,0,0:0:0:0:\n",
        ).unwrap();
        let timeline = KiaiTimeline::new(&beatmap.timing_points);
        for time in [-1, 0, 749, 1300, 1999, 2500] {
            assert!(!timeline.is_active_at(time), "{time}");
        }
        for time in [
            750, 999, 1000, 1100, 1200, 1299, 2000, 2100, 2250, 2449, 2499,
        ] {
            assert!(timeline.is_active_at(time), "{time}");
        }
        let mut points = beatmap.timing_points;
        points.reverse();
        let shuffled = KiaiTimeline::new(&points);
        for time in [
            2250, 749, 750, 1300, 1100, 2000, 1000, 2500, 2499, 1200, 1000,
        ] {
            assert_eq!(timeline.is_active_at(time), shuffled.is_active_at(time));
        }
    }

    #[test]
    fn flashlight_has_clear_center_smooth_edge_and_opaque_outside() {
        let mask = Flashlight {
            center: [100.0, 100.0],
            radius: 50.0,
            band: false,
            dim: 0.0,
        };
        assert_eq!(mask.opacity_at(100.0, 100.0), 0);
        assert_eq!(mask.opacity_at(150.0, 100.0), 0);
        assert_eq!(mask.opacity_at(160.0, 100.0), 128);
        assert_eq!(mask.opacity_at(170.0, 100.0), 255);
        assert_eq!(
            Flashlight { dim: 0.8, ..mask }.opacity_at(100.0, 100.0),
            204
        );
        let band = Flashlight { band: true, ..mask };
        assert_eq!(band.opacity_at(10000.0, 100.0), 0);
        assert_eq!(band.opacity_at(100.0, 155.0), 255);
    }

    #[test]
    fn combo_thresholds_shrink_flashlight_and_breaks_expand_it() {
        let timeline = VisibilityTimeline::new(
            (1..=250).map(|n| n as f64 * 100.0),
            &[BreakPeriod {
                start_time: 26000,
                end_time: 28000,
            }],
        );
        assert_eq!(
            timeline.flashlight_scale(FlashlightKind::Standard, 9900),
            1.0
        );
        assert_eq!(
            timeline.flashlight_scale(FlashlightKind::Standard, 10500),
            0.8125
        );
        assert_eq!(
            timeline.flashlight_scale(FlashlightKind::Standard, 20500),
            0.625
        );
        assert_eq!(
            timeline.flashlight_scale(FlashlightKind::Catch, 20500),
            0.770
        );
        assert_eq!(timeline.flashlight_scale(FlashlightKind::Mania, 20500), 1.0);
        assert_eq!(
            timeline.flashlight_scale(FlashlightKind::Standard, 27500),
            2.5
        );
        assert_eq!(
            timeline.flashlight_scale(FlashlightKind::Standard, 29500),
            0.625
        );
    }

    #[test]
    fn hidden_cover_affects_hold_rows_and_grows_with_combo() {
        let timeline = VisibilityTimeline::new(
            (1..=500).map(|n| n as f64),
            &[BreakPeriod {
                start_time: 1000,
                end_time: 2000,
            }],
        );
        let early = ManiaHidden::new(&timeline, 0, 0, 658, 1.0);
        assert_eq!(early.alpha_at(200), 255);
        assert!((1..255).contains(&early.alpha_at(400)));
        assert_eq!(early.alpha_at(600), 0);
        let late = ManiaHidden::new(&timeline, 500, 0, 658, 1.0);
        assert_eq!(late.alpha_at(400), 0);
        assert_eq!(
            ManiaHidden::new(&timeline, 1000, 0, 658, 1.0).alpha_at(600),
            255
        );
        let mut notes = Img::new(4, 768, [30, 120, 220, 128]);
        early.apply(
            &mut notes,
            PixelRect {
                x: 1,
                y: 0,
                width: 2,
                height: 768,
            },
        );
        assert_eq!(notes.get(1, 200), [30, 120, 220, 128]);
        assert_eq!(notes.get(1, 600), [30, 120, 220, 0]);
        assert_eq!(notes.get(0, 600), [30, 120, 220, 128]);
        assert!((1..128).contains(&notes.get(1, 400)[3]));
    }

    #[test]
    fn autoplay_path_is_seek_order_independent_and_follows_reverse_segments() {
        let path = AutoplayPath::new(
            vec![
                (0.0, [0.0, 0.0]),
                (1000.0, [200.0, 100.0]),
                (2000.0, [0.0, 0.0]),
            ],
            0.0,
        );
        for time in [1500, 500, 1000, 500, -100, 3000] {
            let expected = match time {
                500 | 1500 => [100.0, 50.0],
                1000 => [200.0, 100.0],
                _ => [0.0, 0.0],
            };
            assert_eq!(path.position_at(time), expected);
        }
        let lagged = AutoplayPath::new(vec![(0.0, [0.0, 0.0]), (1000.0, [200.0, 100.0])], 60.0);
        let at = lagged.position_at(1000);
        assert!(at[0] > 180.0 && at[0] < 200.0);
        assert!(lagged.position_at(2000)[0] > at[0]);
    }
}
