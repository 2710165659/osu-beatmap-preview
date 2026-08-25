//! 全局配置：日志目录、文件路径、进程启动时间与开关。

use std::path::{Path, PathBuf};
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
pub fn default_log_dir() -> PathBuf {
    std::env::temp_dir()
        .join("osu-beatmap-preview")
        .join("logs")
}

/// 初始化日志（幂等）。`log_dir` 为空时依次回退到 `OSU_PREVIEW_LOG_DIR`
/// 环境变量与默认目录。目录创建失败时禁用日志，不影响主流程。
pub fn init(log_dir: Option<&Path>) {
    let _ = PROCESS_START.get_or_init(Instant::now);
    {
        let mutex = CONFIG.get_or_init(|| Mutex::new(None));
        let mut guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            return;
        }
        let dir = match log_dir {
            Some(dir) => dir.to_path_buf(),
            None => env_log_dir().unwrap_or_else(default_log_dir),
        };
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

fn env_log_dir() -> Option<PathBuf> {
    std::env::var_os("OSU_PREVIEW_LOG_DIR").map(PathBuf::from)
}

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
