//! osu!taiko 渲染器：多行 PNG 滚动谱面和四行 GIF 动画
//!（lazer Overlapping scroll 算法）。移植自 beatmap_preview/taiko/*。
//!
//! 子模块包括动画时序、常量、时间映射、程序化素材及各输出格式渲染器。

mod animation;
mod constants;
pub(crate) mod conv;
mod gif;
mod notes;
mod png;
pub(crate) mod timing;
mod video;

pub(crate) use gif::render_taiko_gif;
pub(crate) use png::render_taiko_grid;
pub(crate) use video::render_taiko_video;
