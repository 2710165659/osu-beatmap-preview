//! osu!catch 的打击音事件：水果与果汁流（juice stream）。

use crate::domain::models::{Beatmap, CatchHitObject, HitAddition};

use super::common::{head_sample, push_declared_samples, push_default_samples, slider_timing};
use super::sample::SampleResolver;
use super::timeline::TimelineBuilder;

pub(super) fn push_catch<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    object: &CatchHitObject,
    beatmap: &Beatmap,
) {
    let head = head_sample(&object.samples, beatmap, object.start_time);
    push_declared_samples(
        builder,
        &object.samples,
        object.hitsound,
        beatmap,
        object.start_time as f64,
    );

    if object.hit_type & 2 == 0 {
        return;
    }

    // 果汁流的头 / 重复箭头 / 尾部都是「水果」，各自用**自己时刻**的 timing point 补齐参数，
    // 位掩码取自该节点的 `edgeSounds`（osu! `JuiceStream` 的 `GetNodeSamples(nodeIndex++)`）。
    let spans = object.slider_repeats.max(1) as usize;
    let span_duration = (object.end_time - object.start_time) as f64 / spans as f64;

    // 果汁流：小果与节点都使用 `slidertick`，时间规则与滑条 tick 一致。
    let (beat_length, slider_velocity) = slider_timing(object.start_time, beatmap);
    let slider_multiplier = beatmap.difficulty.get_f64_or("SliderMultiplier", 1.4);
    let tick_rate = beatmap.difficulty.get_f64_or("SliderTickRate", 1.0);
    let times = crate::render::cpu::modes::standard::slider::slider_tick_times(
        object.slider_pixel_length,
        object.start_time,
        object.end_time,
        object.slider_repeats,
        beat_length,
        slider_velocity,
        tick_rate,
        slider_multiplier,
    );
    for time in times {
        // 果汁流的每个小果都把头部样本名替换为 slidertick，保留所有层和音量。
        // 参数取自头部（osu! `JuiceStream` 的 `dropletSamples = Samples.With("slidertick")`），
        // 而不是小果所在时刻的 timing point。
        if object.samples.is_empty() {
            builder.push_named(
                head.bank,
                "slidertick",
                head.custom_bank,
                head.volume,
                time,
                0.0,
                false,
            );
            for _ in HitAddition::all_from_hitsound(object.hitsound) {
                builder.push_named(
                    head.bank,
                    "slidertick",
                    head.custom_bank,
                    head.volume,
                    time,
                    0.0,
                    false,
                );
            }
        } else {
            builder.push_transformed_samples(
                &object.samples,
                "slidertick",
                head,
                time,
                0.0,
                false,
            );
        }
    }

    for span in 1..=spans {
        let time = object.start_time as f64 + span as f64 * span_duration;
        match object.slider_edge_samples.get(span - 1) {
            Some(edge) if !edge.is_empty() => builder.push_samples(edge, beatmap, time, 0.0),
            _ => push_default_samples(
                builder,
                beatmap,
                object
                    .slider_edge_hitsounds
                    .get(span)
                    .copied()
                    .unwrap_or(object.hitsound),
                time,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{HitObjects, HitSample, SampleBank};
    use crate::hitsound::build_timeline;
    use crate::hitsound::test_support::{beatmap_with, library_with};

    /// 香蕉使用独立的查找名。
    #[test]
    fn banana_uses_dedicated_lookup_name() {
        let library = library_with(&["catch-banana"]);
        let beatmap = beatmap_with(
            2,
            HitObjects::Catch(vec![CatchHitObject {
                x: 0,
                y: 0,
                start_time: 1000,
                end_time: 1000,
                hit_type: 1,
                samples: vec![HitSample::new(
                    // 音效组不是 Custom：此时才会先查 `bank-name`，再回退到裸名字。
                    SampleBank::Normal,
                    HitAddition::None,
                    100,
                    Some("catch-banana".to_string()),
                )],
                ..Default::default()
            }]),
        );
        let timeline = build_timeline(&beatmap, &library);
        assert!(library.contains("catch-banana"));
        // 自定义文件名优先于 timing point 的默认音效：名字必须原样命中，不能被换成 hitnormal。
        assert!(!timeline.is_empty(), "香蕉应产生事件");
        assert_eq!(
            library.name_of(timeline.events[0].source_id),
            Some("catch-banana")
        );
    }

    /// 果汁流的尾部按节点时刻发声。
    #[test]
    fn juice_stream_tail_sounds_at_node_time() {
        // 头 / 重复箭头 / 尾部都是水果：尾部按自己时刻的 timing point 取音量，谱面常用
        // 「在果汁流尾部插入低音量绿线」把它压掉。
        let library = library_with(&["soft-hitnormal"]);
        let mut beatmap = beatmap_with(
            2,
            HitObjects::Catch(vec![CatchHitObject {
                x: 0,
                y: 0,
                start_time: 1000,
                end_time: 2000,
                hit_type: 2,
                slider_type: Some("L".to_string()),
                slider_points: vec![(100, 0)],
                slider_repeats: 1,
                // 路径足够短：不产生小果，只留头 / 尾两个水果。
                slider_pixel_length: 10.0,
                ..Default::default()
            }]),
        );
        beatmap.timing_points = vec![
            crate::hitsound::test_support::timing_point(0.0, 2, 95),
            crate::hitsound::test_support::timing_point(2000.0, 2, 5),
        ];

        let timeline = build_timeline(&beatmap, &library);
        assert_eq!(timeline.len(), 2, "events={:?}", timeline.events);
        assert!((timeline.events[0].start_ms - 1000.0).abs() < 1e-9);
        assert!((timeline.events[0].gain - 0.95).abs() < 1e-9);
        assert!((timeline.events[1].start_ms - 2000.0).abs() < 1e-9);
        assert!((timeline.events[1].gain - 0.05).abs() < 1e-9);
    }
}
