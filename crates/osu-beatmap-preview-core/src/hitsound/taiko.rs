//! osu!taiko 的打击音事件：鼓点、连打（drumroll）与大连打（swell）。
//!
//! 走的是 legacy（classic 皮肤）路径：音效组取自物件 `hitSample` 与所在 timing point，
//! 鼓边按 `clap|whistle` 判定，strong 追加 `finish`/`whistle`。

use crate::domain::models::{Beatmap, HitAddition, HitSample, SampleBank, StandardHitObject, TaikoHitObject};
use crate::render::cpu::modes::catch::objects::difficulty_range;
use crate::render::cpu::modes::taiko::animation::generate_drum_roll_ticks;
use crate::render::cpu::modes::taiko::constants::{
    DRUMROLL_FLAG, HIT_SOUNDS_RIM, HIT_SOUNDS_STRONG, SWELL_FLAG,
};

use super::common::DefaultSample;
use super::sample::SampleResolver;
use super::timeline::TimelineBuilder;

/// taiko 一次敲击的取样参数：音效组 + 音量 + 自定义音效索引。
#[derive(Debug, Clone, Copy)]
struct TaikoSampleSpec {
    bank: SampleBank,
    volume: i32,
    custom_bank: i32,
}

/// 复现 osu! `HitObject.CreateHitSampleInfo` 的取样规则。
///
/// `name` 是目标样本名（`hitnormal` 或 `hitclap`/`hitwhistle`/`hitfinish`）：
/// - 非 `hitnormal` 时优先继承物件「第一个加成音样本」的音效组与音量，没有加成音才退回普通样本；
/// - `hitnormal` 时只用普通样本。
///
/// 物件没有自带 `hitSample`（或样本未指定音效组/音量）时回退到所在 timing point。
/// 注意这里刻意不按音量重选音效组：那是 osu! Argon 皮肤（`VolumeAwareHitSampleInfo`）的逻辑，
/// 本项目使用 classic 皮肤资源，走的是 legacy 查找路径。
fn taiko_sample_spec(samples: &[HitSample], beatmap: &Beatmap, time: i64, name: &str) -> TaikoSampleSpec {
    let normal = samples.iter().find(|sample| sample.addition == HitAddition::None);
    let addition = samples
        .iter()
        .find(|sample| sample.addition != HitAddition::None);
    let picked = if name == "hitnormal" {
        normal
    } else {
        addition.or(normal)
    };
    let fallback = DefaultSample::at(beatmap, time);
    let bank = match picked.map(|sample| sample.bank) {
        // `Auto` 表示 `.osu` 里没有指定音效组：用 timing point 的采样组。
        Some(SampleBank::Auto) | None => fallback.map_or(SampleBank::Normal, |default| default.bank),
        Some(bank) => bank,
    };
    let volume = picked
        .map(|sample| sample.volume)
        .filter(|volume| *volume > 0)
        .or_else(|| fallback.map(|default| default.volume))
        .unwrap_or(100);
    // 物件的自定义音效索引优先，其次沿用 timing point 的 `sampleIndex`。
    let custom_bank = picked
        .map(|sample| sample.custom_bank)
        .filter(|custom_bank| *custom_bank > 0)
        .or_else(|| fallback.map(|default| default.custom_bank))
        .unwrap_or(0);
    TaikoSampleSpec {
        bank,
        volume,
        custom_bank,
    }
}

/// 敲击一次鼓面：红音符（鼓心）用 `hitnormal`，蓝音符（鼓边）用 `hitclap`。
fn push_taiko_press<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    samples: &[HitSample],
    beatmap: &Beatmap,
    time_ms: f64,
    is_rim: bool,
) {
    let name = if is_rim { "hitclap" } else { "hitnormal" };
    let spec = taiko_sample_spec(samples, beatmap, time_ms as i64, name);
    builder.push_taiko(
        spec.bank,
        name,
        spec.custom_bank,
        spec.volume,
        time_ms,
        0.0,
        false,
    );
}

/// strong 敲击：同一时刻在 base 之上再叠一层 `hitwhistle`（蓝）/ `hitfinish`（红）。
///
/// osu! 的第二下敲击会先 `flushCenter/RimTriggerSources()` 掐掉第一下的 base，两声几乎同刻，
/// 因此这里只发「base + 追加音」两条事件，而不是 base 发两遍。
fn push_taiko_strong<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    samples: &[HitSample],
    beatmap: &Beatmap,
    time_ms: f64,
    is_rim: bool,
) {
    push_taiko_press(builder, samples, beatmap, time_ms, is_rim);
    let name = if is_rim { "hitwhistle" } else { "hitfinish" };
    let spec = taiko_sample_spec(samples, beatmap, time_ms as i64, name);
    builder.push_taiko(
        spec.bank,
        name,
        spec.custom_bank,
        spec.volume,
        time_ms,
        0.0,
        false,
    );
}

