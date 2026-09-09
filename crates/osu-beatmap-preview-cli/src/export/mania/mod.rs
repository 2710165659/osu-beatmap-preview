//! osu!mania 渲染器：纵向多列 PNG 谱面和四段 GIF。
//! 移植自 beatmap_preview/mania/{renderer,gif_renderer,skin,config}.py。

pub(crate) use osu_beatmap_preview_core::render::cpu::modes::mania::{
    animation, constants, skin, utils,
};
mod png;
mod video;

pub(crate) use png::render_mania_grid;
pub(crate) use utils::*;
pub(crate) use video::render_mania_video;

pub(crate) fn render_mania_gif(
    beatmap: &osu_beatmap_preview_core::Beatmap,
    mods: Option<&osu_beatmap_preview_core::ModSettings>,
    options: osu_beatmap_preview_core::domain::shared::time_selection::GifRenderOptions,
    output_path: &std::path::Path,
    fps: Option<u32>,
    deadline: &osu_beatmap_preview_core::domain::timeout::RequestDeadline,
) -> osu_beatmap_preview_core::Result<()> {
    let frames = animation::prepare_mania_gif_frames(beatmap, mods, options, fps, deadline)?;
    super::save_animation_frames(frames, output_path, deadline)
}
