//! osu!taiko 渲染器：多行 PNG 滚动谱面和四行 GIF 动画
//!（lazer Overlapping scroll 算法）。移植自 beatmap_preview/taiko/*。
//!
//! 从子模块重新导出：[constants]、[timing]、[notes]、[png]、[gif]。

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
