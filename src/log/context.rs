//! 进程级渲染上下文：当前 bid、缓存命中状态、阶段耗时与视频编码统计。
//!
//! 单进程单渲染，跨线程（音频线程）安全累加；写汇总时合并进 JSON。

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

/// 缓存类型，对应汇总 JSON 中 `cache` 对象的键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheKind {
    Osu,
    Osz,
    Audio,
    Output,
}

impl CacheKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CacheKind::Osu => "osu",
            CacheKind::Osz => "osz",
            CacheKind::Audio => "audio",
            CacheKind::Output => "output",
        }
    }
}

/// MP4 编码器统计（由 `video` 模块上报）。
#[derive(Debug, Clone, Default)]
pub struct VideoStats {
    pub backend: Option<String>,
    pub resolution: Option<String>,
    pub fps: Option<u32>,
    pub frame_count: Option<usize>,
    pub video_ms: Option<f64>,
    pub render_compose_ms: Option<f64>,
    pub encode_ms: Option<f64>,
    pub mux_ms: Option<f64>,
    pub audio_ms: Option<f64>,
}

#[derive(Debug, Default)]
struct Context {
    bid: Option<String>,
    cache: BTreeMap<String, String>,
    stages: BTreeMap<String, Value>,
    video: Option<VideoStats>,
    output_bytes: Option<u64>,
}

/// 汇总所需的上下文快照（克隆，避免持锁构建 JSON）。
#[derive(Debug, Default, Clone)]
pub struct Snapshot {
    pub cache: BTreeMap<String, String>,
    pub stages: BTreeMap<String, Value>,
    pub video: Option<VideoStats>,
    pub output_bytes: Option<u64>,
}

static CONTEXT: OnceLock<Mutex<Context>> = OnceLock::new();

fn with_context<T>(f: impl FnOnce(&mut Context) -> T) -> T {
    let mutex = CONTEXT.get_or_init(|| Mutex::new(Context::default()));
    let mut guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// 记录当前渲染的 bid；线程内事件传 `None` 时自动回退到该值。
pub fn set_bid(bid: &str) {
    with_context(|ctx| ctx.bid = Some(bid.to_string()));
}

pub(crate) fn bid() -> Option<String> {
    with_context(|ctx| ctx.bid.clone())
}

/// 记录某类缓存的命中状态："hit" / "downloaded" / "miss" / "error"。
pub fn record_cache(kind: CacheKind, state: &str) {
    with_context(|ctx| {
        ctx.cache
            .insert(kind.as_str().to_string(), state.to_string());
    });
}

/// 记录一个阶段耗时（毫秒），写入汇总 JSON 的顶层 `*_ms` 字段。
pub fn record_stage(name: &str, ms: f64) {
    if ms.is_finite() && ms >= 0.0 {
        with_context(|ctx| {
            ctx.stages.insert(name.to_string(), json!(ms));
        });
    }
}

/// 记录一个非数值阶段状态，写入汇总 JSON 的顶层字段。
pub fn record_stage_status(name: &str, status: &str) {
    with_context(|ctx| {
        ctx.stages
            .insert(name.to_string(), Value::String(status.to_string()));
    });
}

/// 记录 MP4 编码统计。
pub fn record_video_stats(stats: VideoStats) {
    with_context(|ctx| ctx.video = Some(stats));
}

/// 记录输出文件字节数。
pub fn record_output_bytes(bytes: u64) {
    with_context(|ctx| ctx.output_bytes = Some(bytes));
}

pub(crate) fn snapshot() -> Snapshot {
    with_context(|ctx| Snapshot {
        cache: ctx.cache.clone(),
        stages: ctx.stages.clone(),
        video: ctx.video.clone(),
        output_bytes: ctx.output_bytes,
    })
}

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    with_context(|ctx| *ctx = Context::default());
}
