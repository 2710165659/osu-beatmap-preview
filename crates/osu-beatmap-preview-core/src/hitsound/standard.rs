//! osu!standard 的打击音事件：物件头、滑条与转盘。

use crate::domain::models::{Beatmap, HitAddition, SampleBank, StandardHitObject};
use crate::render::cpu::modes::catch::objects::difficulty_range;

use super::common::{head_sample, push_declared_samples, push_declared_samples_at, slider_timing};
use super::sample::SampleResolver;
use super::timeline::{PlayFrequency, TimelineBuilder};

pub(super) fn push_standard<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    object: &StandardHitObject,
    beatmap: &Beatmap,
) {
    let (head_bank, head_volume) = head_sample(&object.samples, beatmap, object.start_time);
    if object.hit_type & 2 != 0 {
        push_declared_samples(builder, &object.samples, object.hitsound, beatmap, object.start_time);
        push_standard_slider(builder, object, beatmap, head_bank, head_volume);
    } else if object.hit_type & 8 != 0 {
        push_standard_spinner(builder, object, beatmap, head_bank, head_volume);
    } else {
        push_declared_samples(builder, &object.samples, object.hitsound, beatmap, object.start_time);
    }
}

/// osu! autoplay 的转盘转速：`OsuAutoGenerator` 每毫秒 0.05 弧度（约 477 RPM）。
///
/// 预览无法预知玩家表现，所以统一按 autoplay 的固定转速推算旋转进度、奖励圈与音高。
const SPINNER_AUTO_REVOLUTIONS_PER_MS: f64 = 0.05 / (2.0 * std::f64::consts::PI);

/// `DrawableSpinner` 的旋转音频率调制常量（取自 osu-stable `AudioEngine`）。
const SPINNING_SAMPLE_BASE_FREQUENCY: f64 = 20_000.0 / 44_100.0;
const SPINNING_SAMPLE_MODULATION_RATIO: f64 = 40_000.0 / 44_100.0;
const SPINNING_SAMPLE_MAX_FREQUENCY: f64 = 100_000.0 / 44_100.0;

/// `Spinner` 的奖励圈间隔：转满 `SpinsRequired + 2` 圈后每一圈给一次奖励。
const SPINNER_BONUS_SPINS_GAP: i32 = 2;

/// 转盘的旋转圈数最多按 10 分钟计算：损坏谱面的超长时长会让事件生成卡死，
/// 而 10 分钟的转盘在预览里不存在。
const SPINNER_MAX_ROTATION_MS: f64 = 600_000.0;

