//! Catch 当前计分：水果和大水滴累计 Combo，小水滴独立计分，香蕉仅作奖励。
use crate::render::cpu::modes::catch::objects::ObjType;
use crate::{
    gameplay::{
        events::{GameplayEvent, HitResult},
        state::GameplayState,
    },
    replay::preparation::catch::PreparedCatch,
};

pub(crate) struct Score {
    pub state: GameplayState,
    maximum_combo: f64,
    combo: f64,
    tiny_total: u32,
    tiny_hit: u32,
    judged: u32,
    hit: u32,
    tiny_portion: f64,
    bonus: f64,
}
fn weight(combo: u32) -> f64 {
    (f64::from(combo).log(4.0)).clamp(0.5, 200_f64.log(4.0))
}
impl Score {
    pub fn new(prepared: &PreparedCatch) -> Self {
        let mut combo = 0;
        let mut maximum_combo = 0.0;
        let mut tiny_total = 0;
        let mut fruit = 0;
        for o in &prepared.objects {
            match o.object_type {
                ObjType::Fruit | ObjType::Droplet => {
                    combo += 1;
                    maximum_combo += if o.object_type == ObjType::Fruit {
                        300.0
                    } else {
                        100.0
                    } * weight(combo);
                    fruit += u32::from(o.object_type == ObjType::Fruit);
                }
                ObjType::TinyDroplet => tiny_total += 1,
                ObjType::Banana => (),
            }
        }
        Self {
            state: GameplayState {
                accuracy: Some(1.0),
                ..Default::default()
            },
            maximum_combo,
            combo: 0.0,
            tiny_total,
            tiny_hit: 0,
            judged: 0,
            hit: 0,
            tiny_portion: if tiny_total + fruit == 0 {
                0.0
            } else {
                400_000.0 * f64::from(tiny_total) / f64::from(tiny_total + fruit)
            },
            bonus: 0.0,
        }
    }
    pub fn apply(&mut self, event: &GameplayEvent, prepared: &PreparedCatch) -> bool {
        let previous = self.state.combo;
        let hit = event.result.is_hit();
        *self
            .state
            .statistics
            .entry(event.result.name().into())
            .or_default() += 1;
        match event.maximum {
            HitResult::Great | HitResult::LargeTickHit => {
                if hit {
                    self.state.combo += 1;
                    self.combo += if event.maximum == HitResult::Great {
                        300.0
                    } else {
                        100.0
                    } * weight(self.state.combo);
                } else {
                    self.state.combo = 0;
                }
            }
            HitResult::SmallTickHit => self.tiny_hit += u32::from(hit),
            HitResult::LargeBonus if hit => self.bonus += 200.0,
            _ => (),
        }
        if event.maximum != HitResult::LargeBonus {
            self.judged += 1;
            self.hit += u32::from(hit);
        }
        let change = match event.result {
            HitResult::Great => 0.03,
            HitResult::LargeTickHit => 0.015,
            HitResult::SmallTickHit => 0.0015,
            HitResult::LargeBonus => 0.0025,
            HitResult::Miss | HitResult::LargeTickMiss => {
                crate::render::cpu::modes::catch::objects::difficulty_range(
                    prepared.hp,
                    -0.03,
                    -0.125,
                    -0.2,
                )
            }
            _ => 0.0,
        };
        self.state.health = (self.state.health + change).clamp(0.0, 1.0);
        self.state.max_combo = self.state.max_combo.max(self.state.combo);
        let accuracy = if self.judged == 0 {
            1.0
        } else {
            f64::from(self.hit) / f64::from(self.judged)
        };
        self.state.accuracy = Some(accuracy);
        let combo_progress = if self.maximum_combo == 0.0 {
            1.0
        } else {
            self.combo / self.maximum_combo
        };
        let tiny_progress = if self.tiny_total == 0 {
            0.0
        } else {
            f64::from(self.tiny_hit) / f64::from(self.tiny_total)
        };
        self.state.score = (((1_000_000.0 - self.tiny_portion) * combo_progress
            + self.tiny_portion * tiny_progress
            + self.bonus)
            .round_ties_even()
            * prepared.score_multiplier)
            .round_ties_even() as u64;
        let rank = if accuracy == 1.0 {
            "X"
        } else if accuracy >= 0.98 {
            "S"
        } else if accuracy >= 0.94 {
            "A"
        } else if accuracy >= 0.9 {
            "B"
        } else if accuracy >= 0.85 {
            "C"
        } else {
            "D"
        };
        self.state.rank = Some(
            if prepared.mods.settings.hidden || prepared.mods.settings.flashlight {
                match rank {
                    "X" => "XH",
                    "S" => "SH",
                    _ => rank,
                }
            } else {
                rank
            }
            .into(),
        );
        !hit && previous > 0 && matches!(event.maximum, HitResult::Great | HitResult::LargeTickHit)
    }
}
