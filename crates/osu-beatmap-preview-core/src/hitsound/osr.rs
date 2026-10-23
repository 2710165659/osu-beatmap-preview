//! 成功事件决定一次性起音，追踪/旋转轨迹决定循环；音频查询不推进判定。
use super::{
    common::*,
    timeline::{NamedEvent, TimelineBuilder},
    *,
};
use crate::gameplay::{
    events::{EventKind, HitResult},
    timeline::GameplayTimeline,
};
use crate::replay::preparation::standard::{NodeKind, PreparedStandard};

pub fn build_timeline(
    beatmap: &Beatmap,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    library: &SampleLibrary,
) -> HitsoundTimeline {
    let mut resolver = library;
    build_with(beatmap, prepared, gameplay, &mut resolver)
}
pub fn referenced_names(
    beatmap: &Beatmap,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
) -> Vec<String> {
    let mut resolver = CollectNames::default();
    build_with(beatmap, prepared, gameplay, &mut resolver);
    resolver.into_names()
}
fn build_with<R: SampleResolver>(
    beatmap: &Beatmap,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    resolver: &mut R,
) -> HitsoundTimeline {
    let mut result = HitsoundTimeline::default();
    let policy = gameplay.standard_policy();
    for event in &gameplay.events {
        let mut builder = TimelineBuilder::new(resolver);
        if event.kind == EventKind::Fail || event.kind == EventKind::ComboBreak {
            let name = if event.kind == EventKind::Fail {
                "failsound"
            } else {
                "combobreak"
            };
            builder.push_at(
                std::iter::once(name.as_bytes()),
                100,
                event.time,
                0.0,
                false,
            );
            result.events.extend(builder.finish().events);
            continue;
        }
        let object = &prepared.objects[event.object_id];
        let tail = event
            .node_id
            .and_then(|id| object.nodes.get(id))
            .is_some_and(|node| node.kind == NodeKind::Tail);
        let forced_tail = tail && policy.always_tail_sample;
        if (!event.result.is_hit() && !forced_tail)
            || matches!(event.result, HitResult::IgnoreHit | HitResult::SmallBonus)
        {
            continue;
        }
        // CL 的父滑条只负责最终判定；尾音已经由尾节点在真实尾部播放，不能重复起音。
        if object.path.is_some()
            && policy.classic_sliders
            && event.node_id.is_none()
            && event.maximum == HitResult::Great
        {
            continue;
        }
        let source = &object.source;
        let sample_time = event.sample_time;
        if let Some(node) = event.node_id.filter(|_| source.hit_type & 2 != 0) {
            let node = &object.nodes[node];
            if node.kind == NodeKind::Tick {
                let head = head_sample(&source.samples, beatmap, source.start_time);
                builder.push_named(NamedEvent {
                    bank: head.bank,
                    name: "slidertick",
                    custom_bank: head.custom_bank,
                    volume: head.volume,
                    start_ms: sample_time,
                    duration_ms: 0.0,
                    looping: false,
                });
            } else if let Some(samples) = source
                .slider_edge_samples
                .get(node.edge - 1)
                .filter(|s| !s.is_empty())
            {
                builder.push_samples(samples, beatmap, sample_time, 0.0);
            } else {
                push_default_samples(
                    &mut builder,
                    beatmap,
                    source
                        .slider_edge_hitsounds
                        .get(node.edge)
                        .copied()
                        .unwrap_or(source.hitsound),
                    sample_time,
                );
            }
        } else if event.result == HitResult::LargeBonus {
            let head = head_sample(
                &source.samples,
                beatmap,
                object.end.round_ties_even() as i64,
            );
            builder.push_named(NamedEvent {
                bank: head.bank,
                name: "spinnerbonus",
                custom_bank: head.custom_bank,
                volume: head.volume,
                start_ms: sample_time,
                duration_ms: 0.0,
                looping: false,
            });
        } else {
            let mask = if source.hit_type & 2 != 0 {
                source
                    .slider_edge_hitsounds
                    .first()
                    .copied()
                    .unwrap_or(source.hitsound)
            } else {
                source.hitsound
            };
            push_declared_samples(&mut builder, &source.samples, mask, beatmap, sample_time);
        }
        // 取样参数由节点时刻决定，播放时刻由实际判定决定。尾音由父滑条在真实尾部播放。
        let play_time = if tail { sample_time } else { event.time };
        for mut sample in builder.finish().events {
            sample.start_ms += play_time - sample_time;
            if gameplay
                .failure_time
                .is_none_or(|end| sample.start_ms <= end)
            {
                result.events.push(sample);
            }
        }
    }
    for (id, object) in prepared.objects.iter().enumerate() {
        let head = head_sample(&object.source.samples, beatmap, object.source.start_time);
        let state = &gameplay.objects[id];
        let stop = gameplay
            .failure_time
            .map_or(object.end, |end| end.min(object.end));
        if object.path.is_some() {
            for (i, &(start, tracking)) in state.tracking.iter().enumerate() {
                let end = state
                    .tracking
                    .get(i + 1)
                    .map_or(stop, |next| next.0.min(stop));
                if !tracking || end <= start {
                    continue;
                }
                let mut builder = TimelineBuilder::new(resolver);
                let normal = object
                    .source
                    .samples
                    .iter()
                    .find(|s| s.addition == crate::model::HitAddition::None);
                if let Some(normal) = normal {
                    builder.push_transformed_samples(
                        std::slice::from_ref(normal),
                        "sliderslide",
                        head,
                        start,
                        end - start,
                        true,
                    );
                } else {
                    builder.push_named(NamedEvent {
                        bank: head.bank,
                        name: "sliderslide",
                        custom_bank: head.custom_bank,
                        volume: head.volume,
                        start_ms: start,
                        duration_ms: end - start,
                        looping: true,
                    });
                }
                if object
                    .source
                    .slider_edge_hitsounds
                    .first()
                    .copied()
                    .unwrap_or(object.source.hitsound)
                    & 2
                    != 0
                {
                    builder.push_named(NamedEvent {
                        bank: head.bank,
                        name: "sliderwhistle",
                        custom_bank: head.custom_bank,
                        volume: head.volume,
                        start_ms: start,
                        duration_ms: end - start,
                        looping: true,
                    });
                }
                result.events.extend(builder.finish().events);
            }
        } else if object.source.hit_type & 8 != 0 {
            let (required, maximum) = crate::replay::calculate::standard::spinner::required(
                object.end - object.source.start_time as f64,
                prepared.od,
            );
            let mut phase = 0.0;
            for pair in state.rotation.windows(2) {
                let start = pair[0].0;
                let end = pair[1].0.min(stop);
                if end <= start {
                    continue;
                }
                let turns = pair[0].2 as f64 / 360.0;
                let next = pair[1].2 as f64 / 360.0;
                if (next as u32) > maximum && (next as u32) > (turns as u32) {
                    let mut builder = TimelineBuilder::new(resolver);
                    builder.push_named(NamedEvent {
                        bank: head.bank,
                        name: "spinnerbonus-max",
                        custom_bank: head.custom_bank,
                        volume: head.volume,
                        start_ms: end,
                        duration_ms: 0.0,
                        looping: false,
                    });
                    result.events.extend(builder.finish().events);
                }
                let frequency = |turns: f64| {
                    (20000.0 / 44100.0
                        + 40000.0 / 44100.0
                            * if required == 0 {
                                1.0
                            } else {
                                turns / required as f64
                            })
                    .min(100000.0 / 44100.0)
                };
                let rate = frequency(turns);
                let mut freq = PlayFrequency::ramp(
                    rate,
                    (frequency(next) - rate).max(0.0) / (end - start),
                    100000.0 / 44100.0,
                );
                freq.phase = phase;
                if state.spinning_at(start) {
                    let mut builder = TimelineBuilder::new(resolver);
                    builder.push_spinner(
                        head.bank,
                        head.custom_bank,
                        head.volume,
                        start,
                        end - start,
                        freq,
                    );
                    result.events.extend(builder.finish().events);
                    phase += freq.integral(end - start);
                } else {
                    // 官方 Stop/Play 在重新开始旋转时从样本起点重播；分段连续播放才保留相位。
                    phase = 0.0;
                }
            }
        }
    }
    result
        .events
        .sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    if let Some(stop) = gameplay.failure_time {
        let fail = resolver.resolve(std::iter::once(b"failsound".as_slice()));
        // 失败同时截断尚在发声的一次性命中音；失败提示自身允许播完，歌曲属于另一时间线。
        result.events.retain_mut(|event| {
            if Some(event.source_id) == fail {
                return true;
            }
            if event.start_ms >= stop {
                return false;
            }
            let remaining = stop - event.start_ms;
            event.duration_ms = if event.duration_ms > 0.0 {
                event.duration_ms.min(remaining)
            } else {
                remaining
            };
            true
        });
    }
    result
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
