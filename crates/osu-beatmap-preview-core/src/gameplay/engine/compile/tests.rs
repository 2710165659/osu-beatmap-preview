#![cfg(test)]
//! 合成规则测试不访问网络；完整官方样本由环境变量显式启用。
use super::*;
use crate::replay::{ReplayFrame, ReplayHeader, ReplayMetadata};

#[test]
fn same_time_clicks_use_original_positions_and_order() {
    let beatmap = map("100,100,1000,1,0\n400,100,1000,1,0");
    let prepared = crate::preparation::standard::prepare(&beatmap).unwrap();
    let document = document(&[
        (-10, [0.0; 2], 0),
        (1000, [100.0; 2], 1),
        (1000, [100.0; 2], 0),
        (1000, [400.0, 100.0], 1),
        (1001, [400.0, 100.0], 0),
    ]);
    let timeline = compile_standard(&prepared, Arc::new(document)).unwrap();
    assert_eq!(timeline.events[0].position, Some([100.0; 2]));
    assert_eq!(timeline.events[1].position, Some([400.0, 100.0]));
    assert_eq!(timeline.state_at(1000.0).max_combo, 2);
    assert_eq!(timeline.state_at(f64::INFINITY).score, 1_000_000);
    // 查询顺序不推进规则，模拟音频预读、低 FPS 与倒序并行采样。
    let before = timeline.state_at(999.0).clone();
    for time in [2000.0, 1000.0, 1016.0, 999.0, 1033.0] {
        timeline.state_at(time);
    }
    assert_eq!(timeline.state_at(999.0), &before);
}

#[test]
fn result_mismatch_is_diagnostic_without_overwriting_score_or_blocking_playback() {
    let prepared = crate::preparation::standard::prepare(&map("100,100,1000,1,0")).unwrap();
    let mut replay = document(&[(0, [0.0; 2], 0), (1000, [100.0; 2], 1)]);
    replay.header.total_score = 42;
    let timeline = compile_standard(&prepared, Arc::new(replay)).unwrap();
    assert_eq!(timeline.state_at(f64::INFINITY).score, 1_000_000);
    let difference = timeline.recorded_differences.unwrap();
    assert_eq!(difference["score"]["recorded"], 42);
    assert_eq!(difference["score"]["calculated"], 1_000_000);
}

#[test]
fn all_client_and_score_versions_compile_without_an_allowlist() {
    let prepared = crate::preparation::standard::prepare(&map("100,100,1000,1,0")).unwrap();
    for version in [
        20070916, 20200101, 30_000_000, 30_000_001, 30_000_016, 30_000_019, 40_000_000,
    ] {
        let mut replay = document(&[(0, [0.0; 2], 0), (1000, [100.0; 2], 1)]);
        replay.header.version = version;
        if version < 30_000_001 {
            replay.metadata = None;
        }
        if let Some(metadata) = &mut replay.metadata {
            metadata.client_version = "2026.821.0-tachyon-windows".into();
            metadata
                .pauses
                .push(serde_json::json!({"time":500,"duration":3000}));
        }
        let timeline = compile_standard(&prepared, Arc::new(replay)).unwrap();
        assert_eq!(
            timeline.state_at(f64::INFINITY).score,
            if version < 30_000_000 {
                985_000
            } else {
                1_000_000
            }
        );
        assert_eq!(timeline.state_at(1000.0).statistics["great"], 1);
    }
}

