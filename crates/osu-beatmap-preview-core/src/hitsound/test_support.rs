#![cfg(test)]

//! 各模式打击音测试共用的最小谱面与样本库构造。
//!
//! 只给测试用：`hitsound/mod.rs` 用 `#[cfg(test)] mod test_support;` 引入，
//! 不参与生产构建。

use crate::domain::models::{
    Beatmap, HitAddition, HitObjects, HitSample, SampleBank, StandardHitObject, TimingPoint,
};
use crate::hitsound::{SampleData, SampleLibrary};

/// 构造只有一条红线的最小谱面，用于时间轴测试。
pub(super) fn beatmap_with(mode: i32, objects: HitObjects) -> Beatmap {
    Beatmap {
        metadata: Default::default(),
        difficulty: Default::default(),
        general: Default::default(),
        timing_points: vec![TimingPoint {
            time: 0.0,
            beat_length: 500.0,
            meter: 4,
            uninherited: true,
            kiai_mode: false,
            omit_first_bar_line: false,
            sample_set: 0,
            sample_index: 0,
            sample_volume: 100,
        }],
        hit_objects: objects,
        break_periods: Vec::new(),
        background_filename: None,
        combo_colors: Vec::new(),
        beat_divisor: 0,
    }
    .with_mode(mode)
}

/// 构造一个只有转盘的标准谱面。
pub(super) fn spinner_beatmap(start_time: i64, end_time: i64) -> Beatmap {
    beatmap_with(
        0,
        HitObjects::Standard(vec![StandardHitObject {
            start_time,
            end_time,
            // 位 3（8）= 转盘。
            hit_type: 8,
            ..Default::default()
        }]),
    )
}

pub(super) fn object_sample(bank: SampleBank, volume: i32) -> Vec<HitSample> {
    vec![HitSample::new(bank, HitAddition::None, volume, None)]
}

/// 一条红线：`volume` 为音量百分比，`sample_set` 为音效组 id（0 表示沿用 General.SampleSet）。
pub(super) fn timing_point(time: f64, sample_set: i32, volume: i32) -> TimingPoint {
    TimingPoint {
        time,
        beat_length: 500.0,
        meter: 4,
        uninherited: true,
        kiai_mode: false,
        omit_first_bar_line: false,
        sample_set,
        sample_index: 0,
        sample_volume: volume,
    }
}

pub(super) fn library_with(names: &[&str]) -> SampleLibrary {
    let mut library = SampleLibrary::new();
    for name in names {
        library.insert(*name, SampleData::stereo(vec![1.0, 1.0, 1.0, 1.0], 1000));
    }
    library
}

trait WithMode {
    fn with_mode(self, mode: i32) -> Beatmap;
}

impl WithMode for Beatmap {
    fn with_mode(mut self, mode: i32) -> Beatmap {
        self.general.insert("Mode", mode.to_string());
        self
    }
}
