//! 回放音频按准备结果分派；各模式消费同一不可变判定时间线。
use super::*;
use crate::{gameplay::timeline::GameplayTimeline, replay::preparation::PreparedReplay};

pub fn build_timeline(
    beatmap: &Beatmap,
    prepared: &PreparedReplay,
    gameplay: &GameplayTimeline,
    library: &SampleLibrary,
) -> HitsoundTimeline {
    match prepared {
        PreparedReplay::Standard(p) => osr::build_timeline(beatmap, p, gameplay, library),
        PreparedReplay::Taiko(p) => super::taiko_replay::build_timeline(p, gameplay, library),
        PreparedReplay::Catch(p) => super::catch_replay::build_timeline(p, gameplay, library),
        PreparedReplay::Mania(p) => super::mania_replay::build_timeline(p, gameplay, library),
    }
}
pub fn referenced_names(
    beatmap: &Beatmap,
    prepared: &PreparedReplay,
    gameplay: &GameplayTimeline,
) -> Vec<String> {
    match prepared {
        PreparedReplay::Standard(p) => osr::referenced_names(beatmap, p, gameplay),
        PreparedReplay::Taiko(p) => super::taiko_replay::referenced_names(p, gameplay),
        PreparedReplay::Catch(p) => super::catch_replay::referenced_names(p, gameplay),
        PreparedReplay::Mania(p) => super::mania_replay::referenced_names(p, gameplay),
    }
}
