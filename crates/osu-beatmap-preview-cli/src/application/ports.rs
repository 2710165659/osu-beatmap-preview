//! 应用层端口。后续下载源、实时 Surface 和回放时间线从这里接入。

use crate::application::plan::RenderPlan;
use osu_beatmap_preview_core::model::Beatmap;
use osu_beatmap_preview_core::support::error::Result;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
pub(crate) trait BeatmapSource: Send + Sync {
    fn load(&self, bid: &str, cache_root: &Path, no_cache: bool) -> Result<(Beatmap, PathBuf)>;
}

#[allow(dead_code)]
pub(crate) trait RenderBackend: Send + Sync {
    fn render(&self, beatmap: &Beatmap, plan: &RenderPlan, output: &Path) -> Result<PathBuf>;
}

#[allow(dead_code)]
pub(crate) trait GameplayTimeline: Send + Sync {
    fn cursor_at(&self, time_ms: i64) -> Option<(f32, f32)>;
    fn pressed_lanes_at(&self, time_ms: i64) -> &[u8];
}

// 正式的游戏输入/判定契约放在 core 的 `gameplay` 模块（`InputSource` /
// `JudgementEngine` / `ScoreSnapshot`），CLI 只保留上面这个薄适配层的占位：
// 回放（OSR）与 Web 游玩接入时都应实现 core 的 trait，避免两套输入模型。
