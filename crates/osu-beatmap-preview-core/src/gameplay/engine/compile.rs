//! 输入边沿、节点和超时按明确次序编译；连续对象使用策略内固定积分网格。
use crate::calculate::standard::{circle, score::Score, slider, spinner};
use crate::gameplay::events::{EventKind, GameplayEvent, HitResult};
use crate::gameplay::state::{ObjectState, ReplayStatus};
use crate::gameplay::timeline::GameplayTimeline;
use crate::preparation::standard::{NodeKind, PreparedStandard};
use crate::replay::{compatibility, timeline::standard_actions, ReplayDocument, ReplayTimeline};
use crate::support::error::{PreviewError, Result};
use std::sync::Arc;
mod classic;
mod input_mods;
mod objects;
mod schedule;

pub fn compile_standard(
    prepared: &PreparedStandard,
    document: Arc<ReplayDocument>,
) -> Result<GameplayTimeline> {
    let mut timeline = compile_timeline(prepared, document.clone())?;
    if timeline.failure_time.is_none() {
        timeline.recorded_differences = super::validation::validate(
            &document,
            timeline.state_at(f64::INFINITY),
            &maximum_statistics(prepared, timeline.policy),
        )?;
    }
    Ok(timeline)
}

fn compile_timeline(
    prepared: &PreparedStandard,
    document: Arc<ReplayDocument>,
) -> Result<GameplayTimeline> {
    let policy = compatibility::select(&document)?;
    let input = input_mods::apply(prepared, &document);
    let last = input.last_time() as f64;
    let failure = (last < prepared.last_end).then_some(last);
    let deadline = if failure.is_some() {
        last
    } else {
        prepared
            .objects
            .iter()
            .map(|o| {
                (o.source.start_time as f64 + circle::windows(prepared.od)[2] + 1.0).max(o.end)
            })
            .fold(last, f64::max)
    };
    let mut engine = Compiler {
        policy,
        prepared,
        input: &input,
        objects: prepared
            .objects
            .iter()
            .map(|o| ObjectState {
                nodes: vec![None; o.nodes.len()],
                ..Default::default()
            })
            .collect(),
        events: Vec::new(),
        previous_actions: 0,
        head_actions: vec![0; prepared.objects.len()],
        rotations: (0..prepared.objects.len())
            .map(|_| spinner::Rotation::default())
            .collect(),
        last_angles: vec![None; prepared.objects.len()],
        any_key_after: vec![None; prepared.objects.len()],
        active: Vec::new(),
        next_active: 0,
        first_unjudged: 0,
        last_actions: vec![0; prepared.objects.len()],
        previous_time_actions: vec![0; prepared.objects.len()],
        last_action_times: vec![None; prepared.objects.len()],
        displayed_angles: vec![0.0; prepared.objects.len()],
        rotation_times: vec![None; prepared.objects.len()],
        legacy_relax: prepared.mods.settings.relax && document.header.version < 30_000_000,
        relax_actions: 0,
        relax_next: 2,
        relax_changed: f64::NEG_INFINITY,
        relaxed_frames: Vec::new(),
    };
    // 1ms 网格仅用于连续追踪；点击仍逐个消费原始帧（含同刻按键）。
    // 输出 FPS 和音频窗口完全不参与此积分；节点的分数时刻保留 f64。
    let points = schedule::build(prepared, &document, deadline)?;
    for (time, phase, id) in points {
        if time > deadline {
            break;
        }
        match phase {
            0 => engine.input_frame(id, time),
            1 => engine.relax_update(time),
            2 => engine.continuous(id, time),
            3 => engine.timeout(id, time),
            _ => engine.finish(id, time),
        }
    }
    engine.events.sort_by(|a, b| a.time.total_cmp(&b.time));
    // 先以全部最大判定模拟最大组合部分，再计算实际动态分数，避免从 OSR 总分反推。
    let mut maximums = Vec::new();
    for object in &prepared.objects {
        let classic_slider = policy.classic_sliders && object.path.is_some();
        maximums.push((
            object.source.start_time as f64,
            if classic_slider {
                HitResult::LargeTickHit
            } else {
                HitResult::Great
            },
        ));
        for node in &object.nodes {
            maximums.push((
                node.time,
                if node.kind == NodeKind::Tail && classic_slider {
                    HitResult::SmallTickHit
                } else if node.kind == NodeKind::Tail {
                    HitResult::SliderTailHit
                } else {
                    HitResult::LargeTickHit
                },
            ));
        }
        if classic_slider {
            maximums.push((object.end, HitResult::Great));
        }
    }
    maximums.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut score = Score::new(
        &maximums.iter().map(|p| p.1).collect::<Vec<_>>(),
        policy.score_multiplier() * prepared.score_multiplier,
    );
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
    if let Some(time) = failure {
        score.state.status = ReplayStatus::Failed;
        events.push(GameplayEvent {
            id: events.len(),
            time,
            object_id: usize::MAX,
            node_id: None,
            kind: EventKind::Fail,
            result: HitResult::IgnoreMiss,
            maximum: HitResult::IgnoreHit,
            error: None,
            position: None,
            sample_time: time,
        });
        for object in &mut engine.objects {
            if object.tracking_at(time) {
                object.tracking.push((time, false));
            }
            if object.spinning_at(time) {
                object.spinning.push((time, false));
            }
        }
    } else {
        score.state.status = ReplayStatus::Completed;
    }
    checkpoints.push((deadline, score.state));
    let effective_input = if engine.legacy_relax {
        ReplayTimeline::new(std::mem::take(&mut engine.relaxed_frames))
    } else {
        input.clone()
    };
    let objects = engine.objects;
    Ok(GameplayTimeline {
        document,
        policy,
        input: effective_input,
        events,
        objects,
        checkpoints,
        initial: Default::default(),
        failure_time: failure,
        settlement_time: deadline,
        recorded_differences: None,
    })
}