#[test]
fn stable_and_explicit_cl_share_slider_judgements_and_current_scoring() {
    let prepared =
        crate::preparation::standard::prepare(&map("100,100,1000,2,0,L|200:100,1,100")).unwrap();
    for tail_hit in [true, false] {
        let mut legacy = document(&[
            (0, [100.0; 2], 0),
            (1100, [100.0; 2], 1),
            (1200, [140.0, 100.0], u32::from(tail_hit)),
            (1500, [200.0, 100.0], u32::from(tail_hit)),
            (1600, [200.0, 100.0], 0),
        ]);
        legacy.header.version = 20200101;
        legacy.metadata = None;
        let mut modern = legacy.clone();
        modern.header.version = 30_000_019;
        modern.metadata = document(&[]).metadata;
        modern
            .metadata
            .as_mut()
            .unwrap()
            .mods
            .push(crate::replay::model::ReplayMod {
                acronym: "CL".into(),
                settings: Default::default(),
                extra: Default::default(),
            });
        let stable = compile_standard(&prepared, Arc::new(legacy)).unwrap();
        let explicit = compile_standard(&prepared, Arc::new(modern)).unwrap();
        let state = stable.state_at(f64::INFINITY);
        assert_eq!(state, explicit.state_at(f64::INFINITY));
        assert_eq!(stable.objects[0].head.unwrap().1, HitResult::LargeTickHit);
        assert_eq!(
            stable.objects[0].end.unwrap().1,
            if tail_hit {
                HitResult::Great
            } else {
                HitResult::Ok
            }
        );
        assert_eq!(state.max_combo, 2);
        if tail_hit {
            assert_eq!(state.score, 985_000);
        } else {
            assert_eq!(state.statistics["small_tick_miss"], 1);
        }
    }
}

#[test]
fn classic_note_lock_blocks_later_circle_until_previous_object_is_judged() {
    let prepared =
        crate::preparation::standard::prepare(&map("100,100,1000,1,0\n400,100,1050,1,0")).unwrap();
    let replay = document(&[
        (0, [0.0; 2], 0),
        (1000, [400.0, 100.0], 1),
        (1300, [400.0, 100.0], 0),
    ]);
    let normal = compile_standard(&prepared, Arc::new(replay.clone())).unwrap();
    assert!(normal.objects[1].head.unwrap().1.is_hit());
    let mut classic = replay.clone();
    classic.header.version = 20200101;
    classic.metadata = None;
    let classic = compile_standard(&prepared, Arc::new(classic)).unwrap();
    assert_eq!(classic.objects[1].head.unwrap().1, HitResult::Miss);
    let mut unlocked = replay;
    unlocked
        .metadata
        .as_mut()
        .unwrap()
        .mods
        .push(crate::replay::model::ReplayMod {
            acronym: "CL".into(),
            settings: std::collections::BTreeMap::from([(
                "classic_note_lock".into(),
                false.into(),
            )]),
            extra: Default::default(),
        });
    let unlocked = compile_standard(&prepared, Arc::new(unlocked)).unwrap();
    assert_eq!(unlocked.objects[1].head, normal.objects[1].head);
}

#[test]
fn cl_can_keep_modern_slider_accuracy_through_its_recorded_settings() {
    let prepared =
        crate::preparation::standard::prepare(&map("100,100,1000,2,0,L|200:100,1,100")).unwrap();
    let mut replay = document(&[
        (0, [100.0; 2], 0),
        (1100, [100.0; 2], 1),
        (1600, [200.0, 100.0], 1),
    ]);
    replay
        .metadata
        .as_mut()
        .unwrap()
        .mods
        .push(crate::replay::model::ReplayMod {
            acronym: "CL".into(),
            settings: std::collections::BTreeMap::from([(
                "no_slider_head_accuracy".into(),
                false.into(),
            )]),
            extra: Default::default(),
        });
    let timeline = compile_standard(&prepared, Arc::new(replay)).unwrap();
    assert_eq!(timeline.objects[0].head.unwrap().1, HitResult::Meh);
    assert_eq!(timeline.objects[0].end.unwrap().1, HitResult::IgnoreHit);
    assert!(timeline
        .state_at(f64::INFINITY)
        .statistics
        .contains_key("slider_tail_hit"));
}

