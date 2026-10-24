//! osu!standard 渲染器：将每帧 512×384 游戏画面合成为 PNG 网格（5×8）
//! 或 GIF 动画（2×2 分段）。移植自 Python 渲染器，常量、alpha 曲线与布局保持一致。

pub(crate) use osu_beatmap_preview_core::render::cpu::modes::standard::context;
mod animation;
pub(crate) mod osr;
mod png;
mod video;

pub(crate) use animation::render_standard_gif;
pub(crate) use osu_beatmap_preview_core::render::cpu::modes::standard::render_frame;
pub(crate) use png::render_standard_png;
pub(crate) use video::render_standard_video;

pub(crate) use crate::export::text::draw_time_label;
