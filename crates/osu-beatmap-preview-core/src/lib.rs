//! 跨平台本地渲染引擎核心。
//!
//! 外部调用优先使用 [`api`]、[`model`]、[`render`] 和 [`gameplay`] 入口：
//! `api` 负责会话输入输出，`model` 提供稳定的谱面数据模型，`render` 提供场景
//! 与绘制结果，`gameplay` 提供游玩/回放的输入与状态契约（后续功能的接口位）。
//! `domain` 保留为旧版本兼容层，内部实现不作为新的调用入口。

pub mod api;
pub mod config;
pub mod domain;
pub mod gameplay;
pub mod hitsound;
pub mod model;
pub mod processing;
pub mod render;
pub mod support;

pub use api::{
    ImageData, RealtimeMode, RealtimeOptions, RealtimeSession, RenderConfig, ResourceBundle,
    TimelineInfo,
};
pub use gameplay::{
    GameplayMode, GameplayOptions, GameplayOverlay, InputSnapshot, InputSource, Judgement,
    JudgementCounts, JudgementEngine, OverlayStyle, ScoreSnapshot,
};
pub use hitsound::{
    build_timeline as build_hitsound_timeline, has_embedded_asset as hitsound_has_embedded_asset,
    referenced_names as hitsound_referenced_names, volume_gain as hitsound_volume_gain, Channels,
    HitsoundMixer, HitsoundTimeline, LoopHandle, PlayEvent as HitsoundEvent, PlayFrequency,
    SampleData, SampleLibrary,
};
pub use model::{mods_for_mode, parse_mods, validate_mods, ModSettings};
pub use model::{
    Beatmap, BeatmapInfo, BreakPeriod, CatchHitObject, HitObjects, KvSection, ManiaHitObject,
    StandardHitObject, TaikoHitObject, TimingPoint,
};
pub use processing::conversion::{catch_convert, mania_convert, taiko_convert};
pub use processing::media::{
    entry_extension, normalize_entry_path, sample_entry_matches, BeatmapMedia, MediaEntry,
    SAMPLE_EXTENSIONS,
};
pub use processing::parse::parse_beatmap_bytes;
pub use processing::timeline::preview_start_ms;
pub use render::{DrawCommand, FrameScene, Img, Rgba, SceneRect, SceneSize};
pub use support::error::{ErrorKind, PreviewError, Result};
