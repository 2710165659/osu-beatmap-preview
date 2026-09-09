//! 四种 ruleset 的纯场景预计算代码。

use std::sync::Arc;

use crate::render::canvas::Img;

pub mod modes;

/// 已完成预计算的动画帧序列，由平台层决定编码为 GIF、视频或其他格式。
#[derive(Clone)]
pub struct AnimationFrames {
    frame_count: usize,
    frame_duration_ms: u32,
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
            render: Arc::new(render),
            config: crate::config::current(),
        }
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub fn frame_duration_ms(&self) -> u32 {
        self.frame_duration_ms
    }

    pub fn render(&self, frame_index: usize) -> Img {
        crate::config::with_config(Arc::clone(&self.config), || (self.render)(frame_index))
    }
}
