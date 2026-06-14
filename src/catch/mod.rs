//! osu!catch renderers: render-object expansion (fruits, juice streams,
//! banana showers, HR offsets, hyperdash) plus PNG grid and GIF previews.
//! RNG call order mirrors the Python/stable implementations exactly.

mod constants;
mod drawing;
mod gif;
pub(crate) mod objects;
mod png;

pub(crate) use gif::render_catch_gif;
pub(crate) use png::render_catch_grid;
