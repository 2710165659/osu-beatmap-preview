//! CLI 导出模块：组织四模式的 PNG、GIF 和 MP4 产物生成。

pub(crate) use osu_beatmap_preview_core::render::canvas;
pub(crate) mod catch;
pub(crate) use osu_beatmap_preview_core::render::geometry;
pub(crate) mod mania;
#[cfg(test)]
pub(crate) use osu_beatmap_preview_core::render::scene;
pub(crate) mod standard;
pub(crate) mod taiko;
pub(crate) use osu_beatmap_preview_core::render::text;
pub(crate) use osu_beatmap_preview_core::render::timing;
#[cfg(test)]
mod rasterizer;

fn save_animation_frames(
    frames: osu_beatmap_preview_core::render::cpu::AnimationFrames,
    output_path: &std::path::Path,
    deadline: &osu_beatmap_preview_core::support::timeout::RequestDeadline,
) -> osu_beatmap_preview_core::Result<()> {
    let frame_count = frames.frame_count();
    let frame_duration_ms = frames.frame_duration_ms();
    crate::media::image::save_animated_gif_streamed(
        frame_count,
        move |frame_index| frames.render(frame_index),
        output_path,
        frame_duration_ms,
        deadline,
    )
}
