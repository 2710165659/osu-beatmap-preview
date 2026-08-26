//! 全局配置：日志目录、文件路径、进程启动时间与开关。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

pub(crate) use crate::config::logging::config::*;

#[derive(Clone)]
pub struct LogConfig {
    pub progress_path: PathBuf,
    pub render_path: PathBuf,
}

static CONFIG: OnceLock<Mutex<Option<LogConfig>>> = OnceLock::new();
static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// 默认日志目录：`<临时目录>/osu-beatmap-preview/logs`。
#[allow(dead_code)] // Runtime initialization is owned by the binary target.
pub fn default_log_dir() -> PathBuf {
    crate::config::resolve_path(crate::config::paths::LOG_DIR)
}

/// 初始化日志（幂等），目录始终来自编译时 `LOG_DIR` 配置。
#[allow(dead_code)]
pub fn init() {
    init_with_dir(default_log_dir());
}

#[allow(dead_code)]
fn init_with_dir(dir: PathBuf) {
    let _ = PROCESS_START.get_or_init(Instant::now);
    {
        let mutex = CONFIG.get_or_init(|| Mutex::new(None));
        let mut guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            return;
        }
        let cfg = LogConfig {
            progress_path: dir.join(PROGRESS_FILE),
            render_path: dir.join(RENDER_FILE),
        };
        match std::fs::create_dir_all(&dir) {
            Ok(()) => *guard = Some(cfg),
            Err(error) => eprintln!(
                "[log] failed to create log dir '{}': {error}; logging disabled",
                dir.display()
            ),
        }
    }
    crate::log::event::event("session-start", "info", None, &session_message());
}

#[cfg(test)]
pub(crate) fn init_for_tests(log_dir: &std::path::Path) {
    init_with_dir(log_dir.to_path_buf());
}

#[allow(dead_code)]
fn session_message() -> String {
    let args: Vec<String> = std::env::args().collect();
    format!(
        "version={} build={} args={}",
        crate::log::APP_VERSION,
        crate::log::BUILD_TIMESTAMP,
        serde_json::to_string(&args).unwrap_or_else(|_| "[]".to_string())
    )
}

/// 当前生效的日志配置（未初始化或已禁用时为 `None`）。
pub(crate) fn enabled() -> Option<LogConfig> {
    CONFIG
        .get()
        .and_then(|mutex| mutex.lock().ok())
        .and_then(|guard| guard.clone())
}

/// 返回 (progress.log, render.log) 两个文件的路径。
#[allow(dead_code)]
pub fn paths() -> Option<(PathBuf, PathBuf)> {
    enabled().map(|cfg| (cfg.progress_path, cfg.render_path))
}

/// 进程启动至今的毫秒数（未初始化时返回 0）。
pub(crate) fn process_elapsed_ms() -> f64 {
    PROCESS_START
        .get()
        .map(|t| t.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    let mutex = CONFIG.get_or_init(|| Mutex::new(None));
    *mutex.lock().unwrap_or_else(|e| e.into_inner()) = None;
}
