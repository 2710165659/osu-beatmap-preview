//! 媒体资源处理与视频编码。

#[allow(non_camel_case_types, dead_code)]
mod amf;
pub(crate) mod audio;
pub(crate) mod background_video;
mod cpu;
pub(crate) mod hitsound;
pub(crate) mod image;
mod mux;
#[cfg(windows)]
mod nvenc;
pub(crate) mod storyboard;
mod video;

pub(crate) use background_video::MediaBackground;
pub(crate) use storyboard::MediaStoryboard;
pub(crate) use video::{
    frame_time_ms, resolve_video_time_range, save_mp4_streamed, video_style, FrameComposition,
};
