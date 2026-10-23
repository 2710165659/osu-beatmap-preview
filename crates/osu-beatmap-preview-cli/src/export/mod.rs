//! CLI 导出模块：组织四模式的 PNG、GIF 和 MP4 产物生成。

pub(crate) use osu_beatmap_preview_core::render::canvas;
pub(crate) mod catch;
pub(crate) use osu_beatmap_preview_core::render::geometry;
pub(crate) mod mania;
#[cfg(test)]
pub(crate) use osu_beatmap_preview_core::render::scene;
pub(crate) mod replay;
pub(crate) mod standard;
pub(crate) mod taiko;
pub(crate) use osu_beatmap_preview_core::render::text;
pub(crate) use osu_beatmap_preview_core::render::timing;
#[cfg(test)]
mod rasterizer;
pub(crate) mod segment;

fn save_animation_frames(
    frames: osu_beatmap_preview_core::render::cpu::AnimationFrames,
    output_path: &std::path::Path,
    deadline: &osu_beatmap_preview_core::support::timeout::RequestDeadline,
    label_colors: &[[u8; 3]],
) -> osu_beatmap_preview_core::Result<()> {
    let frame_count = frames.frame_count();
    let frame_rate = frames.frame_rate();
    crate::media::image::save_animated_gif_streamed_with_colors(
        frame_count,
        move |frame_index| frames.render(frame_index),
        output_path,
        frame_rate,
        deadline,
        label_colors,
    )
}

fn gif_time_label_colors(
    show_time_label: bool,
    background: [u8; 4],
    colors: [[u8; 4]; 3],
) -> Vec<[u8; 3]> {
    if !show_time_label {
        return Vec::new();
    }
    // 各模式标签所在的画布底色不同，预留实际合成色才能保留半透明配置的表现。
    colors
        .map(|color| {
            let mut pixel = canvas::Img::new(1, 1, background);
            pixel.blend_px(0, 0, color);
            [pixel.data[0], pixel.data[1], pixel.data[2]]
        })
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use osu_beatmap_preview_core::processing::timeline::{GifRenderOptions, TimeAxis};
    use osu_beatmap_preview_core::support::timeout::RequestDeadline;
    use std::time::{Duration, Instant};

    #[test]
    fn non_standard_gifs_preserve_rendered_time_label_colors() {
        for mode in 1..=3 {
            let source = format!(
                "osu file format v14\n[General]\nMode:{mode}\nPreviewTime:1000\n\
                 [Difficulty]\nCircleSize:4\nApproachRate:5\nOverallDifficulty:5\n\
                 SliderMultiplier:1.4\nSliderTickRate:1\n\
                 [TimingPoints]\n0,500,4,1,0,100,1,0\n\
                 [HitObjects]\n64,192,1000,1,0,0:0:0:0:\n448,192,2000,1,0,0:0:0:0:\n"
            );
            let beatmap = osu_beatmap_preview_core::parse_beatmap_bytes(source.as_bytes()).unwrap();
            let options = GifRenderOptions::ResolvedSegments {
                times_ms: vec![0, 1000],
                duration_seconds: Some(0.2),
                time_axis: TimeAxis::new(0),
            };
            let path = std::env::temp_dir().join(format!(
                "osu-preview-mode-{mode}-label-colors-{}.gif",
                std::process::id()
            ));
            let deadline = RequestDeadline::new(Instant::now(), "gif", Duration::from_secs(60));
            let frames = match mode {
                1 => taiko::animation_render::prepare_taiko_gif_frames,
                2 => catch::animation::prepare_catch_gif_frames,
                _ => mania::animation::prepare_mania_gif_frames,
            }(&beatmap, None, options.clone(), Some(10), &deadline)
            .unwrap();
            let render = match mode {
                1 => taiko::render_taiko_gif,
                2 => catch::render_catch_gif,
                _ => mania::render_mania_gif,
            };
            render(&beatmap, None, options, &path, Some(10), &deadline).unwrap();

            let mut decode = gif::DecodeOptions::new();
            decode.set_color_output(gif::ColorOutput::RGBA);
            let mut decoder = decode
                .read_info(std::fs::File::open(&path).unwrap())
                .unwrap();
            let mut decoded = canvas::Img::new(
                u32::from(decoder.width()),
                u32::from(decoder.height()),
                [0; 4],
            );
            for index in 0..frames.frame_count() {
                let frame = decoder.read_next_frame().unwrap().unwrap();
                decoded.alpha_composite(
                    &canvas::Img {
                        w: u32::from(frame.width),
                        h: u32::from(frame.height),
                        data: frame.buffer.to_vec(),
                    },
                    i64::from(frame.left),
                    i64::from(frame.top),
                );
                let expected = frames.render(index);
                // 同时验证普通时间和 Preview Time，并重建差分帧检查后续帧没有颜色漂移。
                for color in [[95, 221, 108, 255], [232, 232, 232, 255]] {
                    let mut count = 0;
                    for (source, actual) in expected
                        .data
                        .chunks_exact(4)
                        .zip(decoded.data.chunks_exact(4))
                    {
                        if source == color {
                            assert_eq!(actual, color, "模式 {mode} 第 {index} 帧标签颜色改变");
                            count += 1;
                        }
                    }
                    assert!(count > 0, "模式 {mode} 必须实际绘制标签颜色 {color:?}");
                }
            }
            assert!(decoder.read_next_frame().unwrap().is_none());
            drop(decoder);
            std::fs::remove_file(path).unwrap();
        }
    }
}