/// osu! `TaikoBeatmapConverter.RequiredSwellHitsPerSecond`：按 OD 换算大连打所需敲击数。
fn swell_hits_per_second(overall_difficulty: f64) -> f64 {
    difficulty_range(overall_difficulty, 3.0, 5.0, 7.5) * 1.65
}

/// 大连打（swell）：按 autoplay 的节奏交替敲鼓心/鼓边，与 osu! `TaikoAutoGenerator` 一致。
fn push_taiko_swell<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    object: &TaikoHitObject,
    beatmap: &Beatmap,
) {
    // 时长来自谱面，异常值（例如结束时间写成一个巨大浮点数）会让下面的换算溢出并生成天文数字的事件；
    // 大连打不可能有 10 分钟长，因此按 10 分钟封顶，只影响损坏谱面。
    let duration_ms = (object.end_time - object.start_time).clamp(0, 600_000) as f64;
    let overall_difficulty = beatmap.difficulty.get_f64_or("OverallDifficulty", 5.0);
    // 与 C# 的 `(int)Math.Max(1, duration / 1000 * hitsPerSecond)` 一致：先取至少 1，再截断。
    let required = (duration_ms / 1000.0 * swell_hits_per_second(overall_difficulty)).max(1.0) as i32;
    let hit_rate = 50.0_f64.min(duration_ms / required.max(1) as f64);
    if !hit_rate.is_finite() || hit_rate <= 0.0 {
        return;
    }
    let end_time = object.end_time as f64;
    let mut time = object.start_time as f64;
    for index in 0..required.max(1) {
        if time >= end_time {
            break;
        }
        // autoplay 依次敲 鼓心、鼓边、鼓心、鼓边……
        push_taiko_press(builder, &object.samples, beatmap, time, index % 2 == 1);
        time += hit_rate;
    }
}

pub(super) fn push_taiko_object<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    object: &TaikoHitObject,
    beatmap: &Beatmap,
) {
    if object.hit_type & DRUMROLL_FLAG != 0 {
        // 连打：每个 tick 敲一次鼓心，tick 序列与画面共用同一份 osu! 规则实现。
        let slider_tick_rate = beatmap.difficulty.get_f64_or("SliderTickRate", 1.0);
        for tick in generate_drum_roll_ticks(object, &beatmap.timing_points, slider_tick_rate) {
            push_taiko_press(builder, &object.samples, beatmap, tick.time, false);
        }
        return;
    }
    if object.hit_type & SWELL_FLAG != 0 {
        push_taiko_swell(builder, object, beatmap);
        return;
    }
    // 红蓝判定与画面共用 `HIT_SOUNDS_RIM`（whistle | clap），只认 clap 位会漏掉 whistle 蓝音符。
    let is_rim = object.hitsound & HIT_SOUNDS_RIM != 0;
    if object.hitsound & HIT_SOUNDS_STRONG != 0 {
        push_taiko_strong(builder, &object.samples, beatmap, object.start_time as f64, is_rim);
    } else {
        push_taiko_press(builder, &object.samples, beatmap, object.start_time as f64, is_rim);
    }
}