struct Compiler<'a> {
    policy: compatibility::StandardPolicy,
    prepared: &'a PreparedStandard,
    input: &'a ReplayTimeline,
    objects: Vec<ObjectState>,
    events: Vec<GameplayEvent>,
    previous_actions: u32,
    head_actions: Vec<u32>,
    rotations: Vec<spinner::Rotation>,
    last_angles: Vec<Option<f32>>,
    any_key_after: Vec<Option<f64>>,
    active: Vec<usize>,
    next_active: usize,
    first_unjudged: usize,
    last_actions: Vec<u32>,
    previous_time_actions: Vec<u32>,
    last_action_times: Vec<Option<f64>>,
    displayed_angles: Vec<f32>,
    rotation_times: Vec<Option<f64>>,
    legacy_relax: bool,
    relax_actions: u32,
    relax_next: u32,
    relax_changed: f64,
    relaxed_frames: Vec<crate::replay::ReplayFrame>,
}

fn maximum_statistics(
    prepared: &PreparedStandard,
    policy: compatibility::StandardPolicy,
) -> std::collections::BTreeMap<String, u32> {
    let mut values = std::collections::BTreeMap::new();
    let mut add = |result: HitResult, count| {
        *values.entry(result.name().to_owned()).or_default() += count;
    };
    for object in &prepared.objects {
        add(HitResult::Great, 1);
        let classic_slider = object.path.is_some() && policy.classic_sliders;
        if classic_slider {
            add(HitResult::LargeTickHit, 1);
        } else if object.path.is_some() {
            add(HitResult::IgnoreHit, 1);
        }
        for node in &object.nodes {
            add(
                if node.kind == NodeKind::Tail && classic_slider {
                    HitResult::SmallTickHit
                } else if node.kind == NodeKind::Tail {
                    HitResult::SliderTailHit
                } else {
                    HitResult::LargeTickHit
                },
                1,
            );
        }
        if object.source.hit_type & 8 != 0 {
            let (required, total) =
                spinner::required(object.end - object.source.start_time as f64, prepared.od);
            add(HitResult::SmallBonus, required + 2);
            add(HitResult::LargeBonus, total - required - 2);
        }
    }
    values
}

#[cfg(test)]
mod tests;
