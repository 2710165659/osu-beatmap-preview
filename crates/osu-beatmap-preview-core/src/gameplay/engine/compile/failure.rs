//! 对照 ReplayPlayer、ModFailCondition 与 ModEasyWithExtraLives，统一回放失败门槛。
use crate::{
    gameplay::{
        events::{GameplayEvent, HitResult},
        state::GameplayState,
        timeline::GameplayTimeline,
    },
    replay::mods::ReplayMods,
};

pub(super) struct ReplayFailure<'a> {
    last_input: f64,
    mode: u8,
    mods: &'a ReplayMods,
    retries: u64,
    pub time: Option<f64>,
}

impl<'a> ReplayFailure<'a> {
    pub fn new(last_input: f64, mode: u8, mods: &'a ReplayMods) -> Self {
        Self {
            last_input,
            mode,
            mods,
            retries: mods.easy_retries,
            time: None,
        }
    }

    pub fn apply(
        &mut self,
        event: &GameplayEvent,
        state: &mut GameplayState,
        last_judgement: bool,
    ) -> bool {
        // ReplayPlayer 先检查帧是否耗尽，再调用各 Mod 的 PerformFail；不能提前消耗 EZ 生命。
        if event.time < self.last_input || self.mods.no_fail {
            return false;
        }
        let body_break =
            self.mode == 3 && event.maximum == HitResult::IgnoreHit && !event.result.is_hit();
        let breaks_combo =
            matches!(event.result, HitResult::Miss | HitResult::LargeTickMiss) || body_break;
        let relevant = !matches!(
            event.maximum,
            HitResult::IgnoreHit | HitResult::SmallBonus | HitResult::LargeBonus
        ) || body_break;
        let imperfect = if self.mode == 3
            && event.maximum == HitResult::Perfect
            && !self.mods.require_perfect_hits
        {
            !matches!(event.result, HitResult::Perfect | HitResult::Great)
        } else {
            event.result != event.maximum
        };
        let default_failure = if self.mode == 1 {
            last_judgement && state.health < 0.5
        } else {
            state.health <= 0.0
                && !matches!(event.maximum, HitResult::SmallBonus | HitResult::LargeBonus)
                && event.result != HitResult::IgnoreHit
                && !(self.mode == 2 && event.result == HitResult::SmallTickMiss)
        };
        if !(default_failure
            || (self.mods.sudden_death && breaks_combo)
            || (self.mods.perfect && relevant && imperfect))
        {
            return false;
        }
        if self.mode != 1 && self.mods.settings.easy && self.retries > 0 {
            self.retries -= 1;
            state.health = 1.0;
            return false;
        }
        self.time = Some(event.time);
        true
    }
}

