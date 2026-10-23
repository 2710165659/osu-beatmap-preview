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
mod tests;
