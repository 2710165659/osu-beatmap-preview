//! 场景后端接口。现有 CPU 渲染器作为兼容实现，后续 WGPU 后端实现同一契约。

use crate::domain::errors::Result;
use crate::render::canvas::Img;
use crate::render::scene::FrameScene;

#[allow(dead_code)]
pub(crate) trait FrameBackend: Send + Sync {
    fn render_frame(&self, scene: &FrameScene) -> Result<Img>;
}
