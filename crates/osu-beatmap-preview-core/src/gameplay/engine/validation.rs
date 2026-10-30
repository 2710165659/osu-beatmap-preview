//! 完整回放核对原成绩；差异只作诊断，不阻止播放，也不覆盖重算结果。
use crate::gameplay::{events::HitResult, state::GameplayState};
use crate::replay::ReplayDocument;
use crate::Result;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
struct Difference<T> {
    recorded: T,
    calculated: T,
}

pub(super) fn validate(
    document: &ReplayDocument,
    state: &GameplayState,
    maximums: &BTreeMap<String, u32>,
) -> Result<Option<serde_json::Value>> {
    let metadata = document.metadata.as_ref();
    let normalized = |values: &BTreeMap<String, u32>| {
        values
            .iter()
            .filter(|(_, count)| **count != 0)
            .map(|(name, count)| (name.clone(), *count))
            .collect::<BTreeMap<_, _>>()
    };
    let mut differences = serde_json::Map::new();
    macro_rules! compare {
        ($name:literal, $recorded:expr, $calculated:expr) => {
            if $recorded != $calculated {
                differences.insert(
                    $name.into(),
                    serde_json::to_value(Difference {
                        recorded: $recorded,
                        calculated: $calculated,
                    })
                    .expect("成绩字段可序列化"),
                );
            }
        };
    }
    if let Some(metadata) = metadata {
        if !metadata.statistics.is_empty() {
            compare!(
                "statistics",
                normalized(&metadata.statistics),
                normalized(&state.statistics)
            );
        }
        if !metadata.maximum_statistics.is_empty() {
            compare!(
                "maximum_statistics",
                normalized(&metadata.maximum_statistics),
                normalized(maximums)
            );
        }
        if !metadata.rank.is_empty() {
            compare!(
                "rank",
                metadata.rank.clone(),
                state.rank.clone().unwrap_or_default()
            );
        }
        let recorded_accuracy = accuracy(
            &metadata.statistics,
            &metadata.maximum_statistics,
            document.header.ruleset_id,
        );
        if recorded_accuracy
            .zip(state.accuracy)
            .is_some_and(|(a, b)| (a - b).abs() > 1e-12)
        {
            compare!("accuracy", recorded_accuracy, state.accuracy);
        }
    } else {
        // stable 的头部没有 tick 细分统计，也使用历史总分；只比较可对应的主判定。
        let (recorded, calculated) = if document.header.ruleset_id == 2 {
            // stable Catch 的 Miss 合并水果和大水滴，Katu 记录小水滴漏接。
            let recorded = [
                ("great", 0),
                ("large_tick_hit", 1),
                ("small_tick_hit", 2),
                ("small_tick_miss", 4),
                ("miss", 5),
            ]
            .map(|(name, i)| (name.to_owned(), u32::from(document.header.counts[i])))
            .into_iter()
            .collect();
            let mut calculated: BTreeMap<_, _> = [
                "great",
                "large_tick_hit",
                "small_tick_hit",
                "small_tick_miss",
                "miss",
            ]
            .into_iter()
            .map(|name| (name.to_owned(), *state.statistics.get(name).unwrap_or(&0)))
            .collect();
            *calculated.get_mut("miss").unwrap() += state
                .statistics
                .get("large_tick_miss")
                .copied()
                .unwrap_or(0);
            (recorded, calculated)
        } else if document.header.ruleset_id == 3 {
            let names = ["great", "ok", "meh", "perfect", "good", "miss"];
            let recorded = names
                .into_iter()
                .enumerate()
                .map(|(i, name)| (name.to_owned(), u32::from(document.header.counts[i])))
                .collect();
            let calculated = names
                .into_iter()
                .map(|name| (name.to_owned(), *state.statistics.get(name).unwrap_or(&0)))
                .collect();
            (recorded, calculated)
        } else {
            let names = ["great", "ok", "meh", "miss"];
            let indexes = [0, 1, 2, 5];
            let recorded = names
                .into_iter()
                .zip(indexes)
                .map(|(name, i)| (name.to_owned(), u32::from(document.header.counts[i])))
                .collect();
            let calculated = names
                .into_iter()
                .map(|name| (name.to_owned(), *state.statistics.get(name).unwrap_or(&0)))
                .collect();
            (recorded, calculated)
        };
        compare!("statistics", normalized(&recorded), normalized(&calculated));
    }
    compare!("score", u64::from(document.header.total_score), state.score);
    compare!(
        "max_combo",
        u32::from(document.header.max_combo),
        state.max_combo
    );
    if differences.is_empty() {
        Ok(None)
    } else {
        // 官方也会将历史成绩迁移到当前标准化计分；重播和录制结果无需逐项相等。
        Ok(Some(serde_json::Value::Object(differences)))
    }
}

