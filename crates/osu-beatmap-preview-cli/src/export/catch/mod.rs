//! osu!catch 渲染器：展开水果、果汁流、香蕉雨、HR 偏移和 hyperdash 等渲染对象，
//! 并输出 PNG 网格和 GIF 预览。RNG 调用顺序严格匹配 Python/stable 实现。

pub(crate) use osu_beatmap_preview_core::render::cpu::modes::catch::{
    animation, constants, objects,
};
mod png;
mod video;

pub(crate) use png::render_catch_grid;
pub(crate) use video::render_catch_video;

pub(crate) fn render_catch_gif(
    beatmap: &osu_beatmap_preview_core::Beatmap,
    mods: Option<&osu_beatmap_preview_core::ModSettings>,
    options: osu_beatmap_preview_core::processing::timeline::GifRenderOptions,
    output_path: &std::path::Path,
    fps: Option<u32>,
    deadline: &osu_beatmap_preview_core::support::timeout::RequestDeadline,
) -> osu_beatmap_preview_core::Result<()> {
    let frames = animation::prepare_catch_gif_frames(beatmap, mods, options, fps, deadline)?;
    super::save_animation_frames(frames, output_path, deadline)
}
