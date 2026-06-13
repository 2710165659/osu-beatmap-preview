//! osu!taiko renderers: multi-row PNG scroll chart and 4-row animated GIF
//! (lazer Overlapping scroll algorithm). Port of beatmap_preview/taiko/*.
//!
//! Re-exports from submodules: [constants], [timing], [notes], [png], [gif].

mod constants;
mod gif;
mod notes;
mod png;
pub(crate) mod timing;

pub(crate) use gif::render_taiko_gif;
pub(crate) use png::render_taiko_grid;
