//! WGPU 离屏渲染和流式回读共用的运行时类型。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use thiserror::Error;

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
            msaa_samples: 1,
            target_fps: 60,
            max_in_flight: 2,
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
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Self, RendererError> {
        let expected = width as usize * height as usize * 4;
        if data.len() != expected {
            return Err(RendererError::InvalidFrame(format!(
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
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum RendererError {
    #[error("invalid WGPU export request: {0}")]
    InvalidRequest(String),
    #[error("WGPU adapter unavailable: {0}")]
    GpuUnavailable(String),
    #[error("WGPU device error: {0}")]
    Device(String),
    #[error("invalid frame: {0}")]
    InvalidFrame(String),
    #[error("scene error: {0}")]
    Scene(String),
    #[error("canvas too small: {width}x{height}, required {required_width}x{required_height}")]
    CanvasTooSmall {
        width: u32,
        height: u32,
        required_width: u32,
        required_height: u32,
    },
    #[error("duplicate frame index: {0}")]
    DuplicateFrameIndex(u64),
    #[error("WGPU export cancelled")]
    Cancelled,
    #[error("WGPU export callback failed: {0}")]
    Callback(String),
}