#[test]
fn failure_freezes_state_clears_keys_and_settles_same_time_input() {
    let prepared =
        crate::preparation::standard::prepare(&map("100,100,1000,1,0\n400,100,2000,1,0")).unwrap();
    let replay = document(&[(0, [0.0; 2], 0), (1000, [100.0; 2], 1)]);
    let timeline = compile_standard(&prepared, Arc::new(replay)).unwrap();
    let failed = timeline.state_at(1000.0);
    assert_eq!(failed.status, ReplayStatus::Failed);
    assert_eq!(failed.combo, 1);
    assert_eq!(failed, timeline.state_at(9999.0));
    assert_eq!(timeline.input_at(9999.0).unwrap().keys, 0);
    assert_eq!(
        timeline
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Fail)
            .count(),
        1
    );
    assert!(timeline.events.iter().all(|e| e.time <= 1000.0));
    assert_eq!(timeline.state_at(999.0).status, ReplayStatus::Playing);
}

#[test]
fn full_replay_gets_grace_settlement_without_false_failure() {
    let prepared = crate::preparation::standard::prepare(&map("100,100,1000,1,0")).unwrap();
    let replay = document(&[(0, [0.0; 2], 0), (1000, [0.0; 2], 0)]);
    let timeline = compile_timeline(&prepared, Arc::new(replay)).unwrap();
    assert_eq!(timeline.failure_time, None);
    assert!(timeline.settlement_time > 1000.0);
    assert_eq!(timeline.state_at(f64::INFINITY).statistics["miss"], 1);
}

#[test]
fn compilation_limit_is_checked_before_large_step_allocation() {
    let prepared = crate::preparation::standard::prepare(&map("256,192,0,8,0,3000000")).unwrap();
    let replay = document(&[(0, [0.0; 2], 0), (3000000, [0.0; 2], 0)]);
    assert!(compile_timeline(&prepared, Arc::new(replay))
        .err()
        .unwrap()
        .to_string()
        .contains("ReplayCompilationLimit"));
}

#[test]
fn late_head_restores_passed_nodes_and_short_slider_settles_after_head_timeout() {
    let prepared =
        crate::preparation::standard::prepare(&map("100,100,1000,2,0,L|110:100,2,10")).unwrap();
    assert_eq!(prepared.objects[0].end, 1100.0);
    let replay = document(&[
        (0, [0.0; 2], 0),
        (1120, [100.0; 2], 1),
        (1300, [100.0; 2], 0),
    ]);
    let timeline = compile_timeline(&prepared, Arc::new(replay)).unwrap();
    let state = &timeline.objects[0];
    assert_eq!(state.head, Some((1120.0, HitResult::Meh)));
    assert!(state
        .nodes
        .iter()
        .flatten()
        .all(|node| node.0 == 1120.0 && node.1.is_hit()));
    assert_eq!(state.end.unwrap().1, HitResult::IgnoreHit);

    let replay = document(&[(0, [0.0; 2], 0), (1300, [0.0; 2], 0)]);
    let timeline = compile_timeline(&prepared, Arc::new(replay)).unwrap();
    let state = &timeline.objects[0];
    assert_eq!(state.head.unwrap().1, HitResult::Miss);
    assert_eq!(state.end.unwrap().1, HitResult::IgnoreMiss);
    assert!(state.nodes.iter().flatten().all(|node| !node.1.is_hit()));
    assert_eq!(
        state.nodes.iter().flatten().count(),
        prepared.objects[0].nodes.len()
    );
}

#[test]
fn slider_only_accepts_other_key_after_its_release_was_observed() {
    let prepared =
        crate::preparation::standard::prepare(&map("100,100,1000,2,0,L|140:100,1,40")).unwrap();
    let replay = document(&[
        (0, [0.0; 2], 0),
        (1000, [500.0; 2], 2),
        (1010, [100.0; 2], 3),
        (1011, [100.0; 2], 2),
        (1012, [100.0; 2], 0),
        (1013, [100.0; 2], 2),
        (1300, [140.0, 100.0], 0),
    ]);
    let timeline = compile_timeline(&prepared, Arc::new(replay)).unwrap();
    let state = &timeline.objects[0];
    assert!(state.tracking_at(1010.0));
    assert!(!state.tracking_at(1011.0));
    assert!(!state.tracking_at(1013.0));
    assert!(state.tracking_at(1014.0));
}

