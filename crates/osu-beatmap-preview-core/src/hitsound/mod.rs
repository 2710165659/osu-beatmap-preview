//! 打击音（hit sound）时间轴与混音。
//!
//! 这个模块只负责「什么时候播放哪个样本、多大声」，不接触文件系统、网络或音频设备：
//! 样本 PCM 由宿主提供（CLI 解码内嵌音频、Web 宿主用浏览器解码），因此同一套事件与
//! 混音逻辑既能驱动 MP4 离线混音，也能驱动浏览器实时播放。
//!
//! 分工：`sample` 管样本与名字解析，`timeline` 管事件与构建器，`common` 放各模式共用的
//! 取样辅助，`standard`/`taiko`/`catch`/`mania` 按 osu! 规则展开各自的物件，
//! `mixer` 把时间轴混成 PCM。

mod assets;
mod catch;
mod common;
mod mania;
mod mixer;
mod sample;
mod standard;
mod taiko;
mod timeline;

#[cfg(test)]
mod test_support;

pub use assets::{
    asset_bytes, asset_count, asset_names, has_embedded_asset, HITSOUND_ASSETS,
};
pub use mixer::{HitsoundMixer, LoopHandle};
pub use sample::{Channels, SampleData, SampleLibrary, SampleResolver};
pub use timeline::{CollectNames, HitsoundTimeline, PlayEvent, PlayFrequency};

use crate::domain::models::{Beatmap, HitObjects};

use self::catch::push_catch;
use self::mania::push_mania;
use self::standard::push_standard;
use self::taiko::{push_standard_as_taiko, push_taiko_object};
use self::timeline::TimelineBuilder;

/// osu! 的 `SkinnableSound` 将谱面音量百分比直接转换为线性增益。
/// 100 → 1.0，70 → 0.7，50 → 0.5；预览默认 50% 与游戏内一致。
pub fn volume_gain(volume: i32) -> f64 {
    volume.clamp(0, 100) as f64 / 100.0
}

/// 宿主没有提供采样率时使用的默认混音采样率。
pub const SAMPLE_RATE: u32 = 48_000;

/// 根据谱面与目标模式生成完整的打击音时间轴。
///
/// 模式取 `beatmap.mode()`：0/1/2/3 分别对应 standard/taiko/catch/mania。
/// standard 谱面在 taiko 模式下也会按 taiko 规则发声。
pub fn build_timeline(beatmap: &Beatmap, library: &SampleLibrary) -> HitsoundTimeline {
    // 只读借用即可完成解析：`SampleResolver` 对 `&SampleLibrary` 有实现，
    // 不需要克隆整份样本数据。
    let mut resolver = library;
    build_with(beatmap, &mut resolver)
}

/// 收集时间轴会用到的全部候选样本名，供宿主按需预解码。
pub fn referenced_names(beatmap: &Beatmap) -> Vec<String> {
    let mut collector = CollectNames::default();
    build_with(beatmap, &mut collector);
    collector.into_names()
}

