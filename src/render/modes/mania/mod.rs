//! osu!mania 渲染器：纵向多列 PNG 谱面和四段 GIF。
//! 移植自 beatmap_preview/mania/{renderer,gif_renderer,skin,config}.py。

mod animation;
pub(crate) mod constants;
mod png;
mod skin;
mod utils;
mod video;

pub(crate) use animation::render_mania_gif;
pub(crate) use png::render_mania_grid;
pub(crate) use utils::*;
pub(crate) use video::render_mania_video;
