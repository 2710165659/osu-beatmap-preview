//! FL 使用回放光标、真实连击与滑条追踪；尺寸动画预计算以支持乱序查询。
use super::*;
use crate::render::visibility::{AutoplayPath, Flashlight};

pub(super) struct ReplayFlashlight {
    path: AutoplayPath,
    // 时间、起始尺寸、目标尺寸、动画时长均在谱面时间域。
    sizes: Vec<(f64, f64, f64, f64)>,
    size: f64,
    tracking: Vec<(f64, f64)>,
}

impl ReplayFlashlight {
    pub fn new(
        gameplay: &GameplayTimeline,
        cursor: &super::super::autoplay::AutoplayCursor,
        mods: &crate::replay::mods::ReplayMods,
        breaks: &[BreakPeriod],
    ) -> Self {
        let rate = mods.settings.speed_multiplier;
        let path = cursor.flashlight_path_with_delay(rate, mods.flashlight_delay);
        let mut times: Vec<_> = gameplay.checkpoints.iter().map(|p| p.0).collect();
        for period in breaks {
            times.extend([period.start_time as f64, period.end_time as f64]);
        }
        let mut sizes = Vec::new();
        times.sort_by(f64::total_cmp);
        times.dedup();
        let mut from = 1.0_f64;
        let mut target = 1.0_f64;
        let mut start = f64::NEG_INFINITY;
        let mut duration = 0.0;
        for time in times {
            let combo = gameplay.state_at(time).combo;
            let in_break = breaks
                .iter()
                .any(|period| period.start_time as f64 <= time && time < period.end_time as f64);
            let next = if in_break {
                2.5
            } else if !mods.flashlight_combo {
                1.0
            } else if combo >= 200 {
                0.625
            } else if combo >= 100 {
                0.8125
            } else {
                1.0
            };
            if next == target {
                continue;
            }
            from = interpolate(from, target, time - start, duration);
            target = next;
            duration = (from - target).abs() / 0.001875 * rate;
            start = time;
            sizes.push((start, from, target, duration));
        }
        // 合并追踪区间；逐帧只二分查询，不能随全谱物件数线性扫描。
        let mut intervals = Vec::new();
        for state in &gameplay.objects {
            for pair in state.tracking.windows(2) {
                if pair[0].1 {
                    intervals.push((pair[0].0, pair[1].0));
                }
            }
        }
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut tracking: Vec<(f64, f64)> = Vec::new();
        for (start, end) in intervals {
            if let Some(last) = tracking.last_mut().filter(|last| last.1 >= start) {
                last.1 = last.1.max(end);
            } else {
                tracking.push((start, end));
            }
        }
        Self {
            path,
            sizes,
            size: mods.flashlight_size,
            tracking,
        }
    }

    pub fn at(&self, context: &RenderContext, time: i64) -> Flashlight {
        let index = self.sizes.partition_point(|p| p.0 <= time as f64);
        let scale = index.checked_sub(1).map_or(1.0, |index| {
            let (start, from, target, duration) = self.sizes[index];
            interpolate(from, target, time as f64 - start, duration)
        });
        Flashlight {
            center: self.path.position_at(time),
            radius: 125.0 * context.frame_layout.scale * self.size * scale,
            band: false,
            dim: if self
                .tracking
                .partition_point(|p| p.0 <= time as f64)
                .checked_sub(1)
                .is_some_and(|index| (time as f64) < self.tracking[index].1)
            {
                0.8
            } else {
                0.0
            },
        }
    }
}

fn interpolate(from: f64, to: f64, elapsed: f64, duration: f64) -> f64 {
    if duration <= 0.0 {
        to
    } else {
        from + (to - from) * (elapsed / duration).clamp(0.0, 1.0)
    }
}
