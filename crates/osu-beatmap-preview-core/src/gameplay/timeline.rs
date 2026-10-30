//! 不可变稀疏状态索引；音频预读、倒序画面与并行消费者互不推进状态。
use super::events::GameplayEvent;
use super::state::{GameplayState, ObjectState, ReplayStatus};
use crate::gameplay::InputSnapshot;
use crate::replay::{ReplayDocument, ReplayTimeline};
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub struct TaikoPress {
    pub time: f64,
    pub key: u32,
    pub object_id: Option<usize>,
    pub node_id: Option<usize>,
}

pub struct GameplayTimeline {
    pub document: Arc<ReplayDocument>,
    pub policy: crate::replay::compatibility::ReplayPolicy,
    pub input: ReplayTimeline,
    pub taiko_inputs: Vec<TaikoPress>,
    pub events: Vec<GameplayEvent>,
    pub objects: Vec<ObjectState>,
    pub checkpoints: Vec<(f64, GameplayState)>,
    pub initial: GameplayState,
    pub failure_time: Option<f64>,
    pub settlement_time: f64,
    /// 原成绩差异作为诊断返回；画面与音频只消费独立重算的状态。
    pub recorded_differences: Option<serde_json::Value>,
}
impl GameplayTimeline {
    pub fn standard_policy(&self) -> crate::replay::compatibility::StandardPolicy {
        match self.policy {
            crate::replay::compatibility::ReplayPolicy::Standard(policy) => policy,
            crate::replay::compatibility::ReplayPolicy::Taiko(_)
            | crate::replay::compatibility::ReplayPolicy::Catch(_)
            | crate::replay::compatibility::ReplayPolicy::Mania(_) => {
                panic!("Standard consumer received Taiko replay")
            }
        }
    }
    pub fn state_at(&self, time: f64) -> &GameplayState {
        self.checkpoints
            .partition_point(|p| p.0 <= time)
            .checked_sub(1)
            .map_or(&self.initial, |i| &self.checkpoints[i].1)
    }
    pub fn events_between(&self, start: f64, end: f64) -> &[GameplayEvent] {
        let a = self.events.partition_point(|e| e.time <= start);
        let b = self.events.partition_point(|e| e.time <= end);
        &self.events[a..b.max(a)]
    }
    pub fn object_state_at(&self, id: usize, time: f64) -> Option<&ObjectState> {
        let state = self.objects.get(id)?;
        (state.head.is_some_and(|h| h.0 <= time) || state.end.is_some_and(|h| h.0 <= time))
            .then_some(state)
    }
    pub fn input_at(&self, time: f64) -> Option<InputSnapshot> {
        let mut input = self
            .input
            .snapshot_at(self.failure_time.map_or(time, |end| time.min(end)))?;
        if self.state_at(time).status != ReplayStatus::Playing {
            input.keys = 0;
        }
        Some(input)
    }
}
