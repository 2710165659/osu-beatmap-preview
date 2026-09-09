//! 纯场景生成器；WGPU 设备和纹理绘制由 renderer crate 负责。

pub mod modes;

pub mod composition;
mod realtime;

pub use modes::catch::prepare_realtime as prepare_catch;
pub use modes::mania::prepare_realtime as prepare_mania;
pub use modes::standard::prepare_realtime as prepare_standard;
pub use modes::taiko::prepare_realtime as prepare_taiko;
pub use realtime::RealtimeFrameSource;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoStyle {
    pub enable_background_image: bool,
    pub background_dim: f64,
    pub label_color: [u8; 4],
    pub label_font_size: u32,
    pub label_pad: i64,
    pub black_opaque: [u8; 4],
    pub fallback_background: [u8; 4],
}

impl Default for VideoStyle {
    fn default() -> Self {
        Self {
            enable_background_image: true,
            background_dim: 0.7,
            label_color: [255, 255, 255, 255],
            label_font_size: 18,
            label_pad: 16,
            black_opaque: [0, 0, 0, 255],
            fallback_background: [0, 0, 0, 255],
        }
    }
}

pub(crate) fn game_mode(mode: crate::RealtimeMode) -> crate::render::geometry::GameMode {
    match mode {
        crate::RealtimeMode::Standard => crate::render::geometry::GameMode::Standard,
        crate::RealtimeMode::Taiko => crate::render::geometry::GameMode::Taiko,
        crate::RealtimeMode::Catch => crate::render::geometry::GameMode::Catch,
        crate::RealtimeMode::Mania => crate::render::geometry::GameMode::Mania,
    }
}
