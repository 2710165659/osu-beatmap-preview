//! 汇总记录：每张谱面渲染完写一行 NDJSON 到 `render.log`。

use crate::log::config::{enabled, process_elapsed_ms};
use crate::log::context::{snapshot, CacheKind};
use crate::log::timestamp::now_local_millis;
use crate::log::writer::append_line;
use serde_json::{json, Map, Value};

/// 一张谱面渲染的汇总字段（由调用方逐步填充）。
#[derive(Debug, Default, Clone)]
pub struct SummaryRecord {
    /// 状态：success / error / cache-hit。
    pub status: String,
    pub bid: String,
    /// 渲染流水线总耗时（毫秒）
    pub duration_ms: f64,
    pub error: Option<String>,
    pub error_kind: Option<String>,
    // 输入
    pub fmt: Option<String>,
    pub mode: Option<i32>,
    pub target_mode: Option<i32>,
    pub convert: Option<String>,
    pub mods: Option<String>,
    pub time_points: Option<Vec<String>>,
    pub duration_time: Option<f64>,
    pub no_cache: bool,
    // 谱面
    pub set_id: Option<u64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub version: Option<String>,
    pub hit_object_count: Option<usize>,
    pub chart_duration_ms: Option<i64>,
    pub bpm: Option<f64>,
    pub ar: Option<f64>,
    pub cs: Option<f64>,
    pub hp: Option<f64>,
    pub od: Option<f64>,
    pub format_version: Option<i32>,
    pub osu_bytes: Option<u64>,
    // 阶段（其余阶段经上下文 record_stage / video stats 合并）
    pub download_osu_ms: Option<f64>,
    pub parse_ms: Option<f64>,
    pub render_ms: Option<f64>,
}

/// 把汇总记录写为一行 NDJSON（合并进程上下文中的缓存、阶段与视频统计）。
pub fn write_summary(rec: &SummaryRecord) {
    let Some(cfg) = enabled() else {
        return;
    };
    let ctx = snapshot();
    let mut map = Map::new();

    map.insert("ts".to_string(), Value::String(now_local_millis()));
    map.insert("pid".to_string(), json!(std::process::id()));
    map.insert("bid".to_string(), Value::String(rec.bid.clone()));
    map.insert("status".to_string(), Value::String(rec.status.clone()));
    map.insert("duration_ms".to_string(), json!(round1(rec.duration_ms)));
    map.insert("total_ms".to_string(), json!(round1(process_elapsed_ms())));
    map.insert(
        "app_version".to_string(),
        Value::String(crate::log::APP_VERSION.to_string()),
    );
    map.insert(
        "build_time".to_string(),
        Value::String(crate::log::BUILD_TIMESTAMP.to_string()),
    );
    if let Ok(cores) = std::thread::available_parallelism() {
        map.insert("cores".to_string(), json!(cores.get()));
    }

    insert_opt(&mut map, "fmt", rec.fmt.as_deref());
    insert_opt(&mut map, "mode", rec.mode);
    insert_opt(&mut map, "target_mode", rec.target_mode);
    insert_opt(&mut map, "convert", rec.convert.as_deref());
    insert_opt(&mut map, "mods", rec.mods.as_deref());
    if let Some(points) = &rec.time_points {
        map.insert("time_points".to_string(), json!(points));
    }
    insert_opt(&mut map, "duration_time", rec.duration_time);
    if rec.no_cache {
        map.insert("no_cache".to_string(), json!(true));
    }

    insert_opt(&mut map, "set_id", rec.set_id);
    insert_opt(&mut map, "title", rec.title.as_deref());
    insert_opt(&mut map, "artist", rec.artist.as_deref());
    insert_opt(&mut map, "version", rec.version.as_deref());
    insert_opt(&mut map, "hit_object_count", rec.hit_object_count);
    insert_opt(&mut map, "chart_duration_ms", rec.chart_duration_ms);
    insert_opt(&mut map, "bpm", rec.bpm);
    insert_opt(&mut map, "ar", rec.ar);
    insert_opt(&mut map, "cs", rec.cs);
    insert_opt(&mut map, "hp", rec.hp);
    insert_opt(&mut map, "od", rec.od);
    insert_opt(&mut map, "format_version", rec.format_version);
    insert_opt(&mut map, "osu_bytes", rec.osu_bytes);

    insert_opt(&mut map, "download_osu_ms", rec.download_osu_ms.map(round1));
    insert_opt(&mut map, "parse_ms", rec.parse_ms.map(round1));
    insert_opt(&mut map, "render_ms", rec.render_ms.map(round1));
    for (name, value) in &ctx.stages {
        let value = value
            .as_f64()
            .map(|ms| json!(round1(ms)))
            .unwrap_or_else(|| value.clone());
        map.insert(name.clone(), value);
    }

    if let Some(video) = &ctx.video {
        insert_opt(&mut map, "backend", video.backend.as_deref());
        insert_opt(&mut map, "resolution", video.resolution.as_deref());
        insert_opt(&mut map, "fps", video.fps);
        insert_opt(&mut map, "frame_count", video.frame_count);
        insert_opt(&mut map, "video_ms", video.video_ms);
        insert_opt(&mut map, "render_compose_ms", video.render_compose_ms);
        insert_opt(&mut map, "encode_ms", video.encode_ms);
        insert_opt(&mut map, "mux_ms", video.mux_ms);
        insert_opt(&mut map, "audio_ms", video.audio_ms);
    }

    if let Some(bytes) = ctx.output_bytes {
        map.insert("output_bytes".to_string(), json!(bytes));
    }

    let mut cache = Map::new();
    for kind in [
        CacheKind::Osu,
        CacheKind::Osz,
        CacheKind::Audio,
        CacheKind::Output,
    ] {
        if let Some(state) = ctx.cache.get(kind.as_str()) {
            cache.insert(kind.as_str().to_string(), Value::String(state.clone()));
        }
    }
    if !cache.is_empty() {
        map.insert("cache".to_string(), Value::Object(cache));
    }

    if let Some(error) = &rec.error {
        map.insert(
            "error".to_string(),
            Value::String(truncate_chars(error, 2000)),
        );
    }
    if let Some(kind) = &rec.error_kind {
        map.insert("error_kind".to_string(), Value::String(kind.clone()));
    }

    let line = serde_json::to_string(&Value::Object(map)).unwrap_or_default();
    append_line(&cfg.render_path, &line);
}

fn insert_opt<V: Into<Value>>(map: &mut Map<String, Value>, key: &str, value: Option<V>) {
    if let Some(value) = value {
        map.insert(key.to_string(), value.into());
    }
}

fn round1(ms: f64) -> f64 {
    if ms.is_finite() {
        (ms * 10.0).round() / 10.0
    } else {
        0.0
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}
