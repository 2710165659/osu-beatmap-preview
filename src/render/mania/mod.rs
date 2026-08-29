//! osu!mania renderers: vertical multi-column PNG chart and 4-segment GIF.
//! Port of beatmap_preview/mania/{renderer,gif_renderer,skin,config}.py.

mod constants;
pub(crate) mod conv;
mod gif;
mod png;
mod skin;
mod utils;
mod video;

pub(crate) use gif::render_mania_gif;
pub(crate) use png::render_mania_grid;
pub(crate) use utils::*;
pub(crate) use video::render_mania_video;