/// 判定可以预先完整编译，但失败后的未来对象状态不能泄漏到画面或连续音效中。
pub(super) fn truncate(timeline: &mut GameplayTimeline) {
    let Some(time) = timeline.failure_time else {
        return;
    };
    for object in &mut timeline.objects {
        object.head = object.head.filter(|p| p.0 <= time);
        object.end = object.end.filter(|p| p.0 <= time);
        for node in &mut object.nodes {
            *node = node.filter(|p| p.0 <= time);
        }
        object.tracking.retain(|p| p.0 <= time);
        object.spinning.retain(|p| p.0 <= time);
        object.rotation.retain(|p| p.0 <= time);
        if object.tracking_at(time) {
            object.tracking.push((time, false));
        }
        if object.spinning_at(time) {
            object.spinning.push((time, false));
        }
    }
    timeline.taiko_inputs.retain(|p| p.time <= time);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::events::EventKind;

    fn judgement(time: f64, result: HitResult, maximum: HitResult) -> GameplayEvent {
        GameplayEvent {
            id: 0,
            time,
            object_id: 0,
            node_id: None,
            result,
            maximum,
            kind: EventKind::Judgement,
            error: None,
            position: None,
            sample_time: time,
        }
    }

    #[test]
    fn recorded_frames_gate_fail_mods_and_easy_lives_in_all_rulesets() {
        for mode in 0..=3 {
            let mut mods = ReplayMods::default();
            mods.sudden_death = true;
            mods.settings.easy = true;
            let mut fail = ReplayFailure::new(1000.0, mode, &mods);
            let mut state = GameplayState::default();
            assert!(!fail.apply(
                &judgement(999.0, HitResult::Miss, HitResult::Great),
                &mut state,
                false
            ));
            assert_eq!(fail.retries, 2);
            for attempt in 0..=2 {
                state.health = 0.0;
                let failed = fail.apply(
                    &judgement(1000.0, HitResult::Miss, HitResult::Great),
                    &mut state,
                    false,
                );
                assert_eq!(failed, mode == 1 || attempt == 2);
                if failed {
                    break;
                }
                assert_eq!(state.health, 1.0);
            }
            assert_eq!(fail.time, Some(1000.0));
        }
    }

    #[test]
    fn no_fail_and_taiko_completion_use_game_failure_overrides() {
        for mode in 0..=3 {
            let mut mods = ReplayMods::default();
            mods.no_fail = true;
            let mut fail = ReplayFailure::new(0.0, mode, &mods);
            let mut state = GameplayState {
                health: 0.0,
                ..Default::default()
            };
            assert!(!fail.apply(
                &judgement(1000.0, HitResult::Miss, HitResult::Great),
                &mut state,
                true
            ));
        }
        let mods = ReplayMods::default();
        let mut fail = ReplayFailure::new(0.0, 1, &mods);
        let mut state = GameplayState {
            health: 0.0,
            ..Default::default()
        };
        let event = judgement(1000.0, HitResult::Miss, HitResult::Great);
        assert!(!fail.apply(&event, &mut state, false));
        state.health = 0.5;
        assert!(!fail.apply(&event, &mut state, true));
        state.health = 0.49;
        assert!(fail.apply(&event, &mut state, true));
    }

    #[test]
    fn perfect_uses_mania_setting_and_combo_breaks_even_without_previous_combo() {
        for (mode, require_perfect_hits, result, maximum, expected) in [
            (3, false, HitResult::Great, HitResult::Perfect, false),
            (3, true, HitResult::Great, HitResult::Perfect, true),
            (3, true, HitResult::Perfect, HitResult::Perfect, false),
            (3, false, HitResult::Good, HitResult::Perfect, true),
            (3, false, HitResult::IgnoreMiss, HitResult::IgnoreHit, true),
            (0, false, HitResult::Ok, HitResult::Great, true),
            (
                2,
                false,
                HitResult::SmallTickMiss,
                HitResult::SmallTickHit,
                true,
            ),
            (
                1,
                false,
                HitResult::IgnoreMiss,
                HitResult::LargeBonus,
                false,
            ),
        ] {
            let mods = ReplayMods {
                perfect: true,
                require_perfect_hits,
                ..Default::default()
            };
            let mut fail = ReplayFailure::new(0.0, mode, &mods);
            assert_eq!(
                fail.apply(
                    &judgement(1000.0, result, maximum),
                    &mut GameplayState::default(),
                    false
                ),
                expected
            );
        }
        let mods = ReplayMods {
            sudden_death: true,
            ..Default::default()
        };
        let mut fail = ReplayFailure::new(0.0, 3, &mods);
        assert!(fail.apply(
            &judgement(1000.0, HitResult::IgnoreMiss, HitResult::IgnoreHit),
            &mut GameplayState::default(),
            false
        ));
        let mut fail = ReplayFailure::new(0.0, 2, &mods);
        let mut state = GameplayState {
            health: 0.0,
            ..Default::default()
        };
        assert!(!fail.apply(
            &judgement(1000.0, HitResult::SmallTickMiss, HitResult::SmallTickHit),
            &mut state,
            false
        ));
        assert!(!fail.apply(
            &judgement(1000.0, HitResult::IgnoreMiss, HitResult::LargeBonus),
            &mut state,
            false
        ));
        assert!(fail.apply(
            &judgement(1000.0, HitResult::LargeTickMiss, HitResult::LargeTickHit),
            &mut state,
            false
        ));
    }
}
