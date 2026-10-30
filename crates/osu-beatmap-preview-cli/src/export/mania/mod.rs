//! osu!mania 渲染器：纵向多列 PNG 谱面和四段 GIF。
//! 移植自 beatmap_preview/mania/{renderer,gif_renderer,skin,config}.py。

use osu_beatmap_preview_core::render::cpu::modes::mania::constants as mania_constants;

pub(crate) use osu_beatmap_preview_core::render::cpu::modes::mania::{
    animation, constants, skin, utils,
};
pub(crate) mod osr;
mod png;
mod video;

pub(crate) use png::render_mania_grid;
pub(crate) use utils::*;
pub(crate) use video::render_mania_video;

pub(crate) fn render_mania_gif(
    beatmap: &osu_beatmap_preview_core::Beatmap,
    mods: Option<&osu_beatmap_preview_core::ModSettings>,
    options: osu_beatmap_preview_core::processing::timeline::GifRenderOptions,
    output_path: &std::path::Path,
    fps: Option<u32>,
    deadline: &osu_beatmap_preview_core::support::timeout::RequestDeadline,
) -> osu_beatmap_preview_core::Result<()> {
    let frames = animation::prepare_mania_gif_frames(beatmap, mods, options, fps, deadline)?;
    let label_colors = super::gif_time_label_colors(
        mania_constants::gif::style::SHOW_TIME_LABEL,
        mania_constants::gif::style::IMAGE_BACKGROUND,
        [
            mania_constants::gif::style::PREVIEW_TIME_LABEL_COLOR,
            mania_constants::gif::style::TIME_LABEL_COLOR,
            mania_constants::gif::style::TIME_LABEL_NOTE_COLOR,
        ],
    );
    super::save_animation_frames(frames, output_path, deadline, &label_colors, false)
}
