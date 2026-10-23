//! Taiko 当前游戏计分与血量；奖励节点不参与 ACC、Combo 或普通命中回血。
use crate::domain::rulesets::taiko::gameplay::difficulty_range;
use crate::gameplay::{
    events::{GameplayEvent, HitResult},
    state::GameplayState,
};
use crate::replay::preparation::taiko::{PreparedTaiko, TaikoObjectKind};

pub(crate) struct Score {
    pub state: GameplayState,
    maximum_combo_portion: f64,
    combo_portion: f64,
    total: u32,
    judged: u32,
    base: u32,
    bonus: f64,
    multiplier: f64,
    health_gain: f64,
    health_loss: f64,
}

fn combo_weight(combo: u32) -> f64 {
    (f64::from(combo).log(4.0)).clamp(0.5, 400_f64.log(4.0))
}

impl Score {
    pub fn new(prepared: &PreparedTaiko) -> Self {
        let total = prepared
            .objects
            .iter()
            .filter(|o| matches!(o.kind, TaikoObjectKind::Hit))
            .count() as u32;
        Self {
            state: GameplayState {
                health: if total == 0 { 1.0 } else { 0.0 },
                ..Default::default()
            },
            maximum_combo_portion: (1..=total).map(|combo| 300.0 * combo_weight(combo)).sum(),
            combo_portion: 0.0,
            total,
            judged: 0,
            base: 0,
            bonus: 0.0,
            multiplier: prepared.score_multiplier,
            health_gain: 1.0
                / (3.0 * f64::from(total.max(1)) * difficulty_range(prepared.hp, 0.5, 0.75, 0.98)),
            health_loss: difficulty_range(prepared.hp, 0.0018, 0.0075, 0.012),
        }
    }

    pub fn apply(&mut self, event: &GameplayEvent, prepared: &PreparedTaiko) -> bool {
        let previous_combo = self.state.combo;
        *self
            .state
            .statistics
            .entry(event.result.name().into())
            .or_default() += 1;
        if event.maximum == HitResult::Great {
            self.judged += 1;
            let value = match event.result {
                HitResult::Great => 300,
                HitResult::Ok => 150,
                _ => 0,
            };
            self.base += value;
            if value != 0 {
                self.state.combo += 1;
                self.combo_portion += f64::from(value) * combo_weight(self.state.combo);
                self.state.health += if value == 300 { 3.0 } else { 1.1 } * self.health_gain;
                self.state.hit_error_ms = event.error;
            } else {
                self.state.combo = 0;
                self.state.health -= self.health_loss;
            }
        } else if matches!(event.result, HitResult::SmallBonus | HitResult::LargeBonus) {
            let object = &prepared.objects[event.object_id];
            let scale = if object.strong()
                && event.node_id.is_some()
                && event.result == HitResult::LargeBonus
            {
                if matches!(object.kind, TaikoObjectKind::DrumRoll { .. }) {
                    3.0
                } else {
                    7.0
                }
            } else {
                1.0
            };
            self.bonus += f64::from(event.result.value()) * scale;
        } else if event.result == HitResult::IgnoreMiss
            && event.node_id.is_none()
            && matches!(
                prepared.objects[event.object_id].kind,
                TaikoObjectKind::Swell { .. }
            )
        {
            self.state.health -= 0.65 * self.health_loss;
        }
        self.state.health = self.state.health.clamp(0.0, 1.0);
        self.state.max_combo = self.state.max_combo.max(self.state.combo);
        let accuracy = if self.judged == 0 {
            1.0
        } else {
            f64::from(self.base) / (300.0 * f64::from(self.judged))
        };
        self.state.accuracy = Some(accuracy);
        let combo_progress = if self.maximum_combo_portion == 0.0 {
            1.0
        } else {
            self.combo_portion / self.maximum_combo_portion
        };
        let accuracy_progress = if self.total == 0 {
            1.0
        } else {
            f64::from(self.judged) / f64::from(self.total)
        };
        let raw = 250_000.0 * combo_progress
            + 750_000.0 * accuracy.powf(3.6) * accuracy_progress
            + self.bonus;
        self.state.score = (raw.round_ties_even() * self.multiplier).round_ties_even() as u64;
        let mut rank = if accuracy == 1.0 {
            "X"
        } else if accuracy >= 0.95 {
            "S"
        } else if accuracy >= 0.9 {
            "A"
        } else if accuracy >= 0.8 {
            "B"
        } else if accuracy >= 0.7 {
            "C"
        } else {
            "D"
        };
        if matches!(rank, "X" | "S") && self.state.statistics.get("miss").copied().unwrap_or(0) > 0
        {
            rank = "A";
        }
        self.state.rank = Some(rank.into());
        event.result == HitResult::Miss && previous_combo > 0
    }
}