#[test]
fn spinner_sound_requires_motion_stops_after_damping_and_clears_at_failure() {
    let prepared = crate::preparation::standard::prepare(&map("256,192,1000,8,0,3000")).unwrap();
    let mut frames = vec![(0, [356.0, 192.0], 0), (1000, [356.0, 192.0], 1)];
    for step in 1..=60 {
        let angle = (step as f32 * 12.0).to_radians();
        frames.push((
            1000 + step * 10,
            [256.0 + 100.0 * angle.cos(), 192.0 + 100.0 * angle.sin()],
            1,
        ));
    }
    frames.push((3000, [356.0, 192.0], 1));
    let timeline = compile_timeline(&prepared, Arc::new(document(&frames))).unwrap();
    let state = &timeline.objects[0];
    assert!(!state.spinning_at(1000.0));
    assert!(state.spinning_at(1300.0));
    assert!(!state.spinning_at(2600.0));
    assert!(!state.spinning_at(3000.0));
    frames.retain(|frame| frame.0 <= 1300);
    let failed = compile_timeline(&prepared, Arc::new(document(&frames))).unwrap();
    assert_eq!(failed.failure_time, Some(1300.0));
    assert!(failed.objects[0].spinning_at(1299.0));
    assert!(!failed.objects[0].spinning_at(1300.0));
}

#[test]
#[ignore = "需要 OSR_SAMPLE_DIR；与官方 Headless ReplayPlayer 的实际重播结果核对"]
fn official_standard_sample_matches_official_replay_results() {
    let root = std::path::PathBuf::from(std::env::var("OSR_SAMPLE_DIR").unwrap());
    let beatmap =
        crate::parse_beatmap_bytes(&std::fs::read(root.join("4903207.osu")).unwrap()).unwrap();
    let document = Arc::new(
        crate::replay::parse_osr(
            &std::fs::read(root.join("xuan_yuan-osu-6086051920.osr")).unwrap(),
        )
        .unwrap(),
    );
    let prepared = crate::preparation::standard::prepare(&beatmap).unwrap();
    let timeline = compile_standard(&prepared, document).unwrap();
    let state = timeline.state_at(f64::INFINITY);
    // 官方 ReplayPlayer 实测同样为这些结果；不使用录制成绩覆盖显示。
    assert_eq!(state.score, 991_262);
    assert_eq!(state.max_combo, 729);
    assert_eq!(state.statistics["slider_tail_hit"], 103);
    assert_eq!(state.statistics["ignore_miss"], 2);
    assert!((state.accuracy.unwrap() - 0.997036158861885).abs() < 1e-12);
    assert_eq!(state.rank.as_deref(), Some("S"));
    assert!(timeline.recorded_differences.is_some());
    let oracle: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("oracle-results.json")).unwrap()).unwrap();
    for (kind, result_type) in [
        ("HitCircle", 0),
        ("SliderHeadCircle", 1),
        ("SliderTailCircle", 2),
        ("SliderTick", 3),
    ] {
        let official: Vec<_> = oracle["judgements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == kind)
            .collect();
        let calculated: Vec<_> = timeline
            .events
            .iter()
            .filter(|event| {
                if event.kind != EventKind::Judgement {
                    return false;
                }
                let object = &prepared.objects[event.object_id];
                match result_type {
                    0 => object.source.hit_type & (2 | 8) == 0,
                    1 => {
                        object.path.is_some()
                            && event.node_id.is_none()
                            && event.maximum == HitResult::Great
                    }
                    2 => event.maximum == HitResult::SliderTailHit,
                    _ => event.maximum == HitResult::LargeTickHit,
                }
            })
            .collect();
        assert_eq!(official.len(), calculated.len(), "{kind}");
        for (official, calculated) in official.into_iter().zip(calculated) {
            let result = format!("{:?}", calculated.result);
            assert_eq!(official["result"], result, "{kind} at {}", calculated.time);
            let start = official["start"].as_f64().unwrap();
            assert!(
                (calculated.sample_time - start).abs() < 1e-6,
                "{kind}: nominal time"
            );
            let time = start + official["offset"].as_f64().unwrap();
            // 官方 Headless 按 60Hz 帧轮询连续节点，最多晚一个帧间隔；点击保留原帧时刻。
            let tolerance = if result_type < 2 { 1e-6 } else { 17.0 };
            assert!(
                (calculated.time - time).abs() <= tolerance,
                "{kind}: calculated={} official={time}",
                calculated.time
            );
        }
    }
}

