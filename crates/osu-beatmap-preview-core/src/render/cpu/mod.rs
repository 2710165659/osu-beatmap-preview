//! 四种 ruleset 的纯场景预计算代码。

use std::sync::Arc;

use crate::render::canvas::Img;

pub mod modes;

/// 已完成预计算的动画帧序列，由平台层决定编码为 GIF、视频或其他格式。
#[derive(Clone)]
pub struct AnimationFrames {
    frame_count: usize,
    frame_duration_ms: u32,
    frame_rate: f64,
    render: Arc<dyn Fn(usize) -> Img + Send + Sync>,
    config: Arc<crate::config::CoreConfig>,
}

impl AnimationFrames {
    pub fn new(
        frame_count: usize,
        frame_duration_ms: u32,
        render: impl Fn(usize) -> Img + Send + Sync + 'static,
    ) -> Self {
        Self {
            frame_count,
            frame_duration_ms,
            frame_rate: 1000.0 / f64::from(frame_duration_ms),
            render: Arc::new(render),
            config: crate::config::current(),
        }
    }

    /// 保留精确帧率，供 GIF 编码器分配厘秒延迟，避免逐帧取整积累误差。
    pub fn with_frame_rate(
        frame_count: usize,
        frame_rate: f64,
        render: impl Fn(usize) -> Img + Send + Sync + 'static,
    ) -> Self {
        let mut frames = Self::new(
            frame_count,
            (1000.0 / frame_rate).round().max(1.0) as u32,
            render,
        );
        frames.frame_rate = frame_rate;
        frames
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub fn frame_duration_ms(&self) -> u32 {
        self.frame_duration_ms
    }

    pub fn frame_rate(&self) -> f64 {
        self.frame_rate
    }

    pub fn render(&self, frame_index: usize) -> Img {
        crate::config::with_config(Arc::clone(&self.config), || (self.render)(frame_index))
    }
}
