//! osu!catch 的打击音事件：水果与果汁流（juice stream）。

use crate::domain::models::{Beatmap, CatchHitObject, HitAddition};

use super::common::{head_sample, push_declared_samples, slider_timing};
use super::sample::SampleResolver;
use super::timeline::TimelineBuilder;

pub(super) fn push_catch<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    object: &CatchHitObject,
    beatmap: &Beatmap,
) {
    let (head_bank, head_volume) = head_sample(&object.samples, beatmap, object.start_time);
    push_declared_samples(builder, &object.samples, object.hitsound, beatmap, object.start_time);

    if object.hit_type & 2 == 0 {
        return;
    }

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
        if object.samples.is_empty() {
            builder.push_named(head_bank, "slidertick", head_volume, time, 0.0, false);
            for _ in HitAddition::all_from_hitsound(object.hitsound) {
                builder.push_named(head_bank, "slidertick", head_volume, time, 0.0, false);
            }
        } else {
            builder.push_transformed_samples(
                &object.samples,
                beatmap,
                "slidertick",
                time,
                0.0,
                false,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{HitObjects, HitSample, SampleBank};
    use crate::hitsound::build_timeline;
    use crate::hitsound::test_support::{beatmap_with, library_with};

    #[test]
    fn 香蕉使用独立查找名() {
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
}