#[test]
fn hard_rock_uses_flipped_geometry_and_recorded_cursor_without_double_flip() {
    let beatmap = map("100,100,1000,1,0");
    for legacy in [false, true] {
        let mut replay = document(&[(0, [100.0, 284.0], 0), (1000, [100.0, 284.0], 1)]);
        if legacy {
            replay.header.version = 20200101;
            replay.header.mods_bits = 1 << 4;
            replay.metadata = None;
        } else {
            replay.metadata.as_mut().unwrap().mods =
                serde_json::from_value(serde_json::json!([{"acronym":"HR"}])).unwrap();
        }
        let prepared = crate::preparation::standard::prepare_replay(&beatmap, &replay).unwrap();
        assert_eq!(prepared.objects[0].position, [100.0, 284.0]);
        assert_eq!(prepared.od, 7.0);
        let timeline = compile_standard(&prepared, Arc::new(replay)).unwrap();
        assert_eq!(
            timeline.input_at(1000.0).unwrap().cursor,
            Some([100.0, 284.0])
        );
        let state = timeline.state_at(f64::INFINITY);
        assert_eq!(state.statistics["great"], 1);
        assert_eq!(state.score, if legacy { 1_073_650 } else { 1_090_000 });
    }
}

#[test]
fn replay_mod_difficulty_changes_hit_windows_and_slider_geometry_before_stacking() {
    let beatmap = map("100,100,1000,1,0");
    for (mods, great) in [
        (serde_json::json!([]), true),
        (
            serde_json::json!([{"acronym":"DA","settings":{"overall_difficulty":10}}]),
            false,
        ),
    ] {
        let mut replay = document(&[(0, [100.0; 2], 0), (1040, [100.0; 2], 1)]);
        replay.metadata.as_mut().unwrap().mods = serde_json::from_value(mods).unwrap();
        let prepared = crate::preparation::standard::prepare_replay(&beatmap, &replay).unwrap();
        let timeline = compile_standard(&prepared, Arc::new(replay)).unwrap();
        assert_eq!(
            timeline.events[0].result,
            if great {
                HitResult::Great
            } else {
                HitResult::Ok
            }
        );
    }
    let beatmap = map("100,100,1000,2,0,L|200:150,1,100");
    let mut replay = document(&[(0, [0.0; 2], 0)]);
    replay.metadata.as_mut().unwrap().mods =
        serde_json::from_value(serde_json::json!([{"acronym":"HR"}])).unwrap();
    let prepared = crate::preparation::standard::prepare_replay(&beatmap, &replay).unwrap();
    assert_eq!(prepared.objects[0].source.slider_points[0], (200, 234));
    assert!(prepared.objects[0].position_at(prepared.last_end)[1] < 284.0);
}

#[test]
fn replay_score_applies_combined_mods_without_rescaling_input_time() {
    let beatmap = map("100,100,1000,1,0");
    let mut replay = document(&[(0, [100.0, 284.0], 0), (1000, [100.0, 284.0], 1)]);
    replay.metadata.as_mut().unwrap().mods = serde_json::from_value(serde_json::json!([
        {"acronym":"HR"}, {"acronym":"HD"}, {"acronym":"FL"}, {"acronym":"DT"}, {"acronym":"CL"}
    ]))
    .unwrap();
    let prepared = crate::preparation::standard::prepare_replay(&beatmap, &replay).unwrap();
    let timeline = compile_standard(&prepared, Arc::new(replay)).unwrap();
    assert_eq!(timeline.events[0].time, 1000.0);
    assert_eq!(
        timeline.state_at(f64::INFINITY).score,
        (1_000_000.0_f64 * 1.09 * 1.04 * 1.2 * 1.23 * 0.985).round_ties_even() as u64
    );
}

