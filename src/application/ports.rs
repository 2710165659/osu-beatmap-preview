//! 应用层端口。后续下载源、实时 Surface 和回放时间线从这里接入。

use crate::application::plan::RenderPlan;
use crate::domain::errors::Result;
use crate::domain::models::Beatmap;
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