/// 转盘：旋转循环音（音高随旋转进度升高）+ 奖励音 + 结束时的判定音。
///
/// 规则与 osu! 一致：
/// - `Spinner.ApplyDefaultsToSelf` 按 OD 换算「清关所需圈数」与「上限圈数」；
/// - `Spinner.CreateNestedHitObjects` 生成 `SpinsRequired + 2 + MaximumBonusSpins` 个圈，
///   前 `SpinsRequired + 2` 个是计分圈（不发声），其余是奖励圈（`spinnerbonus`）；
/// - 超出上限的圈数用 `spinnerbonus-max`（`DrawableSpinner.updateBonusScore`）；
/// - `DrawableSpinner.Update` 用 `progressUnclamped`（已转圈数 / 所需圈数）调制旋转音频率；
/// - 转盘自身的判定音在判定成立（`ArmedState.Hit`）时才播，也就是结束处，而不是出现时。
///
/// 这些圈的**发声时刻**由实际旋转决定（每转满一圈响一次），而不是取 `CreateNestedHitObjects`
/// 里那种「按时长均分」的占位 StartTime，因此这里按 autoplay 的转速换算时刻。
fn push_standard_spinner<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    object: &StandardHitObject,
    beatmap: &Beatmap,
    head_bank: SampleBank,
    head_volume: i32,
) {
    let start = object.start_time as f64;
    let end = object.end_time as f64;
    let duration = (object.end_time - object.start_time).max(0) as f64;
    // 时长为 0 的转盘没有任何旋转时间：发声只会变成一个永不停止的循环音。
    if duration <= 0.0 {
        return;
    }
    let overall_difficulty = beatmap.difficulty.get_f64_or("OverallDifficulty", 5.0);

    // lazer 为浮点误差留的容差，必须保留才能与游戏内的圈数完全一致。
    const DURATION_ERROR: f64 = 0.0001;
    let seconds = duration / 1000.0;
    let spins_required = (difficulty_range(overall_difficulty, 90.0, 150.0, 225.0) / 60.0 * seconds
        + DURATION_ERROR) as i32;
    let maximum_spins = (difficulty_range(overall_difficulty, 250.0, 380.0, 430.0) / 60.0 * seconds
        + DURATION_ERROR) as i32;
    let maximum_bonus_spins = (maximum_spins - spins_required - SPINNER_BONUS_SPINS_GAP).max(0);
    let total_spins = maximum_bonus_spins + spins_required + SPINNER_BONUS_SPINS_GAP;
    let spins_required_for_bonus = spins_required + SPINNER_BONUS_SPINS_GAP;

    // 旋转进度随 autoplay 匀速增长，所以频率是线性的：base + 进度 × ratio，上限封顶。
    // `SpinsRequired == 0`（短到没有整数圈的转盘）在 osu! 里直接算作完成，进度恒为 1。
    let (start_frequency, per_ms) = if spins_required > 0 {
        (
            SPINNING_SAMPLE_BASE_FREQUENCY,
            SPINNING_SAMPLE_MODULATION_RATIO * SPINNER_AUTO_REVOLUTIONS_PER_MS
                / spins_required as f64,
        )
    } else {
        (
            SPINNING_SAMPLE_BASE_FREQUENCY + SPINNING_SAMPLE_MODULATION_RATIO,
            0.0,
        )
    };
    builder.push_spinner(
        head_bank,
        head_volume,
        start,
        duration,
        PlayFrequency::ramp(start_frequency, per_ms, SPINNING_SAMPLE_MAX_FREQUENCY),
    );

    // 第 i 圈（0 基）在起始后 `(i + 1) / 转速` 毫秒转满；超过转盘时长的圈数不会发生。
    let rotation_cap =
        (SPINNER_AUTO_REVOLUTIONS_PER_MS * duration.min(SPINNER_MAX_ROTATION_MS)).ceil() as i64 + 1;
    for index in 0..rotation_cap {
        let time = start + (index + 1) as f64 / SPINNER_AUTO_REVOLUTIONS_PER_MS;
        if time >= end {
            break;
        }
        if index >= total_spins as i64 {
            builder.push_named(head_bank, "spinnerbonus-max", head_volume, time, 0.0, false);
        } else if index >= spins_required_for_bonus as i64 {
            builder.push_named(head_bank, "spinnerbonus", head_volume, time, 0.0, false);
        }
    }

    // 判定音：`DrawableHitObject` 只在 `ArmedState.Hit` 时 `PlaySamples()`，而转盘的判定
    // 成立在结束处（`CheckForResult` 在 `Time.Current < EndTime` 时直接返回），所以它响在
    // 转盘结束而不是出现时。音效参数仍取转盘起始处的 timing point。
    push_declared_samples_at(
        builder,
        &object.samples,
        object.hitsound,
        beatmap,
        object.start_time,
        end,
    );
}

