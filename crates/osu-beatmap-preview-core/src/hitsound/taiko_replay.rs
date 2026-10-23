//! Taiko 回放音效：实际按键只发声一次，成功第二击/强滚奏追加 strong 音层。
use super::{
    common::*,
    timeline::{NamedEvent, TimelineBuilder},
    *,
};
use crate::{
    gameplay::{
        events::{EventKind, HitResult},
        timeline::GameplayTimeline,
    },
    replay::preparation::taiko::{PreparedTaiko, TaikoObjectKind},
};

pub fn build_timeline(
    prepared: &PreparedTaiko,
    gameplay: &GameplayTimeline,
    library: &SampleLibrary,
) -> HitsoundTimeline {
    let mut resolver = library;
    build_with(prepared, gameplay, &mut resolver)
}
pub fn referenced_names(prepared: &PreparedTaiko, gameplay: &GameplayTimeline) -> Vec<String> {
    let mut resolver = CollectNames::default();
    build_with(prepared, gameplay, &mut resolver);
    resolver.into_names()
}
fn build_with<R: SampleResolver>(
    prepared: &PreparedTaiko,
    gameplay: &GameplayTimeline,
    resolver: &mut R,
) -> HitsoundTimeline {
    let beatmap = &prepared.beatmap;
    let mut result = HitsoundTimeline::default();
    for press in &gameplay.taiko_inputs {
        let mut builder = TimelineBuilder::new(resolver);
        let matched = press.object_id.and_then(|id| {
            gameplay
                .events_between(press.time - 0.001, press.time)
                .iter()
                .find(|e| {
                    e.kind == EventKind::Judgement
                        && e.object_id == id
                        && e.result.is_hit()
                        && (e.node_id == press.node_id || e.maximum == HitResult::Great)
                })
        });
        let object = matched.map(|e| &prepared.objects[e.object_id]);
        let samples = object.map_or(&[][..], |o| o.source.samples.as_slice());
        let sample_time = matched.map_or(press.time, |e| e.sample_time);
        let strong = matched.is_some_and(|e| {
            e.result == HitResult::LargeBonus && e.node_id.is_some()
                || e.result == HitResult::SmallBonus && object.is_some_and(|o| o.strong())
        });
        if strong {
            taiko::push_taiko_strong(
                &mut builder,
                samples,
                beatmap,
                sample_time,
                press.key & 10 != 0,
            );
        } else {
            taiko::push_taiko_press(
                &mut builder,
                samples,
                beatmap,
                sample_time,
                press.key & 10 != 0,
            );
        }
        for mut event in builder.finish().events {
            event.start_ms += press.time - sample_time;
            result.events.push(event);
        }
    }
    for event in &gameplay.events {
        let mut builder = TimelineBuilder::new(resolver);
        if event.kind != EventKind::Judgement {
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
        } else if event.result == HitResult::LargeBonus
            && event.node_id.is_none()
            && matches!(
                prepared.objects[event.object_id].kind,
                TaikoObjectKind::Swell { .. }
            )
        {
            let sample = head_sample(
                &prepared.objects[event.object_id].source.samples,
                beatmap,
                event.sample_time as i64,
            );
            builder.push_named(NamedEvent {
                bank: sample.bank,
                name: "spinnerbonus",
                custom_bank: sample.custom_bank,
                volume: sample.volume,
                start_ms: event.time,
                duration_ms: 0.0,
                looping: false,
            });
        }
        result.events.extend(builder.finish().events);
    }
    result
        .events
        .sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    result
}