fn build_with<R: SampleResolver>(beatmap: &Beatmap, resolver: &mut R) -> HitsoundTimeline {
    let mut builder = TimelineBuilder::new(resolver);
    match &beatmap.hit_objects {
        HitObjects::Taiko(objects) => {
            for object in objects {
                push_taiko_object(&mut builder, object, beatmap);
            }
        }
        HitObjects::Catch(objects) => {
            for object in objects {
                push_catch(&mut builder, object, beatmap);
            }
        }
        HitObjects::Mania(objects) => {
            for object in objects {
                push_mania(&mut builder, object, beatmap);
            }
        }
        HitObjects::Standard(objects) => {
            if beatmap.mode() == 1 {
                for object in objects {
                    push_standard_as_taiko(&mut builder, object, beatmap);
                }
            } else {
                for object in objects {
                    push_standard(&mut builder, object, beatmap);
                }
            }
        }
    }
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::test_support::{beatmap_with, library_with, object_sample};
    use super::{build_timeline, referenced_names, volume_gain, SampleLibrary};
    use crate::domain::models::{HitAddition, HitObjects, HitSample, SampleBank, StandardHitObject};

    /// 滑条的音效参数取自正确的列。
    #[test]
    fn slider_hitsound_params_come_from_correct_columns() {
        // 回归：滑条第 6 列是曲线（`B|356:192`），曾把它当成 hitSample 解析出
        // additionSet=192，导致音效组与音量全错。
        let source = "osu file format v14\n\n[General]\nMode: 0\n\n[Difficulty]\nCircleSize:4\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,0,100,1,0\n\n[HitObjects]\n256,192,2000,2,0,B|356:192,1,140\n";
        let beatmap = crate::parse_beatmap_bytes(source.as_bytes()).expect("fixture 必须可解析");
        let object = &beatmap.hit_objects.as_standard().expect("必须是 standard 谱面")[0];
        // hitSample 全为 0：物件不覆盖任何参数，交给 timing point 决定。
        assert!(object.samples.is_empty(), "samples={:?}", object.samples);
        let names = referenced_names(&beatmap);
        assert!(names.contains(&"soft-hitnormal".to_string()), "names={names:?}");
        assert!(names.contains(&"soft-sliderslide".to_string()), "names={names:?}");
    }

    /// 音量曲线与游戏内一致。
    #[test]
    fn volume_curve_matches_the_game() {
        assert!((volume_gain(100) - 1.0).abs() < 1e-9);
        assert!((volume_gain(50) - 0.5).abs() < 1e-12);
        assert!((volume_gain(0) - 0.0).abs() < 1e-12);
        // 越界音量按边界处理，不会 panic。
        assert!((volume_gain(-20) - volume_gain(0)).abs() < 1e-12);
        assert!((volume_gain(500) - 1.0).abs() < 1e-9);
    }

    /// 缺失样本只产生静音，不产生事件。
    #[test]
    fn missing_samples_produce_silence_only() {
        let beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                start_time: 1000,
                end_time: 1000,
                hit_type: 1,
                hitsound: 0,
                samples: object_sample(SampleBank::Normal, 100),
                ..Default::default()
            }]),
        );
        // 空样本库：没有任何事件，也不会 panic。
        let empty = SampleLibrary::new();
        assert!(build_timeline(&beatmap, &empty).is_empty());

        // 只有 drum 存在时，normal 物件不会命中 drum。
        let drum_only = library_with(&["drum-hitnormal"]);
        assert!(build_timeline(&beatmap, &drum_only).is_empty());

        let normal = library_with(&["normal-hitnormal"]);
        let timeline = build_timeline(&beatmap, &normal);
        assert_eq!(timeline.len(), 1);
        assert!((timeline.events[0].start_ms - 1000.0).abs() < 1e-9);
        assert!((timeline.events[0].gain - 1.0).abs() < 1e-9);
    }

    /// 引用名收集覆盖全部候选与裸名回退。
    #[test]
    fn referenced_names_cover_candidates_and_bare_fallback() {
        let beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                start_time: 1000,
                end_time: 1000,
                hit_type: 1,
                // 10 = 哨音(2) + 拍手(8) 位；两个加成音都必须保留。
                hitsound: 10,
                samples: vec![
                    HitSample::new(SampleBank::Soft, HitAddition::None, 80, None),
                    HitSample::new(SampleBank::Soft, HitAddition::Whistle, 80, None),
                ],
                ..Default::default()
            }]),
        );
        let names = referenced_names(&beatmap);
        // soft 音效组会同时登记带 bank 前缀的名字与裸名（裸名是 osu! 的回退查找）。
        for expected in ["soft-hitnormal", "hitnormal", "soft-hitwhistle", "hitwhistle"] {
            assert!(
                names.contains(&expected.to_string()),
                "缺少候选名 {expected}：{names:?}"
            );
        }
    }

    /// 转盘的音效参数取自第七列，而不是结束时间。
    #[test]
    fn spinner_hitsound_params_come_from_seventh_column() {
        // 回归：转盘行是 `x,y,time,type,hitSound,endTime,hitSample`，曾经按圆圈的列号去读，
        // 把结束时间（`3000`）当成了音效组 id，结果转盘音效组恒为 normal。
        let source = "osu file format v14\n\n[General]\nMode: 0\n\n[Difficulty]\nCircleSize:4\n\n[TimingPoints]\n0,500,4,2,1,100,1,0\n\n[HitObjects]\n256,192,1000,8,0,3000,2:3:0:40:\n";
        let beatmap = crate::parse_beatmap_bytes(source.as_bytes()).unwrap();
        let object = &beatmap.hit_objects.as_standard().unwrap()[0];
        assert_eq!(object.end_time, 3000);
        let sample = object.samples.first().expect("转盘必须解析出样本");
        assert_eq!(sample.bank, SampleBank::Soft);
        assert_eq!(sample.volume, 40);
    }

    /// 自定义文件名只覆盖普通层。
    #[test]
    fn custom_filename_overrides_normal_layer_only() {
        let source = "osu file format v14\n\n[General]\nMode: 0\n\n[Difficulty]\nCircleSize:4\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,10,0:0:0:100:custom-hit.ogg\n";
        let beatmap = crate::parse_beatmap_bytes(source.as_bytes()).unwrap();
        let object = &beatmap.hit_objects.as_standard().unwrap()[0];
        assert_eq!(object.samples.len(), 3);
        assert_eq!(object.samples[0].filename.as_deref(), Some("custom-hit.ogg"));
        assert!(object.samples[1..].iter().all(|sample| sample.filename.is_none()));
    }

    /// timing point 的索引产生带后缀的候选名。
    #[test]
    fn timing_point_index_produces_suffixed_candidates() {
        // 物件没有自带 hitSample：音效索引来自 timing point 的 sampleIndex 列。
        let mut beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                start_time: 1000,
                end_time: 1000,
                hit_type: 1,
                hitsound: 0,
                samples: Vec::new(),
                ..Default::default()
            }]),
        );
        beatmap.timing_points[0].sample_set = 2;
        beatmap.timing_points[0].sample_index = 20;

        let names = referenced_names(&beatmap);
        // 带后缀的名字优先，不带后缀的仍然是回退（内嵌皮肤提供的是后者）。
        assert!(names.contains(&"soft-hitnormal20".to_string()), "names={names:?}");
        assert!(names.contains(&"soft-hitnormal".to_string()), "names={names:?}");

        // 谱面包里只有带后缀的样本时命中它。
        assert_eq!(build_timeline(&beatmap, &library_with(&["soft-hitnormal20"])).len(), 1);
        // 只有不带后缀的样本时回退到它。
        assert_eq!(build_timeline(&beatmap, &library_with(&["soft-hitnormal"])).len(), 1);
        // 索引 20 的样本缺失、无后缀的也没有时按静音处理。
        assert!(build_timeline(&beatmap, &library_with(&["soft-hitclap20"])).is_empty());
    }

    /// 物件的音效索引优先于 timing point。
    #[test]
    fn object_sample_index_wins_over_timing_point() {
        // 物件的 hitSample 声明了 index=7：普通层、加成音、滑条 tick 与滑行音都带后缀 `7`。
        let samples = vec![
            HitSample::new(SampleBank::Soft, HitAddition::None, 100, None).with_custom_bank(7),
            HitSample::new(SampleBank::Soft, HitAddition::Clap, 100, None).with_custom_bank(7),
        ];
        let mut beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![
                StandardHitObject {
                    start_time: 1000,
                    end_time: 1000,
                    hit_type: 1,
                    hitsound: 8,
                    samples: samples.clone(),
                    ..Default::default()
                },
                // 滑条：滑行音与 tick 沿用物件头部的自定义索引。
                StandardHitObject {
                    start_time: 2000,
                    end_time: 4000,
                    hit_type: 2,
                    hitsound: 0,
                    slider_type: Some("L".to_string()),
                    slider_points: vec![(100, 0)],
                    slider_repeats: 1,
                    slider_pixel_length: 300.0,
                    samples,
                    ..Default::default()
                },
            ]),
        );
        // timing point 只声明索引 1（无后缀），物件声明了 7 就不该用到它。
        beatmap.timing_points[0].sample_set = 2;
        beatmap.timing_points[0].sample_index = 1;

        let names = referenced_names(&beatmap);
        for expected in [
            "soft-hitnormal7",
            "soft-hitclap7",
            "soft-sliderslide7",
            "soft-slidertick7",
        ] {
            assert!(names.contains(&expected.to_string()), "缺少 {expected}：{names:?}");
        }

        // 带后缀的样本存在时优先使用它，而不是同名的无后缀样本（后者属于内嵌皮肤）。
        let library = library_with(&[
            "soft-hitnormal7",
            "soft-hitclap7",
            "soft-sliderslide7",
            "soft-slidertick7",
            "soft-hitnormal",
        ]);
        let timeline = build_timeline(&beatmap, &library);
        assert!(!timeline.is_empty());
        assert!(
            timeline
                .events
                .iter()
                .any(|event| library.name_of(event.source_id) == Some("soft-hitnormal7"))
        );
    }

    /// 索引为一表示谱面自带的无后缀音效。
    #[test]
    fn index_one_means_bare_beatmap_sample() {
        let mut beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                start_time: 1000,
                end_time: 1000,
                hit_type: 1,
                hitsound: 0,
                samples: Vec::new(),
                ..Default::default()
            }]),
        );
        beatmap.timing_points[0].sample_set = 2;
        beatmap.timing_points[0].sample_index = 1;
        let names = referenced_names(&beatmap);
        assert!(names.contains(&"soft-hitnormal".to_string()), "names={names:?}");
        // 1 不带后缀，`soft-hitnormal1` 不是合法候选名。
        assert!(!names.contains(&"soft-hitnormal1".to_string()), "names={names:?}");
    }

    /// taiko 的候选名同样带索引后缀。
    #[test]
    fn taiko_candidates_also_carry_index_suffix() {
        let mut beatmap = beatmap_with(
            1,
            HitObjects::Taiko(vec![crate::domain::models::TaikoHitObject {
                start_time: 1000,
                end_time: 1000,
                hit_type: 0,
                hitsound: 0,
                samples: Vec::new(),
            }]),
        );
        beatmap.timing_points[0].sample_set = 2;
        beatmap.timing_points[0].sample_index = 20;
        let names = referenced_names(&beatmap);
        assert!(names.contains(&"taiko-soft-hitnormal20".to_string()), "names={names:?}");
        assert!(names.contains(&"taiko-soft-hitnormal".to_string()), "names={names:?}");
        assert_eq!(
            build_timeline(&beatmap, &library_with(&["taiko-soft-hitnormal20"])).len(),
            1
        );
    }
}
