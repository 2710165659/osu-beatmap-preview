//! 面向调用者的谱面数据模型门面。
//!
//! 具体解析算法仍位于兼容的内部模块中；调用者只需要从这里获取稳定的数据模型，
//! 不必依赖 `domain` 这一历史命名。

pub mod beatmap {
    pub use crate::domain::models::{
        Beatmap, BreakPeriod, CatchHitObject, HitObjects, KvSection, ManiaHitObject,
        StandardHitObject, TaikoHitObject, TimingPoint,
    };
}

pub mod mods {
    pub use crate::domain::mods::{mods_for_mode, parse_mods, validate_mods, ModSettings};
}

pub mod info {
    pub use crate::domain::info::BeatmapInfo;
}

pub use beatmap::{
    Beatmap, BreakPeriod, CatchHitObject, HitObjects, KvSection, ManiaHitObject, StandardHitObject,
    TaikoHitObject, TimingPoint,
};
pub use info::BeatmapInfo;
pub use mods::{mods_for_mode, parse_mods, validate_mods, ModSettings};
