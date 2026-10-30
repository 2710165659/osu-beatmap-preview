//! Mania 动态成绩：305 分最高判定、长条头尾统计与独立于 FPS 的连击部分。
use crate::{
    gameplay::{
        events::{GameplayEvent, HitResult},
        state::GameplayState,
    },
    replay::preparation::mania::PreparedMania,
};

pub(crate) struct Score {
    pub state: GameplayState,
    total: u32,
    judged: u32,
    base: u64,
    combo_portion: f64,
    maximum_combo_portion: f64,
    hp: f64,
    multiplier: f64,
    holds: Vec<bool>,
}
fn weight(combo: u32) -> f64 {
    (f64::from(combo).log(4.0)).clamp(0.5, 400_f64.log(4.0))
}
impl Score {
    pub fn new(prepared: &PreparedMania) -> Self {
        let total = prepared
            .objects
            .iter()
            .map(|o| if o.is_long_note { 2 } else { 1 })
            .sum();
        Self {
            state: GameplayState {
                accuracy: Some(1.0),
                ..Default::default()
            },
            total,
            judged: 0,
            base: 0,
            combo_portion: 0.0,
            maximum_combo_portion: (1..=total).map(|c| 300.0 * weight(c)).sum(),
            hp: prepared.hp,
            multiplier: prepared.score_multiplier,
            holds: prepared.objects.iter().map(|o| o.is_long_note).collect(),
        }
    }
    pub fn apply(&mut self, event: &GameplayEvent) -> bool {
        let previous = self.state.combo;
        if event.maximum == HitResult::IgnoreHit {
            self.state.combo = 0;
            return previous > 0;
        }
        *self
            .state
            .statistics
            .entry(event.result.name().into())
            .or_default() += 1;
        self.judged += 1;
        self.base += u64::from(event.result.value());
        if event.result.is_hit() {
            self.state.combo += 1;
            self.combo_portion +=
                f64::from(event.result.value().min(300)) * weight(self.state.combo);
            self.state.hit_error_ms = event.error;
        } else {
            self.state.combo = 0;
        }
        // Mania 不随时间扣血；长条头尾的 Miss 损失是普通单键的一半。
        // 回血使用基础增量，暂不模拟 LegacyDrainingHealthProcessor 的谱面自适应倍率。
        let change = match event.result {
            HitResult::Perfect => 0.0055 - self.hp * 0.0005,
            HitResult::Great => 0.005 - self.hp * 0.0005,
            HitResult::Good => 0.004 - self.hp * 0.0004,
            HitResult::Meh => -(self.hp + 1.0) * 0.0016,
            HitResult::Miss => {
                -(self.hp + 1.0)
                    * if self.holds[event.object_id] {
                        0.00375
                    } else {
                        0.0075
                    }
            }
            _ => 0.0,
        };
        self.state.health = (self.state.health + change).clamp(0.0, 1.0);
        self.state.max_combo = self.state.max_combo.max(self.state.combo);
        let accuracy = self.base as f64 / (305.0 * f64::from(self.judged));
        self.state.accuracy = Some(accuracy);
        let raw = (150_000.0 * self.combo_portion / self.maximum_combo_portion.max(1.0)
            + 850_000.0 * accuracy.powf(2.0 + 2.0 * accuracy) * f64::from(self.judged)
                / f64::from(self.total.max(1)))
        .round_ties_even();
        self.state.score = (raw * self.multiplier).round_ties_even() as u64;
        let imperfect = ["good", "ok", "meh", "miss"]
            .into_iter()
            .any(|k| self.state.statistics.get(k).copied().unwrap_or(0) > 0);
        self.state.rank = Some(
            (if !imperfect {
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
            })
            .into(),
        );
        !event.result.is_hit() && previous > 0
    }
}
