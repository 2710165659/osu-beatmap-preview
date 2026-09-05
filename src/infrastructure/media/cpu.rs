//! 使用 openh264（Cisco 开源编码器，通过 `source` 特性编译）的 CPU H.264 编码后端。
//! 当无法初始化 GPU 编码器（NVENC / AMF）时始终可用。
//!
//! 使用码率模式和受限 QP。渲染与编码分阶段执行，因此编码器可使用 openh264
//! 自动线程数，不会与 Rayon 渲染线程池争用资源。

use crate::domain::errors::{PreviewError, Result};
use crate::render::canvas::Img;
use openh264::encoder::{
    BitRate, Complexity, EncodedBitStream, Encoder, EncoderConfig, FrameRate, IntraFramePeriod,
    QpRange, RateControlMode, UsageType,
};
use openh264::formats::{RgbaSliceU8, YUVBuffer};

use super::mux::extract_nals_from_annexb;
use super::{EncodedFrame, FrameEncoder};

pub(crate) struct CpuEncoder {
    encoder: Encoder,
    /// 跨调用复用，避免重复分配拼接后的 Annex-B 缓冲区。
    annexb_buf: Vec<u8>,
}

impl CpuEncoder {
    pub(crate) fn new(_w: u32, _h: u32, fps: u32) -> Result<Self> {
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .complexity(Complexity::Low)
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(
                crate::infrastructure::config::current()
                    .advance
                    .video
                    .CPU_VIDEO_BITRATE,
            ))
            .max_frame_rate(FrameRate::from_hz(fps as f32))
            .intra_frame_period(IntraFramePeriod::from_num_frames(fps.saturating_mul(2)))
            .qp(QpRange::new(18, 42))
            // 跳过的帧不会产生 H.264 slice，但 MP4 封装仍需计入输入帧时长；
            // 写入空 sample 会产生时间戳空洞，严格播放器（如 QQ Windows）可能因此停止。
            // 保留所有输入帧，码率控制仍可将 QP 调整到目标码率。
            .skip_frames(false)
            .scene_change_detect(true)
            .adaptive_quantization(false)
            .background_detection(false)
            // 0 表示让 OpenH264 自动选择编码线程数。
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

        // openh264 通过 bitstream 的 layer/nal API 返回 NAL（彼此之间没有起始码）。
        // 将其重新拼接为连续 Annex-B 缓冲区，使共享的 `extract_nals_from_annexb`
        // 路径能一致处理所有后端。
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

/// 将 RGBA8 图像转换为 openh264 使用的 YUV420 缓冲区。
fn rgba_to_yuv(img: &Img) -> YUVBuffer {
    let rgba = RgbaSliceU8::new(&img.data, (img.w as usize, img.h as usize));
    YUVBuffer::from_rgb_source(rgba)
}

/// 遍历 openh264 的 `EncodedBitStream` 层与 NAL，将其拼接为单一 Annex-B 字节流
///（每个 NAL 前有 `00 00 00 01`）。openh264 返回的每个 NAL 已带起始码，直接复制即可。
fn collect_annexb(bs: &EncodedBitStream, out: &mut Vec<u8>) {
    for l in 0..bs.num_layers() {
        let Some(layer) = bs.layer(l) else {
            continue;
        };
        for n in 0..layer.nal_count() {
            let Some(nal) = layer.nal_unit(n) else {
                continue;
            };
            // openh264 已为每个 NAL 添加起始码。
            out.extend_from_slice(nal);
        }
    }
}
