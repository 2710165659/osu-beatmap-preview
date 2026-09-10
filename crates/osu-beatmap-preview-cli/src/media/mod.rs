//! 媒体资源处理与视频编码。

#[allow(non_camel_case_types, dead_code)]
mod amf;
pub(crate) mod audio;
mod cpu;
pub(crate) mod image;
mod mux;
#[cfg(windows)]
mod nvenc;
mod video;

pub(crate) use video::{
    resolve_video_time_range, save_mp4_streamed, video_style, FrameComposition,
};
