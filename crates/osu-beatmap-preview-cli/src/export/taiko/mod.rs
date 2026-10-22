//! osu!taiko 渲染器：多行 PNG 滚动谱面和四行 GIF 动画
//!（lazer Overlapping scroll 算法）。移植自 beatmap_preview/taiko/*。
//!
//! 子模块包括动画时序、常量、时间映射、程序化素材及各输出格式渲染器。

use osu_beatmap_preview_core::render::cpu::modes::taiko::constants as taiko_constants;

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
    options: osu_beatmap_preview_core::processing::timeline::GifRenderOptions,
    output_path: &std::path::Path,
    fps: Option<u32>,
    deadline: &osu_beatmap_preview_core::support::timeout::RequestDeadline,
) -> osu_beatmap_preview_core::Result<()> {
    let frames = animation_render::prepare_taiko_gif_frames(beatmap, mods, options, fps, deadline)?;
    if mods.is_some_and(|mods| mods.hidden || mods.flashlight) {
        return crate::media::image::save_animated_gif_with_frame_palettes(
            frames.frame_count(),
            |index| frames.render(index),
            output_path,
            frames.frame_rate(),
            deadline,
        );
    }
    let mut fixed_colors = super::gif_time_label_colors(
        taiko_constants::gif::style::SHOW_TIME_LABEL,
        taiko_constants::gif::style::IMAGE_BACKGROUND,
        [
            taiko_constants::gif::style::PREVIEW_TIME_LABEL_COLOR,
            taiko_constants::gif::style::TIME_LABEL_COLOR,
            taiko_constants::gif::style::TIME_LABEL_NOTE_COLOR,
        ],
    );
    // 规则颜色不能依赖抽样帧是否恰好包含对应物件，AT 与普通预览使用同一套颜色来源。
    fixed_colors.extend([
        constants::CENTRE_NOTE_COLOR,
        constants::RIM_NOTE_COLOR,
        constants::ROLL_COLOR,
        constants::SWELL_COLOR,
    ]);
    fixed_colors.extend(
        [constants::NOTE_RING_COLOR, constants::DRUM_ROLL_TICK_COLOR].map(|[r, g, b, _]| [r, g, b]),
    );
    super::save_animation_frames(frames, output_path, deadline, &fixed_colors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use osu_beatmap_preview_core::processing::timeline::{GifRenderOptions, TimeAxis};
    use osu_beatmap_preview_core::support::timeout::RequestDeadline;
    use std::time::{Duration, Instant};

    #[test]
    fn gif_keeps_rendered_note_and_roll_colours_with_and_without_autoplay() {
        let beatmap = osu_beatmap_preview_core::parse_beatmap_bytes(
            b"osu file format v14\n[General]\nMode:1\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1200,1,8,0:0:0:0:\n256,192,1500,2,0,B|384:192,1,280\n",
        ).unwrap();
        for autoplay in [false, true] {
            let mods = osu_beatmap_preview_core::ModSettings {
                autoplay,
                ..Default::default()
            };
            let options = GifRenderOptions::ResolvedSegments {
                times_ms: vec![1000],
                duration_seconds: Some(0.5),
                time_axis: TimeAxis::new(0),
            };
            let path = std::env::temp_dir().join(format!(
                "osu-preview-taiko-colours-{}-{autoplay}.gif",
                std::process::id()
            ));
            let deadline = RequestDeadline::new(Instant::now(), "gif", Duration::from_secs(60));
            let frames = animation_render::prepare_taiko_gif_frames(
                &beatmap,
                Some(&mods),
                options.clone(),
                Some(10),
                &deadline,
            )
            .unwrap();
            render_taiko_gif(&beatmap, Some(&mods), options, &path, Some(10), &deadline).unwrap();
            let mut decode = gif::DecodeOptions::new();
            decode.set_color_output(gif::ColorOutput::RGBA);
            let mut decoder = decode
                .read_info(std::fs::File::open(&path).unwrap())
                .unwrap();
            let mut decoded = super::super::canvas::Img::new(
                u32::from(decoder.width()),
                u32::from(decoder.height()),
                [0; 4],
            );
            let colors = [
                constants::CENTRE_NOTE_COLOR,
                constants::RIM_NOTE_COLOR,
                constants::ROLL_COLOR,
            ];
            let mut counts = [0; 3];
            for index in 0..frames.frame_count() {
                let frame = decoder.read_next_frame().unwrap().unwrap();
                decoded.alpha_composite(
                    &super::super::canvas::Img {
                        w: u32::from(frame.width),
                        h: u32::from(frame.height),
                        data: frame.buffer.to_vec(),
                    },
                    i64::from(frame.left),
                    i64::from(frame.top),
                );
                for (source, actual) in frames
                    .render(index)
                    .data
                    .chunks_exact(4)
                    .zip(decoded.data.chunks_exact(4))
                {
                    if let Some(color_index) = colors.iter().position(|color| source[..3] == *color)
                    {
                        assert_eq!(source, actual, "AT={autoplay}，帧 {index} 的物件颜色改变");
                        counts[color_index] += 1;
                    }
                }
            }
            assert!(counts.into_iter().all(|count| count > 0));
            drop(decoder);
            std::fs::remove_file(path).unwrap();
        }
    }
}
