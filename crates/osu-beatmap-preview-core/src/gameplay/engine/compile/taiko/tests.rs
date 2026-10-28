#![cfg(test)]
//! Taiko 规则边界与不可变查询回归；谱面和输入均为本地合成数据。
use super::*;
use crate::replay::{ReplayFrame, ReplayHeader};

fn document(frames: &[(i64, u32)]) -> ReplayDocument {
    ReplayDocument {
        header: ReplayHeader {
            ruleset_id: 1,
            version: 20200101,
            beatmap_md5: String::new(),
            player: String::new(),
            replay_hash: String::new(),
            counts: [0; 6],
            total_score: 0,
            max_combo: 0,
            perfect: false,
            mods_bits: 0,
            life_graph: String::new(),
            timestamp_ticks: 0,
            online_id: 0,
            target_practice_accuracy: None,
        },
        metadata: None,
        seed: None,
        diagnostics: Vec::new(),
        frames: frames
            .iter()
            .enumerate()
            .map(|(sequence, &(time_ms, keys))| ReplayFrame {
                time_ms,
                keys,
                delta_ms: 0,
                cursor: [0.0; 2],
                sequence,
            })
            .collect(),
    }
}
fn compile(objects: &str, frames: &[(i64, u32)]) -> GameplayTimeline {
    compile_replay(objects, document(frames))
}

fn compile_replay(objects: &str, document: ReplayDocument) -> GameplayTimeline {
    let map = crate::parse_beatmap_bytes(format!("osu file format v14\n[General]\nMode:1\n[Difficulty]\nOverallDifficulty:5\nHPDrainRate:5\nSliderMultiplier:1\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n{objects}\n").as_bytes()).unwrap();
    let document = Arc::new(document);
    let prepared = crate::replay::preparation::taiko::prepare_replay(&map, &document).unwrap();
    compile_taiko(&prepared, document).unwrap()
}

#[test]
fn stable_and_explicit_classic_preserve_current_taiko_judgements_and_scoring() {
    let objects =
        "0,0,1000,1,0\n0,0,2000,1,12\n0,0,3000,1,0\n0,0,4000,2,4,L|100:0,1,100\n0,0,5000,8,0,6000";
    let mut replay = document(&[
        (0, 0),
        (1000, 1),
        (1001, 0),
        (2035, 10),
        (2036, 0),
        (3000, 2),
        (3001, 0),
        (4000, 5),
        (4001, 0),
        (4125, 2),
        (4126, 0),
        (5000, 1),
        (5001, 0),
        (5020, 2),
        (5021, 0),
        (5040, 4),
        (5041, 0),
        (5060, 8),
        (5061, 0),
        (5080, 1),
        (5081, 0),
        (5100, 2),
        (5101, 0),
        (5120, 4),
        (5121, 0),
        (5140, 8),
        (6200, 0),
    ]);
    replay.header.version = 30_000_019;
    replay.metadata = Some(
        serde_json::from_value(serde_json::json!({
            "mods": [{"acronym": "HD"}, {"acronym": "NC"}]
        }))
        .unwrap(),
    );
    let baseline = compile_replay(objects, replay.clone());
    assert_eq!(
        baseline.policy,
        ReplayPolicy::Taiko(compatibility::TaikoPolicy { classic: false })
    );
    let state = baseline.state_at(f64::INFINITY);
    assert_eq!(state.statistics["great"], 1);
    assert_eq!(state.statistics["ok"], 1);
    assert_eq!(state.statistics["miss"], 1);
    assert_eq!(state.statistics["small_bonus"], 2);
    assert_eq!(state.statistics["large_bonus"], 4);

    for version in [
        20070916, 20200101, 30_000_001, 30_000_016, 30_000_017, 30_000_019, 40_000_000,
    ] {
        let mut classic = replay.clone();
        classic.header.version = version;
        if version < 30_000_000 {
            classic.metadata = None;
            classic.header.mods_bits = (1 << 3) | (1 << 6) | (1 << 9);
        } else {
            classic
                .metadata
                .as_mut()
                .unwrap()
                .mods
                .push(serde_json::from_value(serde_json::json!({"acronym": "CL"})).unwrap());
        }
        let mods = crate::replay::mods::decode(&classic).unwrap();
        assert!(mods.settings.classic);
        assert_eq!(
            mods.settings
                .tokens
                .iter()
                .filter(|t| t.as_str() == "CL")
                .count(),
            1
        );
        let actual = compile_replay(objects, classic);
        assert_eq!(
            actual.policy,
            ReplayPolicy::Taiko(compatibility::TaikoPolicy { classic: true })
        );
        assert_eq!(
            actual.checkpoints, baseline.checkpoints,
            "version {version}"
        );
        for (actual, expected) in actual.objects.iter().zip(&baseline.objects) {
            assert_eq!(actual.head, expected.head);
            assert_eq!(actual.end, expected.end);
            assert_eq!(actual.nodes, expected.nodes);
        }
        for time in [6200.0, 2035.0, 999.0, 4500.0, 5140.0] {
            assert_eq!(actual.state_at(time), baseline.state_at(time));
        }
    }
}

