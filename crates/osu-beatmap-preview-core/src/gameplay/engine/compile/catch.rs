//! Catch 按物件时刻采样回放位置，生成独立于帧率与 seek 的判定事件。
use crate::render::cpu::modes::catch::objects::ObjType;
use crate::{
    gameplay::{
        events::{EventKind, GameplayEvent, HitResult},
        state::{ObjectState, ReplayStatus},
        timeline::GameplayTimeline,
    },
    replay::{
        calculate::catch::Score,
        compatibility::{self, ReplayPolicy},
        preparation::catch::PreparedCatch,
        ReplayDocument, ReplayTimeline,
    },
    PreviewError, Result,
};
use std::{collections::BTreeMap, sync::Arc};

pub fn compile_catch(
    prepared: &PreparedCatch,
    document: Arc<ReplayDocument>,
) -> Result<GameplayTimeline> {
    let ReplayPolicy::Catch(policy) = compatibility::select_replay(&document)? else {
        return Err(PreviewError::new("Catch compiler requires ruleset 2"));
    };
    let input = ReplayTimeline::new(document.frames.clone());
    let last_input = input.last_time() as f64;
    let failure_time = (last_input < prepared.last_end).then_some(last_input);
    let settlement_time = failure_time.unwrap_or(prepared.last_end);
    let mut score = Score::new(prepared);
    let initial = score.state.clone();
    let mut checkpoints = Vec::new();
    let mut events = Vec::new();
    let mut objects = vec![ObjectState::default(); prepared.objects.len()];
    let mut maximums = BTreeMap::new();
    for (id, object) in prepared.objects.iter().enumerate() {
        let (maximum, miss) = match object.object_type {
            ObjType::Fruit => (HitResult::Great, HitResult::Miss),
            ObjType::Droplet => (HitResult::LargeTickHit, HitResult::LargeTickMiss),
            ObjType::TinyDroplet => (HitResult::SmallTickHit, HitResult::SmallTickMiss),
            ObjType::Banana => (HitResult::LargeBonus, HitResult::IgnoreMiss),
        };
        *maximums.entry(maximum.name().to_owned()).or_default() += 1;
        let time = object.event_time_or_start();
        if time > settlement_time {
            continue;
        }
        let position = input.snapshot_at(time).and_then(|s| s.cursor);
        let caught = position.is_some_and(|p| {
            (f64::from(p[0]).clamp(0.0, 512.0) - object.x.clamp(0.0, 512.0)).abs()
                <= prepared.half_catch_width
        });
        let result = if caught { maximum } else { miss };
        objects[id].head = Some((time, result));
        objects[id].end = Some((time, result));
        let mut event = GameplayEvent {
            id: events.len(),
            time,
            object_id: id,
            node_id: None,
            result,
            maximum,
            kind: EventKind::Judgement,
            error: None,
            position,
            sample_time: time,
        };
        let combo_break = score.apply(&event, prepared);
        events.push(event.clone());
        if combo_break {
            event.id = events.len();
            event.kind = EventKind::ComboBreak;
            events.push(event);
        }
        checkpoints.push((time, score.state.clone()));
    }
    score.state.status = if failure_time.is_some() {
        ReplayStatus::Failed
    } else {
        ReplayStatus::Completed
    };
    if failure_time.is_some() {
        events.push(GameplayEvent {
            id: events.len(),
            time: settlement_time,
            object_id: usize::MAX,
            node_id: None,
            result: HitResult::IgnoreMiss,
            maximum: HitResult::IgnoreHit,
            kind: EventKind::Fail,
            error: None,
            position: None,
            sample_time: settlement_time,
        });
    }
    checkpoints.push((settlement_time, score.state.clone()));
    let recorded_differences = if failure_time.is_none() {
        super::super::validation::validate(&document, &score.state, &maximums)?
    } else {
        None
    };
    Ok(GameplayTimeline {
        input,
        document,
        policy: ReplayPolicy::Catch(policy),
        taiko_inputs: Vec::new(),
        events,
        objects,
        checkpoints,
        initial,
        failure_time,
        settlement_time,
        recorded_differences,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::{preparation::catch::prepare_replay, ReplayFrame};

    fn fixture() -> (PreparedCatch, ReplayDocument) {
        let map = crate::parse_beatmap_bytes(include_bytes!(
            "../../../../../osu-beatmap-preview-web/test/fixtures/replay-catch.osu"
        ))
        .unwrap();
        let document = crate::replay::parse_osr(include_bytes!(
            "../../../../../osu-beatmap-preview-web/test/fixtures/replay-catch.osr"
        ))
        .unwrap();
        (prepare_replay(&map, &document).unwrap(), document)
    }

    #[test]
    fn native_and_converted_replays_support_implicit_and_explicit_classic() {
        let (prepared, document) = fixture();
        assert!(prepared.mods.settings.classic);
        assert!(prepared
            .objects
            .iter()
            .any(|o| o.object_type == ObjType::TinyDroplet));
        assert!(prepared
            .objects
            .iter()
            .any(|o| o.object_type == ObjType::Banana));
        let native = compile_catch(&prepared, Arc::new(document.clone())).unwrap();
        assert_eq!(native.objects[0].head.unwrap().1, HitResult::Great);
        assert_eq!(native.objects[1].head.unwrap().1, HitResult::Miss);
        assert_eq!(
            native.state_at(f64::INFINITY).status,
            ReplayStatus::Completed
        );
        assert_eq!(native.state_at(1000.0).combo, 1);
        assert_eq!(native.state_at(1500.0).combo, 0);
        // Standard 转谱使用解析器重新构造原始物件，避免只改模式号制造无效模型。
        let text =
            include_str!("../../../../../osu-beatmap-preview-web/test/fixtures/replay-catch.osu")
                .replace("Mode:2", "Mode:0");
        let converted = crate::parse_beatmap_bytes(text.as_bytes()).unwrap();
        for version in [30_000_016, 30_000_019, 40_000_000] {
            let mut replay = document.clone();
            replay.header.version = version;
            replay.metadata = Some(
                serde_json::from_value(serde_json::json!({"mods":[{"acronym":"CL"}]})).unwrap(),
            );
            let p = prepare_replay(&converted, &replay).unwrap();
            assert_eq!(p.beatmap.mode(), 2);
            let timeline = compile_catch(&p, Arc::new(replay)).unwrap();
            assert_eq!(
                native.state_at(f64::INFINITY),
                timeline.state_at(f64::INFINITY)
            );
        }
    }

    #[test]
    fn fractional_slider_tail_is_judged_before_completion() {
        let (_, document) = fixture();
        let original =
            include_str!("../../../../../osu-beatmap-preview-web/test/fixtures/replay-catch.osu");
        let text = format!(
            "{}[HitObjects]\n256,192,2000,6,0,L|397:192,2,141\n",
            original.split("[HitObjects]").next().unwrap()
        );
        let map = crate::parse_beatmap_bytes(text.as_bytes()).unwrap();
        let prepared = prepare_replay(&map, &document).unwrap();
        let tail_time = prepared.objects.last().unwrap().event_time_or_start();
        assert!(tail_time.fract() > 0.0);
        assert_eq!(prepared.last_end, tail_time);
        let timeline = compile_catch(&prepared, Arc::new(document)).unwrap();
        assert_eq!(timeline.settlement_time, tail_time);
        assert!(timeline.objects.last().unwrap().head.is_some());
        assert_eq!(
            timeline.state_at(tail_time - 0.001).status,
            ReplayStatus::Playing
        );
        assert_eq!(timeline.state_at(tail_time).status, ReplayStatus::Completed);
    }

    #[test]
    fn catch_boundary_uses_interpolation_and_last_same_time_frame() {
        let (mut prepared, mut document) = fixture();
        prepared.objects.truncate(1);
        prepared.last_end = 1000.0;
        document.frames = [(0, 0.0), (1000, 50.0), (1000, 100.0), (2000, 200.0)]
            .into_iter()
            .enumerate()
            .map(|(sequence, (time_ms, x))| ReplayFrame {
                time_ms,
                delta_ms: 0,
                cursor: [x, 0.0],
                keys: 0,
                sequence,
            })
            .collect();
        for (offset, expected) in [(0.0, HitResult::Great), (0.001, HitResult::Miss)] {
            prepared.objects[0].x = 100.0 + prepared.half_catch_width + offset;
            let timeline = compile_catch(&prepared, Arc::new(document.clone())).unwrap();
            assert_eq!(timeline.objects[0].head.unwrap().1, expected);
            assert_eq!(timeline.input_at(500.0).unwrap().cursor.unwrap()[0], 25.0);
            assert_eq!(timeline.input_at(1000.0).unwrap().cursor.unwrap()[0], 100.0);
        }
    }

    #[test]
    fn tiny_droplets_and_bananas_do_not_break_combo_and_bonus_excludes_accuracy() {
        let (mut prepared, mut document) = fixture();
        let template = prepared.objects[0].clone();
        prepared.objects = [
            ObjType::Fruit,
            ObjType::TinyDroplet,
            ObjType::Droplet,
            ObjType::Banana,
            ObjType::Banana,
            ObjType::Fruit,
        ]
        .into_iter()
        .enumerate()
        .map(|(id, kind)| {
            let mut o = template.clone();
            o.object_type = kind;
            o.start_time = (id as i64 + 1) * 1000;
            o.event_time = None;
            o.x = if matches!(id, 1..=3) { 400.0 } else { 100.0 };
            o
        })
        .collect();
        prepared.last_end = 6000.0;
        document.frames = vec![
            ReplayFrame {
                time_ms: 0,
                delta_ms: 0,
                cursor: [100.0, 0.0],
                keys: 0,
                sequence: 0,
            },
            ReplayFrame {
                time_ms: 6000,
                delta_ms: 6000,
                cursor: [100.0, 0.0],
                keys: 0,
                sequence: 1,
            },
        ];
        let timeline = compile_catch(&prepared, Arc::new(document.clone())).unwrap();
        assert_eq!(timeline.state_at(2000.0).combo, 1);
        assert_eq!(timeline.state_at(3000.0).combo, 0);
        assert_eq!(timeline.state_at(6000.0).accuracy, Some(0.5));
        assert_eq!(
            timeline
                .events
                .iter()
                .filter(|e| e.kind == EventKind::ComboBreak)
                .count(),
            1
        );
        for o in &mut prepared.objects {
            o.x = 100.0;
        }
        let perfect = compile_catch(&prepared, Arc::new(document)).unwrap();
        assert_eq!(perfect.state_at(6000.0).score, 1_000_400);
        assert_eq!(perfect.state_at(6000.0).combo, 3);
        assert_eq!(perfect.state_at(6000.0).accuracy, Some(1.0));
    }

    #[test]
    fn early_end_freezes_input_results_and_events_without_judging_future_objects() {
        let (prepared, mut document) = fixture();
        document.frames.retain(|f| f.time_ms <= 1500);
        let timeline = compile_catch(&prepared, Arc::new(document)).unwrap();
        assert_eq!(timeline.failure_time, Some(1500.0));
        assert_eq!(timeline.state_at(1500.0).status, ReplayStatus::Failed);
        assert_eq!(timeline.state_at(1500.0), timeline.state_at(6000.0));
        assert_eq!(timeline.input_at(6000.0).unwrap().keys, 0);
        assert!(timeline.objects.iter().skip(2).all(|o| o.head.is_none()));
        assert!(timeline.events.iter().all(|e| e.time <= 1500.0));
    }

    #[test]
    fn rejects_invalid_modes_buttons_mod_settings_and_nonfinite_difficulty() {
        let (prepared, document) = fixture();
        for mods in [
            serde_json::json!([{"acronym":"CL","settings":{"classic_note_lock":true}}]),
            serde_json::json!([{"acronym":"SW"}]),
            serde_json::json!([{"acronym":"HR"},{"acronym":"EZ"}]),
        ] {
            let mut d = document.clone();
            d.metadata = Some(serde_json::from_value(serde_json::json!({"mods":mods})).unwrap());
            assert!(prepare_replay(&prepared.beatmap, &d).is_err());
        }
        let mut d = document.clone();
        d.frames[0].keys = 2;
        assert!(prepare_replay(&prepared.beatmap, &d).is_err());
        let mut d = document;
        d.frames[0].cursor[0] = f32::NAN;
        assert!(prepare_replay(&prepared.beatmap, &d).is_err());
        let mut map = prepared.beatmap;
        map.difficulty.insert("CircleSize", "NaN".to_owned());
        assert!(prepare_replay(&map, &fixture().1).is_err());
    }
}
