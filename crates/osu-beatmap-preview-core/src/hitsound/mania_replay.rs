//! Mania 回放按实际成功头判定发声；长条尾不重复播放头部样本。
use super::{common::*, timeline::TimelineBuilder, *};
use crate::{
    gameplay::{events::EventKind, timeline::GameplayTimeline},
    replay::preparation::mania::PreparedMania,
};

pub fn build_timeline(
    prepared: &PreparedMania,
    gameplay: &GameplayTimeline,
    library: &SampleLibrary,
) -> HitsoundTimeline {
    let mut resolver = library;
    build_with(prepared, gameplay, &mut resolver)
}
pub fn referenced_names(prepared: &PreparedMania, gameplay: &GameplayTimeline) -> Vec<String> {
    let mut resolver = CollectNames::default();
    build_with(prepared, gameplay, &mut resolver);
    resolver.into_names()
}
fn build_with<R: SampleResolver>(
    prepared: &PreparedMania,
    gameplay: &GameplayTimeline,
    resolver: &mut R,
) -> HitsoundTimeline {
    let mut output = HitsoundTimeline::default();
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
            output.events.extend(builder.finish().events);
        } else if event.node_id.is_none() && event.result.is_hit() {
            let source = &prepared.objects[event.object_id];
            push_declared_samples(
                &mut builder,
                &source.samples,
                0,
                &prepared.beatmap,
                source.start_time as f64,
            );
            for mut sound in builder.finish().events {
                sound.start_ms += event.time - source.start_time as f64;
                output.events.push(sound);
            }
        }
    }
    output
        .events
        .sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hitsound::test_support::library_with, replay::preparation::mania::tests::fixture};
    #[test]
    fn samples_follow_actual_heads_with_silent_misses_and_tails() {
        let (prepared, document) = fixture(
            "64,192,1000,128,0,2000:0:0:0:0:\n192,192,2500,1,0\n320,192,3000,1,0",
            &[(0, 0), (1030, 1), (2000, 0), (3100, 4), (3400, 0)],
        );
        let gameplay = crate::gameplay::engine::compile_mania(&prepared, document).unwrap();
        let library = library_with(&["normal-hitnormal", "combobreak"]);
        let timeline = build_timeline(&prepared, &gameplay, &library);
        let times: Vec<_> = timeline.events.iter().map(|event| event.start_ms).collect();
        assert!(times.contains(&1030.0));
        assert!(times.contains(&3100.0));
        assert!(!times.contains(&2000.0));
        assert!(!times.contains(&2500.0));
        assert_eq!(times.len(), 3, "两个成功头判定与一次断连音效");
    }
}
