//! 当前官方标准化计分；历史回放同样重算，不在结尾覆盖为文件记录的总分。
use crate::gameplay::events::{GameplayEvent, HitResult};
use crate::gameplay::state::GameplayState;

pub struct Score {
    pub state: GameplayState,
    base: f64,
    maximum_base: f64,
    count: u32,
    combo_portion: f64,
    bonus: f64,
    maximum_combo_portion: f64,
    maximum_count: u32,
    multiplier: f64,
}
impl Score {
    pub fn new(maximums: &[HitResult], multiplier: f64) -> Self {
        let mut combo = 0_u32;
        let mut maximum_combo_portion = 0.0;
        let mut maximum_count = 0;
        for &result in maximums {
            if matches!(
                result,
                HitResult::Great
                    | HitResult::LargeTickHit
                    | HitResult::SmallTickHit
                    | HitResult::SliderTailHit
            ) {
                if result != HitResult::SmallTickHit {
                    combo += 1;
                }
                maximum_count += 1;
                maximum_combo_portion += result.value() as f64 * (combo as f64).sqrt();
            }
        }
        Self {
            state: GameplayState::default(),
            base: 0.0,
            maximum_base: 0.0,
            count: 0,
            combo_portion: 0.0,
            bonus: 0.0,
            maximum_combo_portion,
            maximum_count,
            multiplier,
        }
    }
    pub fn apply(&mut self, event: &GameplayEvent) -> bool {
        let previous_combo = self.state.combo;
        let max = event.maximum;
        let result = event.result;
        *self
            .state
            .statistics
            .entry(result.name().into())
            .or_default() += 1;
        if matches!(max, HitResult::SmallBonus | HitResult::LargeBonus) {
            self.bonus += result.value() as f64;
        } else if matches!(
            max,
            HitResult::Great
                | HitResult::LargeTickHit
                | HitResult::SmallTickHit
                | HitResult::SliderTailHit
        ) {
            self.maximum_base += max.value() as f64;
            self.base += result.value() as f64;
            self.count += 1;
            // CL 的尾部 SmallTick 影响 ACC 和组合得分，但不改变连击；
            // 普通尾部 IgnoreMiss 影响 ACC，也不计入组合部分。
            if result != HitResult::IgnoreMiss {
                if max != HitResult::SmallTickHit {
                    self.state.combo = if result.is_hit() {
                        self.state.combo + 1
                    } else {
                        0
                    };
                }
                self.state.max_combo = self.state.max_combo.max(self.state.combo);
                self.combo_portion += max.value() as f64 * (self.state.combo as f64).sqrt();
            }
        }
        if event.error.is_some() {
            self.state.hit_error_ms = event.error;
        }
        if self.maximum_base > 0.0 {
            let acc = self.base / self.maximum_base;
            self.state.accuracy = Some(acc);
            self.state.rank = Some(
                super::rank::rank(acc, *self.state.statistics.get("miss").unwrap_or(&0)).into(),
            );
            let combo_progress = self.combo_portion / self.maximum_combo_portion.max(1.0);
            let accuracy_progress = self.count as f64 / self.maximum_count.max(1) as f64;
            let without_mods = (500_000.0 * acc * combo_progress
                + 500_000.0 * acc.powi(5) * accuracy_progress
                + self.bonus)
                .round_ties_even();
            // 官方先将无 Mod 分数取整，再乘 Mod 倍率取整；不能只在最后取整一次。
            self.state.score = (without_mods * self.multiplier).round_ties_even() as u64;
        }
        previous_combo > 20 && self.state.combo == 0
    }
}