fn accuracy(
    statistics: &BTreeMap<String, u32>,
    maximums: &BTreeMap<String, u32>,
    ruleset: u8,
) -> Option<f64> {
    let value = |map: &BTreeMap<String, u32>| {
        [
            HitResult::Perfect,
            HitResult::Great,
            HitResult::Good,
            HitResult::Ok,
            HitResult::Meh,
            HitResult::LargeTickHit,
            HitResult::SmallTickHit,
            HitResult::SliderTailHit,
        ]
        .into_iter()
        .map(|result| {
            u64::from(if ruleset == 2 { 1 } else { result.value() })
                * u64::from(*map.get(result.name()).unwrap_or(&0))
        })
        .sum::<u64>()
    };
    let maximum = value(maximums);
    (maximum != 0).then(|| value(statistics) as f64 / maximum as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accuracy_excludes_ignore_results_and_spinner_bonus() {
        let statistics = BTreeMap::from([
            ("great".into(), 1),
            ("slider_tail_hit".into(), 1),
            ("small_bonus".into(), 100),
        ]);
        let maximum = BTreeMap::from([("great".into(), 2), ("slider_tail_hit".into(), 1)]);
        assert_eq!(accuracy(&statistics, &maximum, 0), Some(0.6));
    }

    #[test]
    fn allows_one_tail_difference_without_overwriting_the_replayed_score() {
        let (document, state, maximums) = fixture();
        let difference = validate(&document, &state, &maximums).unwrap().unwrap();
        assert_eq!(difference["score"]["recorded"], 988_187);
        assert_eq!(difference["score"]["calculated"], 991_262);
        assert_eq!(state.score, 991_262);
        let mut boundary = state.clone();
        boundary.score = u64::from(document.header.total_score) + 5_000;
        assert!(validate(&document, &boundary, &maximums).unwrap().is_some());
        let mut reverse = document.clone();
        reverse.header.total_score = state.score as u32;
        reverse.header.max_combo = state.max_combo as u16;
        reverse.metadata.as_mut().unwrap().statistics = state.statistics.clone();
        let mut recorded = state.clone();
        recorded.score = u64::from(document.header.total_score);
        recorded.max_combo = u32::from(document.header.max_combo);
        recorded.statistics = document.metadata.as_ref().unwrap().statistics.clone();
        recorded.accuracy = accuracy(&recorded.statistics, &maximums, 0);
        assert!(validate(&reverse, &recorded, &maximums).unwrap().is_some());
        let mut document = document;
        document.header.total_score = state.score as u32;
        document.header.max_combo = state.max_combo as u16;
        document.metadata.as_mut().unwrap().statistics = state.statistics.clone();
        assert_eq!(validate(&document, &state, &maximums).unwrap(), None);
    }

    #[test]
    fn records_larger_differences_without_blocking_playback() {
        let (document, state, maximums) = fixture();
        for change in 0..6 {
            let mut state = state.clone();
            match change {
                0 => state.score = u64::from(document.header.total_score) + 5_001,
                1 => state.max_combo = u32::from(document.header.max_combo) + 2,
                2 => state.rank = Some("A".into()),
                3 => state.accuracy = Some(0.99),
                4 => *state.statistics.get_mut("great").unwrap() -= 1,
                _ => *state.statistics.get_mut("slider_tail_hit").unwrap() += 1,
            }
            assert!(validate(&document, &state, &maximums).unwrap().is_some());
        }
        let mut maximums = maximums;
        *maximums.get_mut("slider_tail_hit").unwrap() += 1;
        assert!(validate(&document, &state, &maximums).unwrap().is_some());
    }

    fn fixture() -> (ReplayDocument, GameplayState, BTreeMap<String, u32>) {
        use crate::replay::{ReplayHeader, ReplayMetadata};
        let statistics = BTreeMap::from([
            ("great".into(), 500),
            ("ok".into(), 2),
            ("slider_tail_hit".into(), 102),
            ("ignore_miss".into(), 3),
            ("large_tick_hit".into(), 206),
        ]);
        let maximums = BTreeMap::from([
            ("great".into(), 502),
            ("slider_tail_hit".into(), 105),
            ("large_tick_hit".into(), 206),
        ]);
        let mut state = GameplayState {
            score: 991_262,
            max_combo: 729,
            statistics: statistics.clone(),
            rank: Some("S".into()),
            ..Default::default()
        };
        state.statistics.insert("slider_tail_hit".into(), 103);
        state.statistics.insert("ignore_miss".into(), 2);
        state.accuracy = accuracy(&state.statistics, &maximums, 0);
        let document = ReplayDocument {
            header: ReplayHeader {
                ruleset_id: 0,
                version: 30_000_016,
                beatmap_md5: String::new(),
                player: String::new(),
                replay_hash: String::new(),
                counts: [0; 6],
                total_score: 988_187,
                max_combo: 728,
                perfect: false,
                mods_bits: 0,
                life_graph: String::new(),
                timestamp_ticks: 0,
                online_id: 0,
                target_practice_accuracy: None,
            },
            metadata: Some(ReplayMetadata {
                client_version: "2026.119.0-lazer-windows".into(),
                rank: "S".into(),
                mods: vec![],
                statistics,
                maximum_statistics: maximums.clone(),
                pauses: vec![],
                extra: Default::default(),
            }),
            frames: vec![],
            seed: None,
            diagnostics: vec![],
        };
        (document, state, maximums)
    }
}
