//! ManiaHealthProcessor 的回血校准；沿用 LegacyDrainingHealthProcessor 的迭代，实际游玩不随时间扣血。
use crate::{domain::models::BreakPeriod, model::ManiaHitObject};

pub(super) fn recovery_multiplier(
    objects: &[ManiaHitObject],
    breaks: &[BreakPeriod],
    hp: f64,
) -> f64 {
    let range = |low, mid, high| {
        crate::domain::rulesets::taiko::gameplay::difficulty_range(hp, low, mid, high)
    };
    let lowest_ever = range(0.975, 0.8, 0.3);
    let lowest_end = range(0.99, 0.9, 0.4);
    let recovery_available = range(0.04, 0.02, 0.0);
    let mut multiplier = 1.0;
    let mut drop = 0.00025;
    loop {
        let mut health = 1.0_f64;
        let mut uncapped = 1.0_f64;
        let mut last_time = objects.first().map_or(0.0, |o| o.start_time as f64);
        let mut current_break = 0;
        let mut failed = false;
        for object in objects {
            while current_break < breaks.len()
                && breaks[current_break].end_time <= object.start_time
            {
                last_time = object.start_time as f64;
                current_break += 1;
            }
            let reduction = drop * (object.start_time as f64 - last_time);
            uncapped = (uncapped - reduction).max(0.0);
            health = (health - reduction).max(0.0);
            last_time = object.end_time as f64;
            if health <= lowest_ever {
                failed = true;
                drop *= 0.96;
                break;
            }
            let reduction = drop * (object.end_time - object.start_time) as f64;
            let overkill = (reduction - health).max(0.0);
            uncapped = (uncapped - reduction).max(0.0);
            health = (health - reduction).max(0.0);
            // HoldNote 本体与 Body 的最大判定不回血，只有 HeadNote 和 TailNote 两次 Perfect。
            for _ in 0..if object.is_long_note { 2 } else { 1 } {
                let increase = multiplier * (0.0055 - hp * 0.0005);
                uncapped += increase;
                health = (health + increase).clamp(0.0, 1.0);
            }
            if overkill > 0.0 && health - overkill <= lowest_ever {
                failed = true;
                drop *= 0.96;
                break;
            }
        }
        if objects.is_empty() {
            return multiplier;
        }
        if !failed && health < lowest_end {
            failed = true;
            drop *= 0.94;
            multiplier *= 1.01;
        }
        let recovery = (uncapped - 1.0) / objects.len().max(1) as f64;
        if !failed && recovery < recovery_available {
            failed = true;
            drop *= 0.96;
            multiplier *= 1.01;
        }
        if !failed {
            return if multiplier.is_infinite() {
                1.0
            } else {
                multiplier
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_notes_and_holds_calibrate_per_object_recovery() {
        for (objects, hp, gain, target) in [
            ("64,192,1000,1,0", 5.0, 0.003_f64, 0.02_f64),
            ("64,192,1000,1,0", 0.0, 0.0055, 0.04),
            ("64,192,1000,128,0,1000:0:0:0:0:", 5.0, 0.006, 0.02),
        ] {
            let (prepared, _) =
                crate::replay::preparation::mania::tests::fixture(objects, &[(0, 0), (1300, 0)]);
            // 同刻物件无模拟掉血，校准值是让每物件回血达到阈值的最小 1.01 整数次幂。
            let expected = 1.01_f64.powf(((target / gain).ln() / 1.01_f64.ln()).ceil());
            let actual = recovery_multiplier(&prepared.objects, &[], hp);
            assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
        }
    }
}
