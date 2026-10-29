//! Catch 回放音效只消费成功接取及断连、失败事件；小水滴不重复生成普通水滴样本。
use super::{
    common::*,
    timeline::{NamedEvent, TimelineBuilder},
    *,
};
use crate::render::cpu::modes::catch::objects::ObjType;
use crate::{
    gameplay::{events::EventKind, timeline::GameplayTimeline},
    replay::preparation::catch::PreparedCatch,
};
pub fn build_timeline(
    prepared: &PreparedCatch,
    gameplay: &GameplayTimeline,
    library: &SampleLibrary,
) -> HitsoundTimeline {
    let mut resolver = library;
    build_with(prepared, gameplay, &mut resolver)
}
pub fn referenced_names(prepared: &PreparedCatch, gameplay: &GameplayTimeline) -> Vec<String> {
    let mut collector = CollectNames::default();
    build_with(prepared, gameplay, &mut collector);
    collector.into_names()
}
fn build_with<R: SampleResolver>(
    prepared: &PreparedCatch,
    gameplay: &GameplayTimeline,
    resolver: &mut R,
) -> HitsoundTimeline {
    let beatmap = &prepared.beatmap;
    let sources = beatmap.hit_objects.as_catch().unwrap();
    let mut result = HitsoundTimeline::default();
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
        } else if event.result.is_hit() {
            let object = &prepared.objects[event.object_id];
            let source = &sources[object.source_id];
            let head = head_sample(&source.samples, beatmap, source.start_time);
            match object.object_type {
                ObjType::Banana => builder.push_at(
                    [b"metronomelow".as_slice(), b"catch-banana".as_slice()].into_iter(),
                    100,
                    event.time,
                    0.0,
                    false,
                ),
                ObjType::TinyDroplet => (),
                ObjType::Droplet => {
                    if source.samples.is_empty() {
                        for _ in 0..=crate::domain::models::HitAddition::all_from_hitsound(
                            source.hitsound,
                        )
                        .count()
                        {
                            builder.push_named(NamedEvent {
                                bank: head.bank,
                                name: "slidertick",
                                custom_bank: head.custom_bank,
                                volume: head.volume,
                                start_ms: event.time,
                                duration_ms: 0.0,
                                looping: false,
                            });
                        }
                    } else {
                        builder.push_transformed_samples(
                            &source.samples,
                            "slidertick",
                            head,
                            event.time,
                            0.0,
                            false,
                        );
                    }
                }
                ObjType::Fruit => {
                    if source.hit_type & 2 == 0 || event.time == source.start_time as f64 {
                        push_declared_samples(
                            &mut builder,
                            &source.samples,
                            source.hitsound,
                            beatmap,
                            event.time,
                        );
                    } else {
                        let spans = source.slider_repeats.max(1) as usize;
                        let duration = (source.end_time - source.start_time) as f64 / spans as f64;
                        let node = ((event.time - source.start_time as f64) / duration)
                            .round()
                            .clamp(1.0, spans as f64) as usize;
                        match source.slider_edge_samples.get(node - 1) {
                            Some(samples) if !samples.is_empty() => {
                                builder.push_samples(samples, beatmap, event.time, 0.0)
                            }
                            _ => push_default_samples(
                                &mut builder,
                                beatmap,
                                source
                                    .slider_edge_hitsounds
                                    .get(node)
                                    .copied()
                                    .unwrap_or(source.hitsound),
                                event.time,
                            ),
                        }
                    }
                }
            }
        }
        result.events.extend(builder.finish().events);
    }
    result
        .events
        .sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        gameplay::{engine::compile_catch, events::HitResult},
        replay::{parse_osr, preparation::catch::prepare_replay},
    };
    use std::sync::Arc;

    #[test]
    fn only_caught_objects_play_samples_and_early_end_stops_player_sounds() {
        let map = crate::parse_beatmap_bytes(include_bytes!(
            "../../../osu-beatmap-preview-web/test/fixtures/replay-catch.osu"
        ))
        .unwrap();
        let original = parse_osr(include_bytes!(
            "../../../osu-beatmap-preview-web/test/fixtures/replay-catch.osr"
        ))
        .unwrap();
        let library = crate::hitsound::test_support::library_with(&[
            "normal-hitnormal",
            "normal-hitclap",
            "normal-slidertick",
            "catch-banana",
            "combobreak",
            "failsound",
        ]);
        for fail in [false, true] {
            let mut document = original.clone();
            if fail {
                document.frames.retain(|f| f.time_ms <= 1500);
            }
            let prepared = prepare_replay(&map, &document).unwrap();
            let gameplay = compile_catch(&prepared, Arc::new(document)).unwrap();
            let timeline = build_timeline(&prepared, &gameplay, &library);
            assert!(timeline.events.iter().any(|e| e.start_ms == 1000.0
                && library.name_of(e.source_id) == Some("normal-hitnormal")));
            assert!(!timeline.events.iter().any(|e| e.start_ms == 1500.0
                && library.name_of(e.source_id) == Some("normal-hitnormal")));
            assert_eq!(
                timeline
                    .events
                    .iter()
                    .filter(|e| library.name_of(e.source_id) == Some("failsound"))
                    .count(),
                usize::from(fail)
            );
            if fail {
                assert!(timeline.events.iter().all(|e| e.start_ms <= 1500.0));
            } else {
                for event in &gameplay.events {
                    if event.kind == EventKind::Judgement && event.result == HitResult::LargeTickHit
                    {
                        assert!(timeline.events.iter().any(|e| e.start_ms == event.time
                            && library.name_of(e.source_id) == Some("normal-slidertick")));
                    }
                }
            }
        }
    }
}
