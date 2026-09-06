//! 运行时无关的实时场景与 WGPU 离屏渲染 API。

mod session;
mod types;

pub use crate::render::scene::FrameScene;
pub use crate::render::wgpu::OffscreenRenderer;
pub use session::RealtimeSession;
pub use types::{
    CancellationToken, FrameRequest, MediaPolicy, OffscreenConfig, RealtimeError, RealtimeMode,
    RealtimeRequest, RenderedFrame, RendererInfo, RgbaFrame, TimelineInfo,
};

pub(crate) fn configured_offscreen(mode: crate::render::geometry::GameMode) -> OffscreenConfig {
    let render = &crate::infrastructure::config::current().render;
    macro_rules! make {
        ($config:expr) => {{
            let config = $config;
            OffscreenConfig {
                width: config.WIDTH as u32,
                height: config.HEIGHT as u32,
                msaa_samples: config.MSAA_SAMPLES as u32,
                target_fps: config.TARGET_FPS as u32,
                max_in_flight: config.MAX_IN_FLIGHT as usize,
            }
        }};
    }
    match mode {
        crate::render::geometry::GameMode::Standard => make!(&render.standard.wgpu),
        crate::render::geometry::GameMode::Taiko => make!(&render.taiko.wgpu),
        crate::render::geometry::GameMode::Catch => make!(&render.catch.wgpu),
        crate::render::geometry::GameMode::Mania => make!(&render.mania.wgpu),
    }
}

#[cfg(feature = "wgpu-player")]
pub mod player;
