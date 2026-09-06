//! 实时 API 的公开数据类型。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaPolicy {
    #[default]
    None,
    Background,
    AudioAndBackground,
}

#[derive(Debug, Clone)]
pub struct RealtimeRequest {
    pub bid: String,
    pub convert: Option<String>,
    pub mods: Vec<String>,
    pub config: Option<String>,
    pub scale: Option<f64>,
    pub no_cache: bool,
    pub media_policy: MediaPolicy,
}

impl RealtimeRequest {
    pub fn new(bid: impl Into<String>) -> Self {
        Self {
            bid: bid.into(),
            convert: None,
            mods: Vec::new(),
            config: None,
            scale: None,
            no_cache: false,
            media_policy: MediaPolicy::None,
        }
    }
}

impl From<crate::RenderRequest> for RealtimeRequest {
    fn from(request: crate::RenderRequest) -> Self {
        Self {
            bid: request.source.bid,
            convert: request.ruleset.convert,
            mods: request.ruleset.mods,
            config: request.execution.config,
            scale: request.output.scale,
            no_cache: request.execution.no_cache,
            media_policy: MediaPolicy::AudioAndBackground,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeMode {
    Standard,
    Taiko,
    Catch,
    Mania,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineInfo {
    pub absolute_start_ms: i64,
    pub first_object_ms: i64,
    pub last_object_ms: i64,
    pub duration_ms: i64,
    pub beatmap_speed: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OffscreenConfig {
    pub width: u32,
    pub height: u32,
    pub msaa_samples: u32,
    pub target_fps: u32,
    pub max_in_flight: usize,
}

impl Default for OffscreenConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            msaa_samples: 4,
            target_fps: 30,
            max_in_flight: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererInfo {
    pub adapter_name: String,
    pub backend: String,
    pub msaa_samples: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaFrame {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl RgbaFrame {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    pub(crate) fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Self, RealtimeError> {
        let expected = width as usize * height as usize * 4;
        if data.len() != expected {
            return Err(RealtimeError::InvalidFrame(format!(
                "RGBA frame has {} bytes, expected {expected}",
                data.len()
            )));
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRequest {
    pub index: u64,
    pub absolute_time_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedFrame {
    pub index: u64,
    pub absolute_time_ms: i64,
    pub rgba: RgbaFrame,
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum RealtimeError {
    #[error("invalid realtime request: {0}")]
    InvalidRequest(String),
    #[error("failed to load realtime session: {0}")]
    Load(String),
    #[error("no compatible GPU adapter is available: {0}")]
    GpuUnavailable(String),
    #[error("GPU device error: {0}")]
    Device(String),
    #[error("failed to build frame scene: {0}")]
    Scene(String),
    #[error("invalid RGBA frame: {0}")]
    InvalidFrame(String),
    #[error("offscreen canvas is {width}x{height}, but the scene requires at least {required_width}x{required_height}")]
    CanvasTooSmall {
        width: u32,
        height: u32,
        required_width: u32,
        required_height: u32,
    },
    #[error("duplicate frame index {0}")]
    DuplicateFrameIndex(u64),
    #[error("rendering was cancelled")]
    Cancelled,
    #[error("frame callback failed: {0}")]
    Callback(String),
}

impl From<crate::PreviewError> for RealtimeError {
    fn from(error: crate::PreviewError) -> Self {
        match error.kind() {
            crate::ErrorKind::Render => Self::Scene(error.to_string()),
            _ => Self::Load(error.to_string()),
        }
    }
}