/// standard 谱面按 taiko 规则发声：鼓点来自转谱结果。
pub(super) fn push_standard_as_taiko<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    object: &StandardHitObject,
    beatmap: &Beatmap,
) {
    for hit in crate::domain::rulesets::taiko::taiko_hitsound_events(beatmap, object) {
        // 转谱把源物件的样本原样交给 taiko 鼓点，因此音效组/音量沿用源物件；
        // 红蓝与 strong 判定与原生 taiko 完全相同（长滑条转成的鼓滚沿用现有「只发一次」语义）。
        let is_rim = hit.hitsound & HIT_SOUNDS_RIM != 0;
        if hit.hitsound & HIT_SOUNDS_STRONG != 0 {
            push_taiko_strong(builder, &object.samples, beatmap, hit.time_ms, is_rim);
        } else {
            push_taiko_press(builder, &object.samples, beatmap, hit.time_ms, is_rim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::HitObjects;
    use crate::hitsound::build_timeline;
    use crate::hitsound::test_support::{beatmap_with, library_with, object_sample};

    #[test]
    fn taiko音效组来自timing_point而不是音量() {
        // 回归：曾经按音量分档（>=90 drum / >=60 normal / 其余 soft），那是 osu! Argon 皮肤的逻辑；
        // legacy 路径只认物件 / timing point 声明的采样组。
        let library = library_with(&[
            "taiko-soft-hitnormal",
            "taiko-normal-hitnormal",
            "taiko-drum-hitnormal",
        ]);
        let mut beatmap = beatmap_with(
            1,
            HitObjects::Taiko(vec![TaikoHitObject {
                start_time: 1000,
                end_time: 1000,
                hit_type: 0,
                hitsound: 0,
                samples: Vec::new(),
            }]),
        );
        beatmap.timing_points[0].sample_set = crate::domain::models::SAMPLE_SET_SOFT;
        beatmap.timing_points[0].sample_volume = 95;

        let timeline = build_timeline(&beatmap, &library);
        assert_eq!(timeline.len(), 1);
        assert_eq!(
            library.name_of(timeline.events[0].source_id),
            Some("taiko-soft-hitnormal")
        );
        // 音量仍然按 timing point 生效（95 → 0.95），只是不再用它挑音效组。
        assert!((timeline.events[0].gain - 0.95).abs() < 1e-9);
    }

    #[test]
    fn taiko音效组取自物件自带的hit_sample() {
        let library = library_with(&[
            "taiko-soft-hitnormal",
            "taiko-normal-hitnormal",
            "taiko-drum-hitclap",
        ]);
        let beatmap = beatmap_with(
            1,
            HitObjects::Taiko(vec![
                // 普通层显式指定 soft。
                TaikoHitObject {
                    start_time: 1000,
                    end_time: 1000,
                    hit_type: 0,
                    hitsound: 1,
                    samples: vec![HitSample::new(SampleBank::Soft, HitAddition::None, 80, None)],
                },
                // 普通层没指定音效组（Auto）、加成层是 drum：蓝音符继承加成层的音效组。
                TaikoHitObject {
                    start_time: 2000,
                    end_time: 2000,
                    hit_type: 0,
                    hitsound: 8,
                    samples: vec![
                        HitSample::new(SampleBank::Auto, HitAddition::None, 80, None),
                        HitSample::new(SampleBank::Drum, HitAddition::Clap, 80, None),
                    ],
                },
                // 同一个物件按鼓心发声时用普通层的 timing point 音效组（缺省 normal）。
                TaikoHitObject {
                    start_time: 3000,
                    end_time: 3000,
                    hit_type: 0,
                    hitsound: 0,
                    samples: vec![HitSample::new(SampleBank::Auto, HitAddition::None, 80, None)],
                },
            ]),
        );
        let timeline = build_timeline(&beatmap, &library);
        let names: Vec<_> = timeline
            .events
            .iter()
            .map(|event| library.name_of(event.source_id).unwrap_or_default())
            .collect();
        assert_eq!(
            names,
            vec![
                "taiko-soft-hitnormal",
                "taiko-drum-hitclap",
                "taiko-normal-hitnormal"
            ]
        );
    }

    #[test]
    fn taiko蓝音符判定包含whistle位() {
        // osu! 的 `Hit` 用「样本里含 hitclap 或 hitwhistle」判定 rim，只看 clap 位会漏掉 whistle 蓝音符。
        let library = library_with(&["taiko-normal-hitclap", "taiko-normal-hitnormal"]);
        let beatmap = beatmap_with(
            1,
            HitObjects::Taiko(vec![TaikoHitObject {
                start_time: 1000,
                end_time: 1000,
                hit_type: 0,
                hitsound: 2,
                samples: Vec::new(),
            }]),
        );
        let timeline = build_timeline(&beatmap, &library);
        assert_eq!(timeline.len(), 1);
        assert_eq!(
            library.name_of(timeline.events[0].source_id),
            Some("taiko-normal-hitclap")
        );
    }

    #[test]
    fn taiko强音符追加finish或whistle() {
        let library = library_with(&[
            "taiko-normal-hitnormal",
            "taiko-normal-hitfinish",
            "taiko-normal-hitclap",
            "taiko-normal-hitwhistle",
        ]);
        let beatmap = beatmap_with(
            1,
            HitObjects::Taiko(vec![
                // 4 = finish 位 → strong：鼓心 base + hitfinish。
                TaikoHitObject {
                    start_time: 1000,
                    end_time: 1000,
                    hit_type: 0,
                    hitsound: 1 | 4,
                    samples: Vec::new(),
                },
                // 8 | 4 = clap + finish → 蓝音符 strong：hitclap + hitwhistle。
                TaikoHitObject {
                    start_time: 2000,
                    end_time: 2000,
                    hit_type: 0,
                    hitsound: 8 | 4,
                    samples: Vec::new(),
                },
            ]),
        );
        let timeline = build_timeline(&beatmap, &library);
        let names: Vec<_> = timeline
            .events
            .iter()
            .map(|event| library.name_of(event.source_id).unwrap_or_default())
            .collect();
        assert_eq!(
            names,
            vec![
                "taiko-normal-hitnormal",
                "taiko-normal-hitfinish",
                "taiko-normal-hitclap",
                "taiko-normal-hitwhistle",
            ]
        );
    }

    #[test]
    fn taiko连打按tick发声() {
        let library = library_with(&["taiko-normal-hitnormal"]);
        let beatmap = beatmap_with(
            1,
            HitObjects::Taiko(vec![TaikoHitObject {
                start_time: 1000,
                end_time: 2000,
                // 位 1（2）= 连打。
                hit_type: 2,
                hitsound: 0,
                samples: Vec::new(),
            }]),
        );
        let timeline = build_timeline(&beatmap, &library);
        // beat_length = 500，SliderTickRate 缺省 1（不是 3）→ 四等分 → 125ms 一个 tick，
        // 覆盖到 `end_time + 半个间距`。
        let times: Vec<f64> = timeline.events.iter().map(|event| event.start_ms).collect();
        assert_eq!(
            times,
            vec![1000.0, 1125.0, 1250.0, 1375.0, 1500.0, 1625.0, 1750.0, 1875.0, 2000.0]
        );
    }

    #[test]
    fn taiko转盘按autoplay节奏交替发声() {
        let library = library_with(&["taiko-normal-hitnormal", "taiko-normal-hitclap"]);
        let beatmap = beatmap_with(
            1,
            HitObjects::Taiko(vec![TaikoHitObject {
                start_time: 0,
                end_time: 4000,
                // 位 3（8）= 大连打。
                hit_type: 8,
                hitsound: 0,
                samples: Vec::new(),
            }]),
        );
        // OD 5 → 每秒 5 × 1.65 = 8.25 次，4000ms 需要 33 次；敲击间隔 min(50, 121.2) = 50ms。
        let timeline = build_timeline(&beatmap, &library);
        assert_eq!(timeline.len(), 33);
        for (index, event) in timeline.events.iter().enumerate() {
            let expected = if index % 2 == 0 {
                "taiko-normal-hitnormal"
            } else {
                "taiko-normal-hitclap"
            };
            assert_eq!(library.name_of(event.source_id), Some(expected), "第 {index} 次敲击");
            assert!((event.start_ms - index as f64 * 50.0).abs() < 1e-9);
        }
    }

    #[test]
    fn taiko蓝音符使用hitclap() {
        let library = library_with(&["taiko-normal-hitclap", "taiko-normal-hitnormal"]);
        let beatmap = beatmap_with(
            1,
            HitObjects::Taiko(vec![TaikoHitObject {
                start_time: 500,
                end_time: 500,
                // 位 3 表示 rim（蓝音符）。
                hitsound: 8,
                hit_type: 0,
                samples: object_sample(SampleBank::Normal, 70),
            }]),
        );
        let timeline = build_timeline(&beatmap, &library);
        assert_eq!(timeline.len(), 1);
        assert_eq!(
            library.name_of(timeline.events[0].source_id),
            Some("taiko-normal-hitclap")
        );
    }

    #[test]
    fn taiko谱面从头到尾按legacy规则展开() {
        // 原生 taiko 谱面（Mode: 1）：timing point 是 soft 组，音符依次是
        // 蓝音符（whistle 位）、strong（clap + finish 位）、连打。
        let source = "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nCircleSize:4\nOverallDifficulty:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,0,80,1,0\n\n[HitObjects]\n256,192,1000,1,2,0:0:0:0:\n256,192,2000,1,13,0:0:0:0:\n256,192,3000,2,0,L|356:192,1,140\n";
        let beatmap = crate::parse_beatmap_bytes(source.as_bytes()).expect("fixture 必须可解析");
        let library = library_with(&[
            "taiko-soft-hitclap",
            "taiko-soft-hitwhistle",
            "taiko-soft-hitnormal",
        ]);

        let timeline = build_timeline(&beatmap, &library);
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
        // 1000：whistle 位 → 蓝音符 hitclap；2000：clap + finish → strong（hitclap + hitwhistle）；
        // 3000 起是连打，每 125ms（beat_length 500 的四等分）一次鼓心。
        assert_eq!(
            events,
            vec![
                (1000.0, "taiko-soft-hitclap"),
                (2000.0, "taiko-soft-hitclap"),
                (2000.0, "taiko-soft-hitwhistle"),
                (3000.0, "taiko-soft-hitnormal"),
                (3125.0, "taiko-soft-hitnormal"),
                (3250.0, "taiko-soft-hitnormal"),
                (3375.0, "taiko-soft-hitnormal"),
                (3500.0, "taiko-soft-hitnormal"),
            ]
        );
    }
}
