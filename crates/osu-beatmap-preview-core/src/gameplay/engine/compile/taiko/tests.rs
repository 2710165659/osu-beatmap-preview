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
    let map = crate::parse_beatmap_bytes(format!("osu file format v14\n[General]\nMode:1\n[Difficulty]\nOverallDifficulty:5\nHPDrainRate:5\nSliderMultiplier:1\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n{objects}\n").as_bytes()).unwrap();
    let document = Arc::new(document(frames));
    let prepared = crate::replay::preparation::taiko::prepare_replay(&map, &document).unwrap();
    compile_taiko(&prepared, document).unwrap()
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
