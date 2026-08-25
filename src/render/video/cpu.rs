//! CPU H.264 encoder backend using openh264 (Cisco's open-source encoder,
//! compiled in via the `source` feature). This is the always-available
//! fallback when no GPU encoder (NVENC / AMF) can be initialized.
//!
//! Uses bitrate mode with a bounded QP. Rendering and encoding are performed in
//! separate phases, so the encoder can use openh264's automatic thread count
//! without competing with the Rayon render pool.

use crate::core::errors::{PreviewError, Result};
use crate::render::canvas::Img;
use openh264::encoder::{
    BitRate, Complexity, EncodedBitStream, Encoder, EncoderConfig, FrameRate, IntraFramePeriod,
    QpRange, RateControlMode, UsageType,
};
use openh264::formats::{RgbaSliceU8, YUVBuffer};

use super::mux::extract_nals_from_annexb;
use super::{EncodedFrame, FrameEncoder};
use crate::config::video::video_cpu::*;

pub(crate) struct CpuEncoder {
    encoder: Encoder,
    /// Reused across calls to avoid reallocating the assembled Annex-B buffer.
    annexb_buf: Vec<u8>,
}

impl CpuEncoder {
    pub(crate) fn new(_w: u32, _h: u32, fps: u32) -> Result<Self> {
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .complexity(Complexity::Low)
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(CPU_VIDEO_BITRATE))
            .max_frame_rate(FrameRate::from_hz(fps as f32))
            .intra_frame_period(IntraFramePeriod::from_num_frames(fps.saturating_mul(2)))
            .qp(QpRange::new(18, 42))
            // A skipped frame produces no H.264 slice.  The MP4 muxer still
            // has to account for the input frame's duration, and writing an
            // empty sample creates a timestamp hole that strict players such
            // as QQ Windows may stop on. Keep every input frame in the stream;
            // rate control can still adjust QP toward the target bitrate.
            .skip_frames(false)
            .scene_change_detect(true)
            .adaptive_quantization(false)
            .background_detection(false)
            // 0 lets OpenH264 choose the encoder thread count.
            .num_threads(0);
        let api = openh264::OpenH264API::from_source();
        let encoder = Encoder::with_api_config(api, config)
            .map_err(|e| PreviewError::render(format!("failed to init openh264 encoder: {e}")))?;
        Ok(Self {
            encoder,
            annexb_buf: Vec::new(),
        })
    }
}

impl FrameEncoder for CpuEncoder {
    fn encode(&mut self, rgba: &Img) -> Result<EncodedFrame> {
        let yuv = rgba_to_yuv(rgba);
        let bs = self
            .encoder
            .encode(&yuv)
            .map_err(|e| PreviewError::render(format!("openh264 encode failed: {e}")))?;

        // openh264 returns NALs via the bitstream's layer/nal API (no start
        // codes between them). Reassemble into a contiguous Annex-B buffer so
        // the shared `extract_nals_from_annexb` path handles all backends
        // uniformly.
        self.annexb_buf.clear();
        collect_annexb(&bs, &mut self.annexb_buf);

        let (sps, pps, slice, is_keyframe) = extract_nals_from_annexb(&self.annexb_buf);
        Ok(EncodedFrame {
            sps,
            pps,
            slice,
            is_keyframe,
        })
    }

    fn name(&self) -> &'static str {
        "openh264"
    }
}

/// Convert an RGBA8 image to a YUV420 buffer for openh264.
fn rgba_to_yuv(img: &Img) -> YUVBuffer {
    let rgba = RgbaSliceU8::new(&img.data, (img.w as usize, img.h as usize));
    YUVBuffer::from_rgb_source(rgba)
}

/// Walk openh264's `EncodedBitStream` layers/NALs and concatenate them into a
/// single Annex-B bytestream (each NAL prefixed with `00 00 00 01`). openh264
/// returns NALs *with* start codes already prepended per NAL, so we just copy.
fn collect_annexb(bs: &EncodedBitStream, out: &mut Vec<u8>) {
    for l in 0..bs.num_layers() {
        let Some(layer) = bs.layer(l) else {
            continue;
        };
        for n in 0..layer.nal_count() {
            let Some(nal) = layer.nal_unit(n) else {
                continue;
            };
            // openh264 prepends a start code to each NAL already.
            out.extend_from_slice(nal);
        }
    }
}
