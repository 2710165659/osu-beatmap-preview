//! 原始帧索引及显示插值；边沿只在原帧上生成，不由输出 FPS 决定。

use super::ReplayFrame;
use crate::gameplay::InputSnapshot;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ReplayTimeline {
    pub frames: Arc<[ReplayFrame]>,
}
impl ReplayTimeline {
    pub fn new(frames: Vec<ReplayFrame>) -> Self {
        Self {
            frames: frames.into(),
        }
    }
    pub fn last_time(&self) -> i64 {
        self.frames.last().unwrap().time_ms
    }
    pub fn snapshot_at(&self, time: f64) -> Option<InputSnapshot> {
        let next = self
            .frames
            .partition_point(|frame| frame.time_ms as f64 <= time);
        let before = self.frames.get(next.checked_sub(1)?)?;
        let mut cursor = before.cursor;
        if let Some(after) = self.frames.get(next) {
            let span = (after.time_ms - before.time_ms) as f64;
            if span > 0.0 {
                let factor = ((time - before.time_ms as f64) / span) as f32;
                cursor = [
                    cursor[0] + (after.cursor[0] - cursor[0]) * factor,
                    cursor[1] + (after.cursor[1] - cursor[1]) * factor,
                ];
            }
        }
        Some(InputSnapshot {
            cursor: Some(cursor),
            keys: before.keys,
            pressed: 0,
            released: 0,
        })
    }
    /// `(start,end]` 查询包含同刻的全部帧，并保留原序号。
    pub fn events_between(&self, start: i64, end: i64) -> &[ReplayFrame] {
        let a = self.frames.partition_point(|f| f.time_ms <= start);
        let b = self.frames.partition_point(|f| f.time_ms <= end);
        &self.frames[a..b.max(a)]
    }
}
/// 鼠标和键盘来源位映射到两个动作；K1+M1 只能产生一次左键动作。
pub fn standard_actions(keys: u32) -> u32 {
    u32::from(keys & 5 != 0) | (u32::from(keys & 10 != 0) << 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interpolation_is_unclipped_and_keys_are_steps() {
        let timeline = ReplayTimeline::new(vec![
            ReplayFrame {
                delta_ms: -10,
                time_ms: -10,
                cursor: [-100.0, 0.0],
                keys: 0,
                sequence: 0,
            },
            ReplayFrame {
                delta_ms: 20,
                time_ms: 10,
                cursor: [100.0, 20.0],
                keys: 5,
                sequence: 1,
            },
            ReplayFrame {
                delta_ms: 0,
                time_ms: 10,
                cursor: [100.0, 20.0],
                keys: 0,
                sequence: 2,
            },
        ]);
        let state = timeline.snapshot_at(0.0).unwrap();
        assert_eq!(state.cursor, Some([0.0, 10.0]));
        assert_eq!(state.keys, 0);
        assert_eq!(timeline.events_between(0, 10).len(), 2);
        assert_eq!(standard_actions(5), 1);
        assert_eq!(standard_actions(15), 3);
    }
}
