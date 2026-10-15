//! 回放 GIF 分块并行绘制玩法层，再按顺序消费背景视频；不保留整段 RGBA。
use crate::export::canvas::Img;
use crate::media::{
    background_video::FrameBackgrounds, composition::compose_replay_frame, MediaBackground,
    MediaStoryboard,
};
use osu_beatmap_preview_core::{support::timeout::RequestDeadline, Result};
use rayon::prelude::*;
use std::{collections::VecDeque, path::Path};

#[allow(clippy::too_many_arguments)]
pub(super) fn save(
    times: &[i64],
    draw: impl Fn(usize) -> Img + Send + Sync,
    overlay: impl Fn(&mut Img, i64),
    width: u32,
    height: u32,
    background: MediaBackground,
    storyboard: Option<&MediaStoryboard>,
    path: &Path,
    fps: u32,
    deadline: &RequestDeadline,
) -> Result<()> {
    let style = crate::media::video_style(crate::export::geometry::GameMode::Standard);
    let mut backgrounds = FrameBackgrounds::new(background, width, height, style, times);
    let config = crate::config::current();
    let frame_bytes = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4)
        .max(1);
    let chunk = (config.advance.gif.MAX_PAR_FRAME_BYTES / frame_bytes)
        .clamp(1, config.advance.gif.PAR_CHUNK_SIZE);
    let mut frames = VecDeque::new();
    crate::media::image::save_animated_gif_with_frame_palettes(
        times.len(),
        |index| {
            if frames.is_empty() {
                frames = (index..(index + chunk).min(times.len()))
                    .into_par_iter()
                    .map(&draw)
                    .collect::<Vec<_>>()
                    .into();
            }
            let image = frames
                .pop_front()
                .expect("顺序 GIF 消费者每次读取当前分块一帧");
            let time = times[index];
            let background = backgrounds.background_at_replay(time);
            let mut image = compose_replay_frame(
                image,
                0,
                time,
                0,
                width,
                height,
                background.as_deref(),
                style,
                storyboard,
            );
            overlay(&mut image, time);
            image
        },
        path,
        f64::from(fps),
        deadline,
    )
}
