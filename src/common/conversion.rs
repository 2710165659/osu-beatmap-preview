//! 谱面转换共享辅助函数（standard → taiko/catch/mania）。
//! 由各模式的 conv.rs 模块使用。

use crate::core::models::{Beatmap, StandardHitObject, TimingPoint};

/// 随对象按时间顺序处理，单调推进已排序的 timing points，避免反复线性扫描造成的 O(n×m) 开销。
pub(crate) struct TimingCursor<'a> {
    points: &'a [TimingPoint],
    index: usize,
    pub beat_length: f64,
    pub slider_velocity: f64,
    pub meter: i32,
    pub kiai: bool,
}

impl<'a> TimingCursor<'a> {
    pub fn new(points: &'a [TimingPoint]) -> Self {
        let mut c = Self {
            points,
            index: 0,
            beat_length: 500.0,
            slider_velocity: 1.0,
            meter: 4,
            kiai: false,
        };
        while c.index < c.points.len() && c.points[c.index].time <= 0.0 {
            c.apply(c.points[c.index]);
            c.index += 1;
        }
        c
    }

    pub fn advance_to(&mut self, time_ms: i64) {
        let t = time_ms as f64;
        while self.index < self.points.len() && self.points[self.index].time <= t {
            self.apply(self.points[self.index]);
            self.index += 1;
        }
    }

    #[inline]
    fn apply(&mut self, p: TimingPoint) {
        if p.uninherited {
            self.beat_length = p.beat_length;
            self.slider_velocity = 1.0;
            self.meter = p.meter;
        } else if p.beat_length < 0.0 {
            self.slider_velocity = -100.0 / p.beat_length;
        }
        self.kiai = p.kiai_mode;
    }
}

/// 对应 `math.isclose(a, b, abs_tol=1e-7)`，并保留默认 `rel_tol=1e-9`。
pub(crate) fn almost_equals(a: f64, b: f64) -> bool {
    (a - b).abs() <= f64::max(1e-9 * f64::max(a.abs(), b.abs()), 1e-7)
}

/// 便捷地获取 standard 音符对象切片。
pub(crate) fn std_objects(beatmap: &Beatmap) -> &[StandardHitObject] {
    beatmap.hit_objects.as_standard().unwrap_or(&[])
}
