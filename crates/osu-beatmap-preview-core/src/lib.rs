//! 跨平台本地渲染引擎核心。
//!
//! 外部调用优先使用 [`api`]、[`model`] 和 [`render`] 三个入口：
//! `api` 负责会话输入输出，`model` 提供稳定的谱面数据模型，`render` 提供场景
//! 与绘制结果。`domain` 保留为旧版本兼容层，内部实现不作为新的调用入口。

pub mod api;
pub mod config;
pub mod domain;
pub mod model;
pub mod processing;
pub mod render;
pub mod support;

pub use api::{
    AudioData, ImageData, RealtimeMode, RealtimeOptions, RealtimeSession, RenderConfig,
    ResourceBundle, TimelineInfo,
};
pub use model::{mods_for_mode, parse_mods, validate_mods, ModSettings};
pub use model::{
    Beatmap, BeatmapInfo, BreakPeriod, CatchHitObject, HitObjects, KvSection, ManiaHitObject,
    StandardHitObject, TaikoHitObject, TimingPoint,
};
pub use processing::conversion::{catch_convert, mania_convert, taiko_convert};
pub use processing::parse::parse_beatmap_bytes;
pub use render::{DrawCommand, FrameScene, Img, Rgba, SceneRect, SceneSize};
pub use support::error::{ErrorKind, PreviewError, Result};
