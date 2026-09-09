//! osu!taiko 渲染器：多行 PNG 滚动谱面和四行 GIF 动画
//!（lazer Overlapping scroll 算法）。移植自 beatmap_preview/taiko/*。
//!
//! 子模块包括动画时序、常量、时间映射、程序化素材及各输出格式渲染器。

pub(crate) use osu_beatmap_preview_core::render::cpu::modes::taiko::{
    animation_render, constants, notes, timing,
};
mod png;
mod video;

pub(crate) use png::render_taiko_grid;
pub(crate) use video::render_taiko_video;

pub(crate) fn render_taiko_gif(
    beatmap: &osu_beatmap_preview_core::Beatmap,
    mods: Option<&osu_beatmap_preview_core::ModSettings>,
    options: osu_beatmap_preview_core::domain::shared::time_selection::GifRenderOptions,
    output_path: &std::path::Path,
    fps: Option<u32>,
    deadline: &osu_beatmap_preview_core::domain::timeout::RequestDeadline,
) -> osu_beatmap_preview_core::Result<()> {
    let frames = animation_render::prepare_taiko_gif_frames(beatmap, mods, options, fps, deadline)?;
    super::save_animation_frames(frames, output_path, deadline)
}
