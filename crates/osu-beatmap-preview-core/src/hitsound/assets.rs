//! 随二进制分发的打击音资源。
//!
//! build.rs 把 `assets/hitsound/*.ogg` 内嵌进来，宿主按样本名直接取字节、解码成 PCM
//! 后交给混音器，因此不需要再从网络或磁盘读取音效文件。CLI 与 Web 用同一份来源，
//! 也就不会出现「静态副本忘记同步导致全部加载失败」这类问题。
//!
//! 查找表很小（36 项），直接线性扫描即可，不需要额外建立索引。

include!(concat!(env!("OUT_DIR"), "/hitsound_assets.rs"));

/// 按样本名取回内嵌的 ogg 字节；没有对应资源时返回 `None`。
pub fn asset_bytes(name: &str) -> Option<&'static [u8]> {
    HITSOUND_ASSETS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, bytes)| *bytes)
}

/// 内嵌资源总数，用于日志与测试。
pub fn asset_count() -> usize {
    HITSOUND_ASSETS.len()
}

/// 某个样本名是否随二进制分发。
///
/// 宿主按「谱面自带的同名条目 > 内嵌皮肤 > 静音」的优先级取样本：先在压缩包里用
/// [`sample_entry_matches`](crate::sample_entry_matches) 找同名条目，找不到再用这个名字
/// 取内嵌资源兜底。两者的候选名都由 [`referenced_names`](crate::hitsound::referenced_names)
/// 给出，因此这里只回答「这个名字有没有内嵌资源」。
pub fn has_embedded_asset(name: &str) -> bool {
    asset_bytes(name).is_some()
}

/// 全部内嵌样本名（已排序，因为生成时就按名称排过序）。
pub fn asset_names() -> impl Iterator<Item = &'static str> {
    HITSOUND_ASSETS.iter().map(|(name, _)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hitsound::referenced_names;
    use crate::domain::models::{
        Beatmap, CatchHitObject, HitAddition, HitObjects, HitSample, KvSection, ManiaHitObject,
        SampleBank, StandardHitObject, TaikoHitObject, TimingPoint,
    };

    /// 内嵌资源覆盖四模式全部音效。
    #[test]
    fn embedded_assets_cover_all_modes() {
        assert_eq!(asset_count(), 36);
        for name in [
            "normal-hitnormal",
            "soft-hitnormal",
            "drum-hitnormal",
            "normal-sliderslide",
            "normal-slidertick",
            "spinnerbonus",
            "spinnerbonus-max",
            "spinnerspin",
            "taiko-soft-hitnormal",
            "taiko-normal-hitclap",
            "taiko-drum-hitnormal",
            "taiko-drum-hitfinish",
        ] {
            let bytes = asset_bytes(name).unwrap_or_else(|| panic!("缺少内嵌资源 {name}"));
            // ogg 以 `OggS` 开头：确认取到的是真资源而不是空切片。
            assert_eq!(&bytes[..4], b"OggS", "{name} 不是 ogg 数据");
        }
        assert!(asset_bytes("不存在").is_none());
    }

    /// 各模式引用的样本名都能在内嵌资源里找到。
    #[test]
    fn every_referenced_sample_name_exists_in_embedded_assets() {
        // 不带宽度的裸名是 osu! 的次级回退查找（共享 Gameplay 目录），本套皮肤不提供；
        // 无 bank 前缀的转盘音效与 taiko 的 strong 组（classic 皮肤没有）同样允许缺失。
        const ALLOWED_MISSING: &[&str] = &[
            "hitnormal",
            "hitwhistle",
            "hitfinish",
            "hitclap",
            "slidertick",
            "sliderslide",
            "sliderwhistle",
            "normal-spinnerbonus",
            "normal-spinnerbonus-max",
            "normal-spinnerspin",
            "taiko-strong-hitnormal",
            "taiko-strong-hitclap",
            "taiko-strong-hitflourish",
        ];
        for mode in 0..4 {
            for name in referenced_names(&mode_beatmap(mode)) {
                if ALLOWED_MISSING.contains(&name.as_str()) {
                    continue;
                }
                assert!(
                    asset_bytes(&name).is_some(),
                    "模式 {mode} 引用了未内嵌的样本 {name}"
                );
            }
        }
    }

    /// 构造一个引用全部常规音效名的合成谱面。
    fn mode_beatmap(mode: i32) -> Beatmap {
        let samples = |bank: SampleBank| {
            vec![
                HitSample::new(bank, HitAddition::None, 100, None),
                HitSample::new(bank, HitAddition::Whistle, 100, None),
                HitSample::new(bank, HitAddition::Finish, 100, None),
                HitSample::new(bank, HitAddition::Clap, 100, None),
            ]
        };
        let banks = [SampleBank::Normal, SampleBank::Soft, SampleBank::Drum];
        let mut builder = Beatmap {
            metadata: KvSection::default(),
            difficulty: KvSection::default(),
            general: KvSection::default(),
            timing_points: vec![TimingPoint {
                time: 0.0,
                beat_length: 500.0,
                meter: 4,
                uninherited: true,
                kiai_mode: false,
                omit_first_bar_line: false,
                sample_set: 3,
                sample_index: 0,
                sample_volume: 100,
            }],
            hit_objects: HitObjects::Standard(Vec::new()),
            break_periods: Vec::new(),
            background_filename: None,
            combo_colors: Vec::new(),
            beat_divisor: 0,
        };
        builder.general.insert("Mode", mode.to_string());
        builder.hit_objects = match mode {
            1 => HitObjects::Taiko(
                banks
                    .into_iter()
                    .enumerate()
                    .map(|(index, bank)| TaikoHitObject {
                        start_time: 1000 * index as i64,
                        end_time: 1000 * index as i64,
                        hit_type: 0,
                        hitsound: 1,
                        samples: samples(bank),
                    })
                    .collect(),
            ),
            2 => HitObjects::Catch(
                banks
                    .into_iter()
                    .enumerate()
                    .map(|(index, bank)| CatchHitObject {
                        x: 0,
                        y: 0,
                        start_time: 1000 * index as i64,
                        end_time: 1000 * index as i64 + 1000,
                        hit_type: 2,
                        slider_repeats: 2,
                        slider_pixel_length: 300.0,
                        samples: samples(bank),
                        ..Default::default()
                    })
                    .collect(),
            ),
            3 => HitObjects::Mania(vec![ManiaHitObject {
                lane: 0,
                start_time: 1000,
                end_time: 2000,
                is_long_note: true,
                samples: Vec::new(),
            }]),
            _ => {
                let mut objects: Vec<StandardHitObject> = banks
                    .into_iter()
                    .enumerate()
                    .map(|(index, bank)| StandardHitObject {
                        x: 0,
                        y: 0,
                        start_time: 1000 * index as i64,
                        end_time: 1000 * index as i64 + 1000,
                        hit_type: 2,
                        hitsound: 1,
                        slider_repeats: 2,
                        slider_pixel_length: 300.0,
                        samples: samples(bank),
                        slider_edge_samples: vec![samples(bank)],
                        ..Default::default()
                    })
                    .collect();
                // 转盘覆盖 spinnerspin / spinnerbonus。
                objects.push(StandardHitObject {
                    x: 0,
                    y: 0,
                    start_time: 5000,
                    end_time: 8000,
                    hit_type: 8,
                    hitsound: 1,
                    samples: samples(SampleBank::Normal),
                    ..Default::default()
                });
                HitObjects::Standard(objects)
            }
        };
        builder
    }
}