fn push_standard_slider<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    object: &StandardHitObject,
    beatmap: &Beatmap,
    head_bank: SampleBank,
    head_volume: i32,
) {
    let (beat_length, slider_velocity) = slider_timing(object.start_time, beatmap);
    let slider_multiplier = beatmap.difficulty.get_f64_or("SliderMultiplier", 1.4);
    let tick_rate = beatmap.difficulty.get_f64_or("SliderTickRate", 1.0);

    // 滑行音：按住滑条期间循环播放，继承头部普通音的音量。
    if object.end_time > object.start_time {
        let duration = (object.end_time - object.start_time) as f64;
        if let Some(normal) = object
            .samples
            .iter()
            .find(|sample| sample.addition == HitAddition::None)
        {
            builder.push_transformed_samples(
                std::slice::from_ref(normal),
                beatmap,
                "sliderslide",
                object.start_time as f64,
                duration,
                true,
            );
        } else {
            builder.push_named(
                head_bank,
                "sliderslide",
                head_volume,
                object.start_time as f64,
                duration,
                true,
            );
        }
        // osu! 只把头部的 whistle 复制为 sliderwhistle，其他加成音不参与滑行循环。
        if object.hitsound & 2 != 0
            || object
                .samples
                .iter()
                .any(|sample| sample.addition == HitAddition::Whistle)
        {
            if let Some(whistle) = object
                .samples
                .iter()
                .find(|sample| sample.addition == HitAddition::Whistle)
            {
                builder.push_transformed_samples(
                    std::slice::from_ref(whistle),
                    beatmap,
                    "sliderwhistle",
                    object.start_time as f64,
                    duration,
                    true,
                );
            } else {
                builder.push_named(
                    head_bank,
                    "sliderwhistle",
                    head_volume,
                    object.start_time as f64,
                    duration,
                    true,
                );
            }
        }
    }

    // 滑条 tick：使用滑条头的音效组与音量。
    let tick_times = crate::render::cpu::modes::standard::slider::slider_tick_times(
        object.slider_pixel_length,
        object.start_time,
        object.end_time,
        object.slider_repeats,
        beat_length,
        slider_velocity,
        tick_rate,
        slider_multiplier,
    );
    for time in tick_times {
        builder.push_named(head_bank, "slidertick", head_volume, time, 0.0, false);
    }

    // 重复箭头与滑条尾：使用各节点的自定义音效。
    let spans = object.slider_repeats.max(1) as usize;
    let span_duration = (object.end_time - object.start_time) as f64 / spans as f64;
    // edgeSets[1..] 对应重复节点和尾节点，尾节点在没有重复时也必须发声。
    for span in 1..=spans {
        let time = object.start_time as f64 + span as f64 * span_duration;
        match object.slider_edge_samples.get(span - 1) {
            // 节点自带音效：直接使用。
            Some(edge) if !edge.is_empty() => builder.push_samples(edge, beatmap, time, 0.0),
            // 节点没有自带音效：沿用滑条头的完整 hitsound 位掩码。
            _ => {
                builder.push_named(head_bank, "hitnormal", head_volume, time, 0.0, false);
                for addition in HitAddition::all_from_hitsound(object.hitsound) {
                    builder.push_named(head_bank, addition.suffix(), head_volume, time, 0.0, false);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{HitObjects, HitSample};
    use crate::hitsound::build_timeline;
    use crate::hitsound::test_support::{beatmap_with, library_with, object_sample, spinner_beatmap};

    #[test]
    fn 物件缺省音效参数时回退到timing_point() {
        let library = library_with(&["soft-hitnormal", "drum-hitnormal"]);
        let mut beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![
                // 没有自带 hitSample：应当使用 timing point 的 soft 组与音量。
                StandardHitObject {
                    start_time: 1000,
                    end_time: 1000,
                    hit_type: 1,
                    hitsound: 0,
                    ..Default::default()
                },
                // 自带 hitSample：应当覆盖 timing point 的 soft 组。
                StandardHitObject {
                    start_time: 2000,
                    end_time: 2000,
                    hit_type: 1,
                    hitsound: 0,
                    samples: vec![HitSample::new(SampleBank::Drum, HitAddition::None, 40, None)],
                    ..Default::default()
                },
            ]),
        );
        beatmap.timing_points[0].sample_set = crate::domain::models::SAMPLE_SET_SOFT;

        let timeline = build_timeline(&beatmap, &library);
        assert_eq!(timeline.len(), 2);
        assert_eq!(library.name_of(timeline.events[0].source_id), Some("soft-hitnormal"));
        assert_eq!(library.name_of(timeline.events[1].source_id), Some("drum-hitnormal"));
    }

    #[test]
    fn 滑条生成滑行音与tick事件() {
        let library = library_with(&["normal-sliderslide", "normal-slidertick"]);
        let beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                x: 0,
                y: 0,
                start_time: 1000,
                end_time: 3000,
                hit_type: 2,
                hitsound: 0,
                slider_type: Some("L".to_string()),
                slider_points: vec![(100, 0)],
                slider_repeats: 1,
                slider_pixel_length: 300.0,
                samples: object_sample(SampleBank::Normal, 100),
                ..Default::default()
            }]),
        );
        let timeline = build_timeline(&beatmap, &library);
        // 滑条头本身没有可用样本，因此只应出现滑行音与 tick。
        let slide = timeline
            .events
            .iter()
            .find(|event| library.name_of(event.source_id) == Some("normal-sliderslide"))
            .expect("必须生成滑行音事件");
        assert!(slide.looping);
        assert!((slide.duration_ms - 2000.0).abs() < 1e-9);
        assert!(
            timeline
                .events
                .iter()
                .any(|event| library.name_of(event.source_id) == Some("normal-slidertick")),
            "必须生成滑条 tick 事件"
        );
    }

    #[test]
    fn std转盘按osu规则升调与发奖励音() {
        // 4 秒 OD5 的转盘：清关需要 150/60 × 4 = 10 圈，上限是 380/60 × 4 = 25 圈，
        // 因此前 12 圈（10 + 奖励间隔 2）是计分圈、第 13~25 圈是奖励圈，再往后的圈数用上限音。
        let library = library_with(&[
            "normal-hitnormal",
            "spinnerspin",
            "spinnerbonus",
            "spinnerbonus-max",
        ]);
        let timeline = build_timeline(&spinner_beatmap(0, 4000), &library);
        let events: Vec<(f64, &str)> = timeline
            .events
            .iter()
            .map(|event| {
                (
                    event.start_ms,
                    library.name_of(event.source_id).unwrap_or_default(),
                )
            })
            .collect();

        // autoplay 转速 0.05 rad/ms ≈ 477 RPM → 每圈 125.663706ms。
        let revolution_ms = 125.663_706_143_591_72;
        let first = timeline.events[0];
        assert_eq!(library.name_of(first.source_id), Some("spinnerspin"));
        assert!(first.looping, "旋转音必须是循环音");
        assert!((first.duration_ms - 4000.0).abs() < 1e-9);
        // `DrawableSpinner` 的频率调制：起步 20000/44100，比例 40000/44100，上限 100000/44100。
        assert!((first.frequency.start - 0.453_514_739).abs() < 1e-9);
        assert!((first.frequency.max - 2.267_573_696).abs() < 1e-9);
        // 10 圈（1256.64ms）走完进度 1，故斜率 = (40000/44100) ÷ 1256.64ms。
        assert!((first.frequency.per_ms - 0.000_721_792).abs() < 1e-9);

        assert_eq!(
            events.len(),
            1 + 13 + 6 + 1,
            "事件：旋转音 + 13 奖励 + 6 上限 + 判定音"
        );
        // 第一声奖励音在第 13 圈（`SpinsRequired + 2` 之后的第一圈），而不是转盘出现时。
        for (index, (time, name)) in events[1..14].iter().enumerate() {
            assert_eq!(*name, "spinnerbonus", "第 {index} 个奖励音");
            assert!(
                (time - (index as f64 + 13.0) * revolution_ms).abs() < 1e-6,
                "第 {index} 个奖励音时间 {time}"
            );
        }
        // 第 26 圈起超过奖励上限，改用 `spinnerbonus-max`。
        for (index, (time, name)) in events[14..20].iter().enumerate() {
            assert_eq!(*name, "spinnerbonus-max", "第 {index} 个上限音");
            assert!(
                (time - (index as f64 + 26.0) * revolution_ms).abs() < 1e-6,
                "第 {index} 个上限音时间 {time}"
            );
        }
        // 判定音落在转盘结束处（`DrawableHitObject` 只在判定成立时播样本）。
        assert_eq!(events[20], (4000.0, "normal-hitnormal"));
    }

    #[test]
    fn std转盘不产生超过时长的圈与零时长事件() {
        let library = library_with(&[
            "normal-hitnormal",
            "spinnerspin",
            "spinnerbonus",
            "spinnerbonus-max",
        ]);
        // 300ms 的转盘短到没有整数圈：osu! 直接算作完成，倍率恒定在 base + ratio。
        let timeline = build_timeline(&spinner_beatmap(0, 300), &library);
        let events: Vec<&str> = timeline
            .events
            .iter()
            .map(|event| library.name_of(event.source_id).unwrap_or_default())
            .collect();
        assert_eq!(events, vec!["spinnerspin", "normal-hitnormal"]);
        let frequency = timeline.events[0].frequency;
        assert_eq!(frequency.per_ms, 0.0);
        assert!((frequency.start - 60_000.0 / 44_100.0).abs() < 1e-9);

        // 时长为 0 的转盘没有任何旋转时间：一个事件都不该有（否则循环音永不停止）。
        let empty = build_timeline(&spinner_beatmap(1000, 1000), &library);
        assert!(empty.is_empty(), "events={:?}", empty.events);
    }
}
