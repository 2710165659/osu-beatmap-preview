//! osu!mania 的打击音事件：原生长条只在头部发声。

use crate::domain::models::{Beatmap, ManiaHitObject};

use super::common::push_declared_samples;
use super::sample::SampleResolver;
use super::timeline::TimelineBuilder;

pub(super) fn push_mania<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    object: &ManiaHitObject,
    beatmap: &Beatmap,
) {
    // 原生 mania 长条只在头部播放样本；持续滑行音只属于转换生成的 hold。
    push_declared_samples(
        builder,
        &object.samples,
        0,
        beatmap,
        object.start_time as f64,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::HitObjects;
    use crate::hitsound::build_timeline;
    use crate::hitsound::test_support::{beatmap_with, library_with};

    /// mania 按 timing point 的音效组发声。
    #[test]
    fn mania_uses_timing_point_sample_bank() {
        let mut beatmap = beatmap_with(
            3,
            HitObjects::Mania(vec![ManiaHitObject {
                lane: 0,
                start_time: 1000,
                end_time: 1000,
                is_long_note: false,
                samples: Vec::new(),
            }]),
        );
        beatmap.timing_points[0].sample_set = crate::domain::models::SAMPLE_SET_DRUM;
        beatmap.timing_points[0].sample_volume = 80;

        let library = library_with(&["drum-hitnormal"]);
        let timeline = build_timeline(&beatmap, &library);
        assert_eq!(timeline.len(), 1);
        // 采样音量 80 → 0.8 线性增益
        let expected = 0.8;
        assert!(
            (timeline.events[0].gain - expected).abs() < 1e-12,
            "gain={} expected={}",
            timeline.events[0].gain,
            expected
        );
    }
}
