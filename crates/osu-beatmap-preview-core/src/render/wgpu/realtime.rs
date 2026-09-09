//! 四模式 WGPU 实时场景共享的已准备帧源。

use std::sync::Arc;

use crate::domain::errors::Result;
use crate::render::geometry::GameMode;
use crate::render::scene::FrameScene;

#[derive(Clone)]
pub struct RealtimeFrameSource {
    pub mode: GameMode,
    render: Arc<dyn Fn(i64) -> Result<FrameScene> + Send + Sync>,
}

impl std::fmt::Debug for RealtimeFrameSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RealtimeFrameSource")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

impl RealtimeFrameSource {
    pub fn new(
        mode: GameMode,
        render: impl Fn(i64) -> Result<FrameScene> + Send + Sync + 'static,
    ) -> Self {
        Self {
            mode,
            render: Arc::new(render),
        }
    }

    pub fn render(&self, absolute_time_ms: i64) -> Result<FrameScene> {
        (self.render)(absolute_time_ms)
    }
}