#[test]
fn four_independent_buttons_and_wrong_colour() {
    for (key, sound) in [(1, 0), (2, 8), (4, 0), (8, 2)] {
        let timeline = compile(
            &format!("0,0,1000,1,{sound}"),
            &[(0, 0), (1000, key), (1200, 0)],
        );
        assert_eq!(timeline.state_at(1200.0).score, 1_000_000);
        assert_eq!(timeline.taiko_inputs[0].key, key);
    }
    let t = compile("0,0,1000,1,0", &[(0, 0), (1000, 2), (1200, 0)]);
    assert_eq!(t.state_at(1200.0).statistics["miss"], 1);
    assert_eq!(t.state_at(1200.0).rank.as_deref(), Some("D"));
}

#[test]
fn window_edges_and_note_lock_discard_same_timestamp_presses() {
    for (offset, expected) in [
        (34, HitResult::Great),
        (35, HitResult::Ok),
        (79, HitResult::Ok),
        (80, HitResult::Miss),
        (94, HitResult::Miss),
    ] {
        let t = compile("0,0,1000,1,0", &[(0, 0), (1000 + offset, 1), (1200, 0)]);
        assert_eq!(t.objects[0].head.unwrap().1, expected, "offset {offset}");
    }
    let t = compile(
        "0,0,1000,1,0\n0,0,1010,1,0",
        &[
            (0, 0),
            (1000, 1),
            (1000, 0),
            (1000, 4),
            (1010, 0),
            (1010, 1),
            (1200, 0),
        ],
    );
    assert_eq!(t.objects[1].head.unwrap().0, 1010.0);
    assert_eq!(t.taiko_inputs.len(), 3);
    let before = t.state_at(999.0).clone();
    for time in [1300.0, 1000.0, 1200.0, 998.0] {
        t.state_at(time);
        t.events_between(time, time + 50.0);
    }
    assert_eq!(t.state_at(999.0), &before);
}

#[test]
fn strong_second_hit_requires_the_first_button_still_held_and_30ms_window() {
    for (frames, bonus) in [
        (vec![(0, 0), (1000, 5), (1200, 0)], true),
        (vec![(0, 0), (1000, 1), (1030, 5), (1200, 0)], true),
        (vec![(0, 0), (1000, 1), (1031, 5), (1200, 0)], false),
        (
            vec![(0, 0), (1000, 1), (1001, 0), (1002, 5), (1200, 0)],
            false,
        ),
        (vec![(0, 0), (1000, 1), (1001, 3), (1200, 0)], false),
    ] {
        let t = compile("0,0,1000,1,4", &frames);
        assert_eq!(
            t.state_at(1200.0).score,
            1_000_000 + if bonus { 350 } else { 0 }
        );
    }
}

#[test]
fn drum_roll_and_swell_are_bonus_without_accuracy_or_combo() {
    let t = compile(
        "0,0,1000,2,4,L|100:0,1,100",
        &[
            (0, 0),
            (1000, 1),
            (1001, 0),
            (1125, 2),
            (1126, 0),
            (1250, 4),
            (1251, 0),
            (1375, 8),
            (1800, 0),
        ],
    );
    let state = t.state_at(2000.0);
    assert_eq!(state.combo, 0);
    assert_eq!(state.accuracy, Some(1.0));
    assert_eq!(state.score, 1_000_000 + 4 * 160);
    assert_eq!(state.health, 1.0);
    let t = compile(
        "0,0,1000,8,0,2000",
        &[
            (0, 0),
            (1000, 1),
            (1010, 0),
            (1020, 4),
            (1030, 0),
            (1040, 2),
            (1040, 0),
            (1040, 1),
            (1050, 0),
            (1060, 1),
            (2200, 0),
        ],
    );
    assert_eq!(
        t.objects[0]
            .nodes
            .iter()
            .flatten()
            .filter(|(_, r)| *r == HitResult::IgnoreHit)
            .count(),
        3
    );
    assert_eq!(t.state_at(2200.0).health, 1.0 - 0.65 * 0.0075);
}

#[test]
fn early_end_freezes_without_judging_future_objects_and_hp_uses_half_pass_line() {
    let t = compile(
        "0,0,1000,1,0\n0,0,2000,1,0",
        &[(0, 0), (1000, 1), (1050, 0)],
    );
    assert_eq!(t.failure_time, Some(1050.0));
    assert!(t.objects[1].head.is_none());
    assert_eq!(t.state_at(1100.0), t.state_at(5000.0));
    assert_eq!(t.state_at(5000.0).status, ReplayStatus::Failed);
    for (offset, passed) in [(0, true), (40, false)] {
        let t = compile("0,0,1000,1,0", &[(0, 0), (1000 + offset, 1), (1200, 0)]);
        assert_eq!(t.state_at(1200.0).status == ReplayStatus::Completed, passed);
    }
}
