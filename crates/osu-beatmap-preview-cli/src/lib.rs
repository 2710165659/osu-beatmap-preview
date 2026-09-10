//! CLI 与 native 平台适配层。
//!
//! 本 crate 同时承载命令行解析、文件/网络资源、缓存、配置、媒体编码和导出实现。

use osu_beatmap_preview_core::{
    parse_beatmap_bytes, AudioData, Beatmap, ImageData, ResourceBundle, Result,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub mod adapters;
pub(crate) mod application;
pub(crate) mod cache;
pub mod cli;
pub(crate) mod config;
pub(crate) mod download;
pub(crate) mod export;
pub(crate) mod logging;
pub(crate) mod media;

pub use application::request::{parse_fps, parse_positive_finite, RenderRequest};
pub use application::{
    ExecutionOptions, OutputOptions, RulesetOptions, SourceOptions, ViewOptions,
};
pub use osu_beatmap_preview_core::processing::validation::parse_time_point;
pub use osu_beatmap_preview_core::ErrorKind;
pub use osu_beatmap_preview_core::PreviewError;

pub fn generate_preview(request: RenderRequest) -> Result<serde_json::Value> {
    application::execute(request)
}

pub fn read_bytes(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    std::fs::read(path.as_ref()).map_err(|error| {
        PreviewError::new(format!("读取文件失败 {}: {error}", path.as_ref().display()))
    })
}
pub fn load_beatmap(path: impl AsRef<Path>) -> Result<Beatmap> {
    parse_beatmap_bytes(&read_bytes(path)?)
}

#[derive(Debug, Clone)]
pub struct ResourceLoader {
    pub cache_dir: Option<PathBuf>,
}
impl ResourceLoader {
    pub fn new(cache_dir: Option<PathBuf>) -> Self {
        Self { cache_dir }
    }
    pub fn bundle_from_files(
        &self,
        beatmap_path: impl AsRef<Path>,
        background: Option<(&Path, u32, u32)>,
        audio: Option<(&Path, Option<String>, i64)>,
    ) -> Result<ResourceBundle> {
        let beatmap = load_beatmap(beatmap_path)?;
        let background = background
            .map(|(path, width, height)| {
                Ok(ImageData {
                    width,
                    height,
                    rgba: read_bytes(path)?,
                })
            })
            .transpose()?;
        let audio = audio
            .map(|(path, mime_type, start_time_ms)| {
                Ok(AudioData {
                    bytes: Arc::from(read_bytes(path)?.into_boxed_slice()),
                    mime_type,
                    start_time_ms,
                })
            })
            .transpose()?;
        Ok(ResourceBundle {
            beatmap: Some(beatmap),
            background,
            audio,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
pub trait Logger: Send + Sync {
    fn log(&self, level: LogLevel, message: &str);
}
pub struct StderrLogger;
impl Logger for StderrLogger {
    fn log(&self, level: LogLevel, message: &str) {
        eprintln!("[{level:?}] {message}");
    }
}
