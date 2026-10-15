#![cfg(test)]
//! 用独立 PCM 验证起音、失败截断、转盘启停和跳转相位，不访问外部样本。
use super::*;
use crate::gameplay::{state::ObjectState, timeline::GameplayTimeline};
use crate::replay::{ReplayDocument, ReplayHeader, ReplayTimeline};
use std::sync::Arc;

#[test]
fn failure_stops_player_samples_but_allows_fail_prompt_to_finish() {
    let (beatmap, prepared, mut gameplay) = fixture("100,100,1000,2,0,L|200:100,1,100");
    gameplay.failure_time = Some(1200.0);
    gameplay.objects[0].tracking = vec![(1000.0, true), (1200.0, false)];
    gameplay.events = vec![
        event(1000.0, EventKind::Judgement, HitResult::Great),
        event(1200.0, EventKind::Fail, HitResult::IgnoreMiss),
    ];
    let library = library(&["normal-hitnormal", "normal-sliderslide", "failsound"]);
    let timeline = build_timeline(&beatmap, &prepared, &gameplay, &library);
    assert_eq!(timeline.events.len(), 3);
    for sample in &timeline.events {
        match library.name_of(sample.source_id).unwrap() {
            "failsound" => assert_eq!((sample.start_ms, sample.duration_ms), (1200.0, 0.0)),
            _ => assert_eq!(sample.start_ms + sample.duration_ms, 1200.0),
        }
    }
    let mut mixer = HitsoundMixer::new(library, timeline, 1000);
    mixer.seek_replay(1199.0);
    let samples = mixer.render(3);
    assert_ne!(samples[0], 0.0);
    assert_ne!(samples[2], 0.0);
    mixer.seek_replay(1201.0);
    assert!(mixer.render(4).iter().all(|&value| value == 0.0));
}

#[test]
fn spinner_stops_and_restarts_phase_and_plays_bonus_after_the_score_cap() {
    let (beatmap, prepared, mut gameplay) = fixture("256,192,1000,8,0,2000");
    gameplay.objects[0].rotation = vec![
        (1000.0, 0.0, 0.0),
        (1010.0, 100.0, 100.0),
        (1020.0, 100.0, 100.0),
        (1030.0, 100.0, 100.0),
        (1040.0, 2880.0, 2880.0),
        (1050.0, 3240.0, 3240.0),
    ];
    gameplay.objects[0].spinning = vec![(1000.0, true), (1020.0, false), (1030.0, true)];
    let library = library(&["normal-spinnerspin", "normal-spinnerbonus-max"]);
    let timeline = build_timeline(&beatmap, &prepared, &gameplay, &library);
    let loops: Vec<_> = timeline
        .events
        .iter()
        .filter(|event| event.looping)
        .collect();
    assert_eq!(loops.len(), 4);
    assert_eq!(loops[0].frequency.phase, 0.0);
    assert!(loops[1].frequency.phase > 0.0);
    assert_eq!(loops[2].start_ms, 1030.0);
    assert_eq!(loops[2].frequency.phase, 0.0);
    let bonus: Vec<_> = timeline
        .events
        .iter()
        .filter(|event| !event.looping)
        .collect();
    assert_eq!(bonus.len(), 2);
    assert_eq!(bonus[0].start_ms, 1040.0);
    assert_eq!(bonus[1].start_ms, 1050.0);
}

#[test]
fn classic_plays_missed_tail_once_and_respects_the_recorded_tail_sample_setting() {
    let (beatmap, prepared, mut gameplay) = fixture("100,100,1000,2,0,L|200:100,1,100");
    Arc::make_mut(&mut gameplay.document).header.version = 20200101;
    gameplay.policy = crate::replay::compatibility::select(&gameplay.document).unwrap();
    let mut head = event(1000.0, EventKind::Judgement, HitResult::LargeTickHit);
    head.maximum = HitResult::LargeTickHit;
    let mut tail = event(1464.0, EventKind::Judgement, HitResult::SmallTickMiss);
    tail.node_id = Some(0);
    tail.maximum = HitResult::SmallTickHit;
    tail.sample_time = 1500.0;
    let parent = event(1500.0, EventKind::Judgement, HitResult::Ok);
    gameplay.events = vec![head, tail, parent];
    let library = library(&["normal-hitnormal"]);
    let timeline = build_timeline(&beatmap, &prepared, &gameplay, &library);
    assert_eq!(
        timeline
            .events
            .iter()
            .map(|e| e.start_ms)
            .collect::<Vec<_>>(),
        [1000.0, 1500.0]
    );
    let document = Arc::make_mut(&mut gameplay.document);
    document.header.version = 30_000_016;
    document.metadata = Some(crate::replay::ReplayMetadata {
        client_version: String::new(),
        rank: String::new(),
        statistics: Default::default(),
        maximum_statistics: Default::default(),
        pauses: vec![],
        extra: Default::default(),
        mods: vec![crate::replay::model::ReplayMod {
            acronym: "CL".into(),
            settings: std::collections::BTreeMap::from([(
                "always_play_tail_sample".into(),
                false.into(),
            )]),
            extra: Default::default(),
        }],
    });
    gameplay.policy = crate::replay::compatibility::select(&gameplay.document).unwrap();
    let timeline = build_timeline(&beatmap, &prepared, &gameplay, &library);
    assert_eq!(timeline.events.len(), 1);
    assert_eq!(timeline.events[0].start_ms, 1000.0);
}

fn event(time: f64, kind: EventKind, result: HitResult) -> crate::gameplay::events::GameplayEvent {
    crate::gameplay::events::GameplayEvent {
        id: 0,
        object_id: 0,
        node_id: None,
        time,
        sample_time: time,
        result,
        maximum: HitResult::Great,
        kind,
        error: None,
        position: None,
    }
}

fn library(names: &[&str]) -> SampleLibrary {
    let mut library = SampleLibrary::new();
    for name in names {
        library.insert(
            *name,
            SampleData::mono(vec![0.2; 2000], 1000).with_loop(2000),
        );
    }
    library
}

fn fixture(objects: &str) -> (Beatmap, PreparedStandard, GameplayTimeline) {
    let beatmap = crate::parse_beatmap_bytes(format!("osu file format v14\n[General]\nMode:0\n[Difficulty]\nCircleSize:5\nOverallDifficulty:5\nSliderMultiplier:1\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n{objects}\n").as_bytes()).unwrap();
    let prepared = crate::preparation::standard::prepare(&beatmap).unwrap();
    let document = Arc::new(ReplayDocument {
        header: ReplayHeader {
            ruleset_id: 0,
            version: 30_000_016,
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
        frames: vec![crate::replay::ReplayFrame {
            delta_ms: 0,
            time_ms: 0,
            cursor: [0.0; 2],
            keys: 0,
            sequence: 0,
        }],
        seed: None,
        diagnostics: vec![],
    });
    let gameplay = GameplayTimeline {
        policy: Default::default(),
        document,
        input: ReplayTimeline::new(vec![]),
        events: vec![],
        objects: vec![ObjectState::default()],
        checkpoints: vec![],
        initial: Default::default(),
        failure_time: None,
        settlement_time: prepared.last_end,
        recorded_differences: None,
    };
    (beatmap, prepared, gameplay)
}
