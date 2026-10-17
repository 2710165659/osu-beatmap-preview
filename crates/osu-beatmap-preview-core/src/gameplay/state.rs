//! 回放动态快照及对象轨迹；不包含输出格式、字体或像素尺寸。
use super::events::HitResult;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ReplayStatus {
    Playing,
    Completed,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GameplayState {
    pub score: u64,
    pub accuracy: Option<f64>,
    pub combo: u32,
    pub max_combo: u32,
    pub statistics: BTreeMap<String, u32>,
    pub rank: Option<String>,
    pub hit_error_ms: Option<f64>,
    pub health: f64,
    pub status: ReplayStatus,
}
impl Default for GameplayState {
    fn default() -> Self {
        Self {
            score: 0,
            accuracy: None,
            combo: 0,
            max_combo: 0,
            statistics: BTreeMap::new(),
            rank: None,
            hit_error_ms: None,
            health: 1.0,
            status: ReplayStatus::Playing,
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct ObjectState {
    pub head: Option<(f64, HitResult)>,
    pub end: Option<(f64, HitResult)>,
    pub nodes: Vec<Option<(f64, HitResult)>>,
    pub tracking: Vec<(f64, bool)>,
    pub rotation: Vec<(f64, f32, f32)>,
    /// 由旋转速度与缓动盘角决定发声，不能把按住按键当作正在旋转。
    pub spinning: Vec<(f64, bool)>,
}
impl ObjectState {
    pub fn spinning_at(&self, time: f64) -> bool {
        self.spinning
            .partition_point(|p| p.0 <= time)
            .checked_sub(1)
            .is_some_and(|i| self.spinning[i].1)
    }
    pub fn tracking_at(&self, time: f64) -> bool {
        self.tracking
            .partition_point(|p| p.0 <= time)
            .checked_sub(1)
            .is_some_and(|i| self.tracking[i].1)
    }
    pub fn rotation_at(&self, time: f64) -> (f32, f32) {
        self.rotation
            .partition_point(|p| p.0 <= time)
            .checked_sub(1)
            .map_or((0.0, 0.0), |i| (self.rotation[i].1, self.rotation[i].2))
    }
}
