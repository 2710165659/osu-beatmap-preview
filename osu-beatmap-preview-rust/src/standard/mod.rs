//! osu!standard renderer: per-frame 512×384 gameplay snapshots composed into a
//! PNG grid (5×8) or animated GIF (2×2 segments). Port of the Python renderer
//! with identical constants, alpha curves and layout.

mod alpha;
mod constants;
pub(crate) mod context;
mod gif;
mod objects;
mod png;
pub(crate) mod slider;

pub(crate) use gif::render_standard_gif;
pub(crate) use png::render_standard_png;
