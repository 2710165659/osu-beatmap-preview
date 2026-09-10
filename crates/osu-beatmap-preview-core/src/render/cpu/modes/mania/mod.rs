//! osu!mania 渲染器：纵向多列 PNG 谱面和四段 GIF。
//! 移植自 beatmap_preview/mania/{renderer,gif_renderer,skin,config}.py。

pub mod animation;
pub mod constants;
pub mod png;
pub mod skin;
pub mod utils;

pub use utils::*;