#[test]
fn mirror_reflects_geometry_without_changing_recorded_cursor() {
    let beatmap = map("100,100,1000,2,0,L|200:150,1,100");
    for (reflection, expected, control) in [
        (serde_json::json!("Horizontal"), [412.0, 100.0], (312, 150)),
        (serde_json::json!("Vertical"), [100.0, 284.0], (200, 234)),
        (serde_json::json!("Both"), [412.0, 284.0], (312, 234)),
        (serde_json::json!(2), [412.0, 284.0], (312, 234)),
    ] {
        let mut replay = document(&[(0, expected, 0), (1000, expected, 1), (1700, expected, 0)]);
        replay.metadata.as_mut().unwrap().mods = serde_json::from_value(serde_json::json!([
            {"acronym":"MR", "settings":{"reflection":reflection}}
        ]))
        .unwrap();
        let prepared = crate::preparation::standard::prepare_replay(&beatmap, &replay).unwrap();
        assert_eq!(prepared.objects[0].position, expected);
        assert_eq!(prepared.objects[0].source.slider_points[0], control);
        let timeline = compile_standard(&prepared, Arc::new(replay)).unwrap();
        assert_eq!(timeline.input_at(1000.0).unwrap().cursor, Some(expected));
        assert!(timeline.objects[0].head.unwrap().1.is_hit());
    }
}

#[test]
fn alternate_and_single_tap_filter_press_edges_and_preserve_releases() {
    let beatmap = map("100,100,1000,1,0\n100,100,1200,1,0\n100,100,1400,1,0");
    for (code, hit) in [("AL", [true, false, true]), ("SG", [true, true, false])] {
        let mut replay = document(&[
            (0, [100.0; 2], 0),
            (1000, [100.0; 2], 5),
            (1050, [100.0; 2], 0),
            (1200, [100.0; 2], 1),
            (1250, [100.0; 2], 0),
            (1400, [100.0; 2], 10),
            (1500, [100.0; 2], 0),
        ]);
        replay.metadata.as_mut().unwrap().mods =
            serde_json::from_value(serde_json::json!([{"acronym":code}])).unwrap();
        let prepared = crate::preparation::standard::prepare_replay(&beatmap, &replay).unwrap();
        let timeline = compile_standard(&prepared, Arc::new(replay)).unwrap();
        for (object, hit) in timeline.objects.iter().zip(hit) {
            assert_eq!(object.head.unwrap().1.is_hit(), hit, "{code}");
        }
        assert_eq!(timeline.input_at(1250.0).unwrap().keys, 0);
    }
    let beatmap = crate::parse_beatmap_bytes(map_bytes_with_break().as_bytes()).unwrap();
    let mut replay = document(&[
        (0, [100.0; 2], 0),
        (1000, [100.0; 2], 1),
        (1050, [100.0; 2], 0),
        (3000, [100.0; 2], 1),
        (3100, [100.0; 2], 0),
    ]);
    replay.metadata.as_mut().unwrap().mods =
        serde_json::from_value(serde_json::json!([{"acronym":"AL"}])).unwrap();
    let prepared = crate::preparation::standard::prepare_replay(&beatmap, &replay).unwrap();
    let timeline = compile_standard(&prepared, Arc::new(replay)).unwrap();
    assert!(timeline.objects.iter().all(|o| o.head.unwrap().1.is_hit()));
}

