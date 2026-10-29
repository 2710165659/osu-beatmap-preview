//! Taiko 原始按键边沿与超时编译；生成与 FPS、seek 和音频预读无关的不可变结果。
use crate::gameplay::{
    events::{EventKind, GameplayEvent, HitResult},
    state::{ObjectState, ReplayStatus},
    timeline::{GameplayTimeline, TaikoPress},
};
use crate::replay::{
    calculate::taiko::Score,
    compatibility::{self, ReplayPolicy},
    preparation::taiko::{PreparedTaiko, TaikoObjectKind},
    ReplayDocument, ReplayTimeline,
};
use crate::support::error::{PreviewError, Result};
use std::{collections::BTreeMap, sync::Arc};

pub fn compile_taiko(
    prepared: &PreparedTaiko,
    document: Arc<ReplayDocument>,
) -> Result<GameplayTimeline> {
    let ReplayPolicy::Taiko(policy) = compatibility::select_replay(&document)? else {
        return Err(PreviewError::new("Taiko compiler received Standard replay"));
    };
    let windows = crate::domain::rulesets::taiko::gameplay::hit_windows(prepared.od);
    let last_input = document.frames.last().unwrap().time_ms as f64;
    let failure_time = (last_input < prepared.last_end).then_some(last_input);
    let settlement_time = failure_time.unwrap_or_else(|| {
        prepared
            .objects
            .iter()
            .map(|o| match &o.kind {
                TaikoObjectKind::Hit => o.source.start_time as f64 + windows[2].floor() + 1.0,
                TaikoObjectKind::DrumRoll { ticks, window } => ticks
                    .last()
                    .map_or(o.source.end_time as f64, |t| (t.time + window).next_up()),
                TaikoObjectKind::Swell { .. } => o.source.end_time as f64,
            })
            .fold(last_input, f64::max)
    });
    let mut engine = Compiler {
        prepared,
        windows,
        objects: prepared
            .objects
            .iter()
            .map(|o| ObjectState {
                nodes: vec![None; o.node_count()],
                ..Default::default()
            })
            .collect(),
        events: Vec::new(),
        presses: Vec::new(),
        first_key: vec![0; prepared.objects.len()],
        last_press: vec![None; prepared.objects.len()],
        swell_colour: vec![None; prepared.objects.len()],
        active: Vec::new(),
        next_active: 0,
    };
    // 同刻输入保持文件顺序；边界内的按键先于严格超窗的自动 Miss。
    let mut points = Vec::new();
    for (id, frame) in document.frames.iter().enumerate() {
        points.push((frame.time_ms as f64, 0_u8, id, None));
        points.push((frame.time_ms as f64 + 31.0, 1, usize::MAX, None));
    }
    for (id, object) in prepared.objects.iter().enumerate() {
        match &object.kind {
            TaikoObjectKind::Hit => points.push((
                object.source.start_time as f64 + windows[2].floor() + 1.0,
                1,
                id,
                None,
            )),
            TaikoObjectKind::DrumRoll { ticks, window } => {
                for (node, tick) in ticks.iter().enumerate() {
                    points.push(((tick.time + window).next_up(), 1, id, Some(node)));
                }
                points.push((object.source.end_time as f64, 2, id, None));
            }
            TaikoObjectKind::Swell { .. } => {
                points.push((object.source.end_time as f64, 1, id, None))
            }
        }
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut previous = 0;
    for (time, phase, id, node) in points {
        if time > settlement_time {
            break;
        }
        engine.expire_strong(time);
        if phase == 0 {
            let frame = &document.frames[id];
            let pressed = frame.keys & !previous;
            let released = previous & !frame.keys;
            for key in &mut engine.first_key {
                if *key & released != 0 {
                    *key = 0;
                }
            }
            previous = frame.keys;
            engine.activate(time);
            for key in [1, 2, 4, 8] {
                if pressed & key == 0 {
                    continue;
                }
                let target = engine.press(time, key, frame.keys);
                engine.presses.push(TaikoPress {
                    time,
                    key,
                    object_id: target.map(|p| p.0),
                    node_id: target.and_then(|p| p.1),
                });
            }
        } else if id != usize::MAX {
            engine.timeout(time, id, node, phase == 2);
        }
    }
    engine.events.sort_by(|a, b| a.time.total_cmp(&b.time));
    let mut score = Score::new(prepared);
    let initial = score.state.clone();
    let mut checkpoints = Vec::new();
    let mut events = Vec::new();
    for mut event in engine.events {
        let combo_break = score.apply(&event, prepared);
        event.id = events.len();
        events.push(event.clone());
        if combo_break {
            event.id = events.len();
            event.kind = EventKind::ComboBreak;
            events.push(event);
        }
        checkpoints.push((events.last().unwrap().time, score.state.clone()));
    }
    score.state.status = if failure_time.is_some() || score.state.health < 0.5 {
        ReplayStatus::Failed
    } else {
        ReplayStatus::Completed
    };
    if score.state.status == ReplayStatus::Failed {
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
    let maximums = maximum_statistics(prepared);
    let recorded_differences = if failure_time.is_none() {
        super::super::validation::validate(&document, &score.state, &maximums)?
    } else {
        None
    };
    Ok(GameplayTimeline {
        input: ReplayTimeline::new(document.frames.clone()),
        document,
        policy: ReplayPolicy::Taiko(policy),
        taiko_inputs: engine.presses,
        events,
        objects: engine.objects,
        checkpoints,
        initial,
        failure_time,
        settlement_time,
        recorded_differences,
    })
}

struct Compiler<'a> {
    prepared: &'a PreparedTaiko,
    windows: [f64; 3],
    objects: Vec<ObjectState>,
    events: Vec<GameplayEvent>,
    presses: Vec<TaikoPress>,
    first_key: Vec<u32>,
    last_press: Vec<Option<f64>>,
    swell_colour: Vec<Option<bool>>,
    active: Vec<usize>,
    next_active: usize,
}

impl Compiler<'_> {
    fn activate(&mut self, time: f64) {
        while let Some(object) = self.prepared.objects.get(self.next_active) {
            if object.source.start_time as f64 - self.windows[2] > time {
                break;
            }
            self.active.push(self.next_active);
            self.next_active += 1;
        }
        self.active.retain(|&id| {
            let o = &self.prepared.objects[id];
            time <= o.source.end_time as f64 + self.windows[2] + 31.0
        });
    }

    fn emit(
        &mut self,
        time: f64,
        id: usize,
        node: Option<usize>,
        result: HitResult,
        maximum: HitResult,
        sample_time: f64,
        error: Option<f64>,
    ) {
        if let Some(node) = node {
            self.objects[id].nodes[node] = Some((time, result));
        } else {
            self.objects[id].head = Some((time, result));
        }
        self.events.push(GameplayEvent {
            id: 0,
            time,
            object_id: id,
            node_id: node,
            result,
            maximum,
            kind: EventKind::Judgement,
            error,
            position: None,
            sample_time,
        });
    }

    fn press(&mut self, time: f64, key: u32, held: u32) -> Option<(usize, Option<usize>)> {
        for index in 0..self.active.len() {
            let id = self.active[index];
            let object = &self.prepared.objects[id];
            let start = object.source.start_time as f64;
            match &object.kind {
                TaikoObjectKind::Hit => {
                    if object.strong()
                        && self.objects[id].nodes[0].is_none()
                        && self.objects[id]
                            .head
                            .is_some_and(|(hit, result)| result.is_hit() && time - hit <= 30.0)
                        && held & self.first_key[id] != 0
                        && key != self.first_key[id]
                        && (key & 10 != 0) == object.rim()
                    {
                        self.emit(
                            time,
                            id,
                            Some(0),
                            HitResult::LargeBonus,
                            HitResult::LargeBonus,
                            start,
                            None,
                        );
                        return Some((id, Some(0)));
                    }
                    if self.last_press[id] == Some(time) {
                        return Some((id, None));
                    }
                    if self.objects[id].head.is_some() {
                        continue;
                    }
                    self.last_press[id] = Some(time);
                    let error = time - start;
                    if error.abs() > self.windows[2] {
                        continue;
                    }
                    let result = if (key & 10 != 0) != object.rim() {
                        HitResult::Miss
                    } else if error.abs() <= self.windows[0] {
                        HitResult::Great
                    } else if error.abs() <= self.windows[1] {
                        HitResult::Ok
                    } else {
                        HitResult::Miss
                    };
                    self.first_key[id] = key;
                    self.emit(
                        time,
                        id,
                        None,
                        result,
                        HitResult::Great,
                        start,
                        result.is_hit().then_some(error),
                    );
                    if object.strong() && !result.is_hit() {
                        self.emit(
                            time,
                            id,
                            Some(0),
                            HitResult::IgnoreMiss,
                            HitResult::LargeBonus,
                            start,
                            None,
                        );
                    }
                    return Some((id, None));
                }
                TaikoObjectKind::DrumRoll { ticks, window } => {
                    let Some(node) = self.objects[id].nodes.iter().position(Option::is_none) else {
                        continue;
                    };
                    let sample_time = ticks[node].time;
                    if (time - sample_time).abs() > *window {
                        continue;
                    }
                    self.emit(
                        time,
                        id,
                        Some(node),
                        HitResult::SmallBonus,
                        HitResult::SmallBonus,
                        sample_time,
                        None,
                    );
                    if object.strong() {
                        // Strong 滚奏节点自动跟随父节点，不要求另一只手补击。
                        self.events.push(GameplayEvent {
                            id: 0,
                            time,
                            object_id: id,
                            node_id: Some(node),
                            result: HitResult::LargeBonus,
                            maximum: HitResult::LargeBonus,
                            kind: EventKind::Judgement,
                            error: None,
                            position: None,
                            sample_time,
                        });
                    }
                    return Some((id, Some(node)));
                }
                TaikoObjectKind::Swell { required } => {
                    if time < start
                        || time > object.source.end_time as f64
                        || self.objects[id].head.is_some()
                    {
                        continue;
                    }
                    let rim = key & 10 != 0;
                    if self.swell_colour[id] == Some(rim) {
                        continue;
                    }
                    if self.last_press[id] == Some(time) {
                        return Some((id, None));
                    }
                    self.swell_colour[id] = Some(rim);
                    self.last_press[id] = Some(time);
                    let node = self.objects[id]
                        .nodes
                        .iter()
                        .position(Option::is_none)
                        .unwrap();
                    self.emit(
                        time,
                        id,
                        Some(node),
                        HitResult::IgnoreHit,
                        HitResult::IgnoreHit,
                        start,
                        None,
                    );
                    if node + 1 == *required as usize {
                        self.emit(
                            time,
                            id,
                            None,
                            HitResult::LargeBonus,
                            HitResult::LargeBonus,
                            start,
                            None,
                        );
                        self.objects[id].end = Some((time, HitResult::LargeBonus));
                    }
                    return Some((id, Some(node)));
                }
            }
        }
        None
    }

    fn expire_strong(&mut self, time: f64) {
        for index in 0..self.active.len() {
            let id = self.active[index];
            let o = &self.prepared.objects[id];
            if matches!(o.kind, TaikoObjectKind::Hit)
                && o.strong()
                && self.objects[id].nodes[0].is_none()
            {
                if let Some((hit, _)) = self.objects[id].head.filter(|(hit, _)| time > hit + 30.0) {
                    self.emit(
                        hit + 31.0,
                        id,
                        Some(0),
                        HitResult::IgnoreMiss,
                        HitResult::LargeBonus,
                        o.source.start_time as f64,
                        None,
                    );
                }
            }
        }
    }

    fn timeout(&mut self, time: f64, id: usize, node: Option<usize>, finish_roll: bool) {
        let o = &self.prepared.objects[id];
        let start = o.source.start_time as f64;
        if finish_roll {
            self.objects[id].end = Some((time, HitResult::IgnoreHit));
            self.emit(
                time,
                id,
                None,
                HitResult::IgnoreHit,
                HitResult::IgnoreHit,
                start,
                None,
            );
            return;
        }
        if let Some(node) = node {
            if self.objects[id].nodes[node].is_none() {
                self.emit(
                    time,
                    id,
                    Some(node),
                    HitResult::IgnoreMiss,
                    HitResult::SmallBonus,
                    start,
                    None,
                );
                if o.strong() {
                    self.events.push(GameplayEvent {
                        id: 0,
                        time,
                        object_id: id,
                        node_id: Some(node),
                        result: HitResult::IgnoreMiss,
                        maximum: HitResult::LargeBonus,
                        kind: EventKind::Judgement,
                        error: None,
                        position: None,
                        sample_time: start,
                    });
                }
            }
        } else if self.objects[id].head.is_none() {
            let (result, maximum) = if matches!(o.kind, TaikoObjectKind::Hit) {
                (HitResult::Miss, HitResult::Great)
            } else {
                (HitResult::IgnoreMiss, HitResult::LargeBonus)
            };
            self.emit(time, id, None, result, maximum, start, None);
            for node in 0..self.objects[id].nodes.len() {
                if self.objects[id].nodes[node].is_none() {
                    self.emit(
                        time,
                        id,
                        Some(node),
                        HitResult::IgnoreMiss,
                        if matches!(o.kind, TaikoObjectKind::Hit) {
                            HitResult::LargeBonus
                        } else {
                            HitResult::IgnoreHit
                        },
                        start,
                        None,
                    );
                }
            }
            self.objects[id].end = Some((time, result));
        }
    }
}

fn maximum_statistics(prepared: &PreparedTaiko) -> BTreeMap<String, u32> {
    let mut values = BTreeMap::new();
    let mut add =
        |result: HitResult, count| *values.entry(result.name().into()).or_default() += count;
    for o in &prepared.objects {
        match &o.kind {
            TaikoObjectKind::Hit => {
                add(HitResult::Great, 1);
                if o.strong() {
                    add(HitResult::LargeBonus, 1);
                }
            }
            TaikoObjectKind::DrumRoll { ticks, .. } => {
                add(HitResult::IgnoreHit, 1);
                add(HitResult::SmallBonus, ticks.len() as u32);
                if o.strong() {
                    add(HitResult::LargeBonus, ticks.len() as u32);
                }
            }
            TaikoObjectKind::Swell { required } => {
                add(HitResult::IgnoreHit, *required);
                add(HitResult::LargeBonus, 1);
            }
        }
    }
    values
}

#[cfg(test)]
mod tests {
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
}
