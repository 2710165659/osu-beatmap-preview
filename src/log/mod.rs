//! 多进程安全的双日志系统（NDJSON 汇总 + `tail -f` 进度事件）。
//!
//! 所有进程共享写入同一组固定路径的文件（默认 `<临时目录>/osu-beatmap-preview/logs`）：
//! - `render.log`：每张谱面渲染完（成功 / 失败 / 缓存命中）追加一行 NDJSON 汇总。
//! - `progress.log`：人类可读的阶段事件流，可用 `tail -f` 实时查看多进程进度。
//!
//! 日志目录只由编译时 `LOG_DIR` 配置决定。
//! 日志写入失败只降级到 stderr 提示，绝不阻断渲染。

pub(crate) mod config;
mod context;
mod event;
mod summary;
mod timestamp;
mod writer;

#[cfg(test)]
mod tests;

pub use context::{
    record_cache, record_output_bytes, record_stage, record_stage_status, record_video_stats,
    set_bid, CacheKind, VideoStats,
};
pub use event::event;
pub use summary::{write_summary, SummaryRecord};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    config::reset_for_tests();
    context::reset_for_tests();
}