#[test]
fn relax_rebuilds_legacy_inputs_but_keeps_modern_recorded_presses() {
    let beatmap = map("100,100,1000,2,0,L|200:100,1,100");
    let frames = [
        (0, [100.0; 2], 0),
        (1000, [100.0; 2], 0),
        (1500, [200.0, 100.0], 0),
        (1600, [200.0, 100.0], 0),
    ];
    for legacy in [true, false] {
        let mut replay = document(&frames);
        if legacy {
            replay.metadata = None;
            replay.header.version = 20200101;
            replay.header.mods_bits = 1 << 7;
        } else {
            replay.metadata.as_mut().unwrap().mods =
                serde_json::from_value(serde_json::json!([{"acronym":"RX"}])).unwrap();
        }
        let prepared = crate::preparation::standard::prepare_replay(&beatmap, &replay).unwrap();
        let timeline = compile_standard(&prepared, Arc::new(replay)).unwrap();
        assert_eq!(timeline.objects[0].head.unwrap().1.is_hit(), legacy);
        if legacy {
            assert!(timeline.objects[0]
                .nodes
                .iter()
                .all(|n| n.unwrap().1.is_hit()));
            assert_eq!(timeline.state_at(f64::INFINITY).score, 98_500);
            assert_ne!(timeline.input_at(1000.0).unwrap().keys, 0);
        }
    }
}

#[test]
fn spun_out_rotates_without_recorded_keys_or_cursor_motion() {
    let beatmap = map("256,192,1000,8,0,3000");
    let mut replay = document(&[(0, [0.0; 2], 0), (3100, [0.0; 2], 0)]);
    replay.metadata.as_mut().unwrap().mods =
        serde_json::from_value(serde_json::json!([{"acronym":"SO"}])).unwrap();
    let prepared = crate::preparation::standard::prepare_replay(&beatmap, &replay).unwrap();
    let timeline = compile_standard(&prepared, Arc::new(replay)).unwrap();
    assert_eq!(timeline.objects[0].end.unwrap().1, HitResult::Great);
    let angle = timeline.objects[0].rotation.last().unwrap().2;
    assert!((angle - 5.05 * 360.0).abs() < 0.1, "{angle}");
    assert_eq!(timeline.input_at(2000.0).unwrap().cursor, Some([0.0; 2]));
}

fn map_bytes_with_break() -> String {
    "osu file format v14\n[General]\nMode:0\n[Difficulty]\nCircleSize:5\nOverallDifficulty:5\n[TimingPoints]\n0,500\n[Events]\n2,1500,2500\n[HitObjects]\n100,100,1000,1,0\n100,100,3000,1,0\n".into()
}

fn map(objects: &str) -> crate::Beatmap {
    crate::parse_beatmap_bytes(format!("osu file format v14\n[General]\nMode:0\n[Difficulty]\nCircleSize:5\nOverallDifficulty:5\nSliderMultiplier:1\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n{objects}\n").as_bytes()).unwrap()
}

fn document(frames: &[(i64, [f32; 2], u32)]) -> ReplayDocument {
    let hits = frames.iter().filter(|f| f.2 != 0).count() as u32;
    ReplayDocument {
        header: ReplayHeader {
            ruleset_id: 0,
            version: 30_000_016,
            beatmap_md5: "0".repeat(32),
            player: "test".into(),
            replay_hash: String::new(),
            counts: [hits as u16, 0, 0, 0, 0, 0],
            total_score: 1_000_000,
            max_combo: hits as u16,
            perfect: true,
            mods_bits: 0,
            life_graph: String::new(),
            timestamp_ticks: 0,
            online_id: 0,
            target_practice_accuracy: None,
        },
        metadata: Some(ReplayMetadata {
            client_version: "2026.119.0-lazer-windows".into(),
            rank: "X".into(),
            mods: vec![],
            statistics: std::collections::BTreeMap::from([("great".into(), hits)]),
            maximum_statistics: std::collections::BTreeMap::from([("great".into(), hits)]),
            pauses: vec![],
            extra: Default::default(),
        }),
        frames: frames
            .iter()
            .enumerate()
            .map(|(sequence, &(time_ms, cursor, keys))| ReplayFrame {
                time_ms,
                cursor,
                keys,
                sequence,
                delta_ms: time_ms - sequence.checked_sub(1).map_or(0, |i| frames[i].0),
            })
            .collect(),
        seed: None,
        diagnostics: vec![],
    }
}
