//! Mania 列按键边沿、长条头尾与超时判定；只在加载时编译，支持无状态 seek。
use crate::{
    gameplay::{
        events::{EventKind, GameplayEvent, HitResult},
        state::{ObjectState, ReplayStatus},
        timeline::GameplayTimeline,
    },
    replay::{
        compatibility::ReplayPolicy,
        preparation::mania::{result_for, PreparedMania},
        ReplayDocument, ReplayTimeline,
    },
    Result,
};
use std::{collections::BTreeMap, sync::Arc};

pub fn compile_mania(
    prepared: &PreparedMania,
    document: Arc<ReplayDocument>,
) -> Result<GameplayTimeline> {
    let last = document.frames.last().unwrap().time_ms as f64;
    let failure_time = (last < prepared.last_end).then_some(last);
    let settlement_time = failure_time.unwrap_or_else(|| {
        prepared
            .objects
            .iter()
            .map(|o| {
                if o.is_long_note {
                    o.end_time as f64 + prepared.windows[5] * 1.5
                } else {
                    o.start_time as f64 + prepared.windows[5]
                }
            })
            .fold(last, f64::max)
            .next_up()
    });
    let mut engine = Compiler {
        prepared,
        objects: vec![ObjectState::default(); prepared.objects.len()],
        broken: vec![false; prepared.objects.len()],
        active: vec![Vec::new(); prepared.keys as usize],
        queues: vec![Vec::new(); prepared.keys as usize],
        next: vec![0; prepared.keys as usize],
        events: Vec::new(),
    };
    let mut schedule = Vec::with_capacity(document.frames.len() + prepared.objects.len() * 2);
    for (index, frame) in document.frames.iter().enumerate() {
        schedule.push((frame.time_ms as f64, 0, index));
    }
    for (id, object) in prepared.objects.iter().enumerate() {
        engine.queues[object.lane as usize].push(id);
        schedule.push((
            (object.start_time as f64 + prepared.windows[5]).next_up(),
            1,
            id,
        ));
        if object.is_long_note {
            schedule.push((
                (object.end_time as f64 + prepared.windows[5] * 1.5).next_up(),
                2,
                id,
            ));
        }
    }
    schedule.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut previous = 0;
    let mut frames = document.frames.clone();
    for frame in &mut frames {
        frame.keys = frame.cursor[0] as u32;
        frame.cursor = [0.0; 2];
    }
    for (time, phase, id) in schedule {
        if time > settlement_time {
            break;
        }
        if phase == 0 {
            let mask = frames[id].keys;
            for lane in 0..prepared.keys as usize {
                let bit = 1 << lane;
                if previous & bit != 0 && mask & bit == 0 {
                    engine.release(lane, time);
                }
                if previous & bit == 0 && mask & bit != 0 {
                    engine.press(lane, time);
                }
            }
            previous = mask;
        } else if phase == 1 && engine.objects[id].head.is_none() {
            engine.judge(id, false, time, HitResult::Miss, None);
        } else if phase == 2 && engine.objects[id].end.is_none() {
            engine.judge(id, true, time, HitResult::Miss, None);
            if engine.objects[id].tracking_at(time) {
                engine.objects[id].tracking.push((time, false));
            }
        }
    }
    engine.events.sort_by(|a, b| a.time.total_cmp(&b.time));
    let mut score = crate::replay::calculate::mania::Score::new(prepared);
    let initial = score.state.clone();
    let mut checkpoints = Vec::new();
    let mut events = Vec::new();
    for mut event in engine.events {
        let breaks = score.apply(&event);
        event.id = events.len();
        events.push(event.clone());
        if breaks {
            event.id = events.len();
            event.kind = EventKind::ComboBreak;
            events.push(event);
        }
        checkpoints.push((events.last().unwrap().time, score.state.clone()));
    }
    if let Some(time) = failure_time {
        score.state.status = ReplayStatus::Failed;
        events.push(GameplayEvent {
            id: events.len(),
            time,
            object_id: usize::MAX,
            node_id: None,
            result: HitResult::IgnoreMiss,
            maximum: HitResult::IgnoreHit,
            kind: EventKind::Fail,
            error: None,
            position: None,
            sample_time: time,
        });
        for object in &mut engine.objects {
            if object.tracking_at(time) {
                object.tracking.push((time, false));
            }
        }
    } else {
        score.state.status = ReplayStatus::Completed;
    }
    checkpoints.push((settlement_time, score.state.clone()));
    let maximums = BTreeMap::from([(
        "perfect".into(),
        prepared
            .objects
            .iter()
            .map(|o| if o.is_long_note { 2 } else { 1 })
            .sum(),
    )]);
    let recorded_differences = if failure_time.is_none() {
        super::super::validation::validate(&document, &score.state, &maximums)?
    } else {
        None
    };
    Ok(GameplayTimeline {
        document,
        policy: ReplayPolicy::Mania(prepared.policy),
        input: ReplayTimeline::new(frames),
        taiko_inputs: Vec::new(),
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
    prepared: &'a PreparedMania,
    objects: Vec<ObjectState>,
    broken: Vec<bool>,
    active: Vec<Vec<usize>>,
    queues: Vec<Vec<usize>>,
    next: Vec<usize>,
    events: Vec<GameplayEvent>,
}
impl Compiler<'_> {
    fn judge(&mut self, id: usize, tail: bool, time: f64, result: HitResult, error: Option<f64>) {
        let source = &self.prepared.objects[id];
        let state = &mut self.objects[id];
        if tail {
            state.end = Some((time, result));
        } else {
            state.head = Some((time, result));
            if !source.is_long_note {
                state.end = state.head;
            } else if !self.active[source.lane as usize].contains(&id) {
                self.active[source.lane as usize].push(id);
            }
        }
        self.events.push(GameplayEvent {
            id: 0,
            time,
            object_id: id,
            node_id: tail.then_some(0),
            result,
            maximum: HitResult::Perfect,
            kind: EventKind::Judgement,
            error,
            position: None,
            sample_time: if tail {
                source.end_time
            } else {
                source.start_time
            } as f64,
        });
    }
    fn press(&mut self, lane: usize, time: f64) {
        // 漏头或提前松开的长条允许重新按住，但尾判定最多为 50。
        self.active[lane].retain(|&id| self.objects[id].end.is_none());
        for &id in &self.active[lane] {
            let source = &self.prepared.objects[id];
            if time <= source.end_time as f64 + self.prepared.windows[5] {
                self.objects[id].tracking.push((time, true));
            }
        }
        while self.next[lane] < self.queues[lane].len() {
            let id = self.queues[lane][self.next[lane]];
            if self.objects[id].head.is_some() {
                self.next[lane] += 1;
                continue;
            }
            let source = &self.prepared.objects[id];
            let offset = time - source.start_time as f64;
            let Some(result) = result_for(offset, &self.prepared.windows) else {
                return;
            };
            if source.is_long_note {
                self.objects[id].tracking.push((time, true));
            }
            self.judge(id, false, time, result, Some(offset));
            self.next[lane] += 1;
            return;
        }
    }
    fn release(&mut self, lane: usize, time: f64) {
        let active = self.active[lane].clone();
        for id in active {
            if self.objects[id].end.is_some() || !self.objects[id].tracking_at(time) {
                continue;
            }
            let offset = time - self.prepared.objects[id].end_time as f64;
            // legacy 长条尾的所有窗口带 1.5 倍松键宽限，早放手在窗外只打断主体。
            if let Some(mut result) = result_for(offset / 1.5, &self.prepared.windows) {
                let head_hit = self.objects[id].head.is_some_and(|(_, r)| r.is_hit());
                if result.is_hit() && (!head_hit || self.broken[id]) {
                    result = HitResult::Meh;
                }
                self.judge(id, true, time, result, Some(offset));
            } else if !self.broken[id] {
                self.broken[id] = true;
                self.events.push(GameplayEvent {
                    id: 0,
                    time,
                    object_id: id,
                    node_id: Some(1),
                    result: HitResult::IgnoreMiss,
                    maximum: HitResult::IgnoreHit,
                    kind: EventKind::Judgement,
                    error: None,
                    position: None,
                    sample_time: time,
                });
            }
            self.objects[id].tracking.push((time, false));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::preparation::mania::tests::fixture;
    fn compile(objects: &str, frames: &[(i64, u32)]) -> GameplayTimeline {
        let (prepared, document) = fixture(objects, frames);
        compile_mania(&prepared, document).unwrap()
    }
    #[test]
    fn chords_use_mouse_x_mask_and_preserve_same_time_edges_and_seek() {
        let timeline = compile(
            "64,192,1000,1,0\n192,192,1000,1,0\n64,192,1100,1,0",
            &[(0, 0), (1000, 3), (1000, 2), (1100, 3), (1300, 0)],
        );
        assert!(timeline
            .objects
            .iter()
            .all(|o| o.head.unwrap().1 == HitResult::Perfect));
        assert_eq!(timeline.state_at(1400.0).score, 1_000_000);
        assert_eq!(timeline.state_at(1400.0).max_combo, 3);
        assert_eq!(timeline.input_at(1000.0).unwrap().keys, 2);
        assert!(timeline.document.frames.iter().all(|f| f.keys == 0));
        let initial = timeline.state_at(999.0).clone();
        for time in [1400.0, 1100.0, 800.0, 1000.0] {
            timeline.state_at(time);
        }
        assert_eq!(timeline.state_at(999.0), &initial);
    }
    #[test]
    fn holds_judge_release_and_cap_regrab_after_body_break_or_missed_head() {
        for (frames, head, tail, broken) in [
            (
                vec![(0, 0), (1000, 1), (2000, 0), (2300, 0)],
                HitResult::Perfect,
                HitResult::Perfect,
                false,
            ),
            (
                vec![
                    (0, 0),
                    (1000, 1),
                    (1500, 0),
                    (1600, 1),
                    (2000, 0),
                    (2300, 0),
                ],
                HitResult::Perfect,
                HitResult::Meh,
                true,
            ),
            (
                vec![(0, 0), (1500, 1), (2000, 0), (2300, 0)],
                HitResult::Miss,
                HitResult::Meh,
                false,
            ),
            (
                vec![(0, 0), (1000, 1), (2300, 0)],
                HitResult::Perfect,
                HitResult::Miss,
                false,
            ),
        ] {
            let timeline = compile("64,192,1000,128,0,2000:0:0:0:0:", &frames);
            assert_eq!(timeline.objects[0].head.unwrap().1, head);
            assert_eq!(timeline.objects[0].end.unwrap().1, tail);
            assert_eq!(
                timeline
                    .events
                    .iter()
                    .any(|e| e.maximum == HitResult::IgnoreHit && e.kind == EventKind::Judgement),
                broken
            );
            assert!(!timeline.objects[0].tracking_at(2400.0));
        }
    }
    #[test]
    fn all_six_judgements_and_early_end_freeze_future_state() {
        for (offset, result) in [
            (0, HitResult::Perfect),
            (30, HitResult::Great),
            (65, HitResult::Good),
            (100, HitResult::Ok),
            (125, HitResult::Meh),
            (160, HitResult::Miss),
        ] {
            let timeline = compile("64,192,1000,1,0", &[(0, 0), (1000 + offset, 1), (1400, 0)]);
            assert_eq!(timeline.objects[0].head.unwrap().1, result);
        }
        let timeline = compile(
            "64,192,1000,1,0\n192,192,2000,1,0",
            &[(0, 0), (1000, 1), (1100, 0)],
        );
        assert_eq!(timeline.failure_time, Some(1100.0));
        assert!(timeline.objects[1].head.is_none());
        assert_eq!(timeline.state_at(1200.0), timeline.state_at(5000.0));
        assert_eq!(timeline.state_at(5000.0).status, ReplayStatus::Failed);
    }
}
