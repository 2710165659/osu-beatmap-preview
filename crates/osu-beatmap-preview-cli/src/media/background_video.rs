//! 谱面背景视频：`.osz` 里的 mp4 逐帧合成到 MP4 导出背景。
//!
//! 行为对齐 osu!（`DrawableStoryboardVideo`）：
//! - 视频时间 = 谱面时间 − `Video` 事件偏移（[`BackgroundVideo::start_ms`]）；
//! - 取「显示时间不晚于目标」的最新帧，顺序解码只前进不回退；
//! - 静态背景图在最底、视频叠在其上，两者用同一 `BACKGROUND_DIM` 暗化；
//! - 视频开始处 [`FADE_MS`] 淡入、结束前 [`FADE_MS`] 淡出，窗口外回退背景图
//!  （osu! 在视频结束后露出背景图而不是定格最后一帧）。
//!
//! 解码栈是 `mp4` crate 解封装（仅 avc1/H.264）+ openh264 软解，与导出编码侧
//! 共用现有依赖，不引入 ffmpeg 子进程。容器/编码不受支持或解码失败时本模块
//! 报错，由调用方静默回退静态背景（与 osu! 的降级行为一致）。

use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::Arc;

use openh264::decoder::{DecodeOptions, DecodedYUV, Flush};
use openh264::formats::YUVSource;
use osu_beatmap_preview_core::support::error::{PreviewError, Result};

use crate::export::canvas::Img;

use super::video::{prepare_video_background, VideoStyle};

/// osu! 的淡入淡出时长（`DrawableStoryboardVideo` 的 `FadeIn(500)` / `FadeOut(500)`）。
pub(crate) const FADE_MS: i64 = 500;

/// MP4 导出的背景素材：静态背景图与背景视频（视频叠在图上）。
pub(crate) struct MediaBackground {
    pub(crate) image: Option<Img>,
    pub(crate) video: Option<BackgroundVideo>,
}

impl MediaBackground {
    /// 两者都没有时玩法层需要自己填内容框底色，不能保持透明。
    pub(crate) fn is_empty(&self) -> bool {
        self.image.is_none() && self.video.is_none()
    }
}

/// `.osz` 里的背景视频：mp4 解封装 + H.264 顺序解码。
pub(crate) struct BackgroundVideo {
    reader: mp4::Mp4Reader<Cursor<Vec<u8>>>,
    track_id: u32,
    timescale: u32,
    sample_count: u32,
    /// 下一个待读 sample（1-based，与 `mp4` crate 一致）。
    next_sample: u32,
    decoder: openh264::decoder::Decoder,
    /// Annex-B 的 SPS+PPS 头包，只在首个 sample 前喂给解码器一次。
    header_packet: Vec<u8>,
    /// 已交给解码器、尚未输出画面的显示时间（毫秒，升序）。
    ///
    /// H.264 解码器按显示序输出画面，因此第 k 个输出画面对应这里第 k 小的
    /// 显示时间：mp4 的 stts/ctts 给的是解码序时间戳，两者靠这张队列对齐。
    pending_times_ms: Vec<i64>,
    /// 流尾冲刷出来的画面（解码器重排缓冲里压着的尾巴）。
    tail: VecDeque<(i64, Arc<Img>)>,
    flushed: bool,
    /// `Video` 事件偏移：视频第 0 帧对应的谱面时间（毫秒）。
    pub(crate) start_ms: i64,
    /// 视频总时长（毫秒，容器声明值），决定淡出窗口。
    pub(crate) duration_ms: i64,
}

impl BackgroundVideo {
    /// 打开背景视频。容器/轨道/编码不受支持时返回错误，调用方回退静态背景。
    pub(crate) fn open(bytes: Vec<u8>, start_ms: i64) -> Result<Self> {
        let size = bytes.len() as u64;
        let reader = mp4::Mp4Reader::read_header(Cursor::new(bytes), size)
            .map_err(|e| PreviewError::render(format!("failed to parse background video: {e}")))?;
        // openh264 只能解 H.264；HEVC/VP9 等容器直接拒绝，让调用方降级。
        let Some((&track_id, track)) = reader.tracks().iter().find(|(_, track)| {
            matches!(track.track_type(), Ok(mp4::TrackType::Video))
                && matches!(track.media_type(), Ok(mp4::MediaType::H264))
        }) else {
            return Err(PreviewError::render(
                "background video has no H.264 video track",
            ));
        };
        let timescale = track.timescale();
        if timescale == 0 {
            return Err(PreviewError::render(
                "background video track has an invalid timescale",
            ));
        }
        let sample_count = reader.sample_count(track_id).map_err(|e| {
            PreviewError::render(format!("failed to read background video samples: {e}"))
        })?;
        if sample_count == 0 {
            return Err(PreviewError::render("background video has no samples"));
        }
        let sps = track.sequence_parameter_set().map_err(|e| {
            PreviewError::render(format!("background video is missing SPS: {e}"))
        })?;
        let pps = track.picture_parameter_set().map_err(|e| {
            PreviewError::render(format!("background video is missing PPS: {e}"))
        })?;
        // 头包用 Annex-B 拼好，首个 sample 解码前一次性喂给解码器。
        let mut header_packet = Vec::with_capacity(sps.len() + pps.len() + 8);
        for nal in [sps, pps] {
            header_packet.extend_from_slice(&[0, 0, 0, 1]);
            header_packet.extend_from_slice(nal);
        }
        let duration_ms = track.duration().as_millis().max(1) as i64;
        Ok(Self {
            reader,
            track_id,
            timescale,
            sample_count,
            next_sample: 1,
            decoder: openh264::decoder::Decoder::new()
                .map_err(|e| PreviewError::render(format!("failed to init H.264 decoder: {e}")))?,
            header_packet,
            pending_times_ms: Vec::new(),
            tail: VecDeque::new(),
            flushed: false,
            start_ms,
            duration_ms,
        })
    }

    /// 取下一个显示画面（原始 RGBA 与视频自身时间轴的显示时间，毫秒）；流尾返回 `None`。
    ///
    /// 解码只前进不回退：调用方需要保证目标时间大体单调（MP4 导出的帧时间
    /// 天然递增），回退场景由 [`FrameBackgrounds`] 用已缓存的画面兜底。
    pub(crate) fn next_picture(&mut self) -> Option<(i64, Arc<Img>)> {
        loop {
            if let Some(picture) = self.tail.pop_front() {
                return Some(picture);
            }
            if self.next_sample > self.sample_count {
                if self.flushed {
                    return None;
                }
                self.flushed = true;
                self.flush_decoder_tail();
                continue;
            }
            let sample_id = self.next_sample;
            self.next_sample += 1;
            let sample = match self.reader.read_sample(self.track_id, sample_id) {
                Ok(Some(sample)) => sample,
                // 读不到 sample 就当流尾处理：已解出的画面照常返回。
                Ok(None) => return None,
                Err(_) => return None,
            };
            // 显示时间 = stts 的解码时间 + ctts 的合成偏移，再从轨道时基换算成毫秒。
            let time_ms = (sample.start_time as i64 + sample.rendering_offset as i64) * 1000
                / self.timescale as i64;
            let packet = sample_to_annexb(&sample.bytes, &mut self.header_packet);
            let Some(packet) = packet else {
                // sample 结构损坏：剩余画面无法可靠对齐时间戳，直接收尾。
                return None;
            };
            self.pending_times_ms.push(time_ms);
            self.pending_times_ms.sort_unstable();
            // 不要逐次 flush：openh264 的重排缓冲在带 B 帧的流上被提前冲刷会
            // 直接报错断流（实测某 24fps B 帧视频第 10 个样本即失败，表现为
            // 视频只动几帧后全程定格）；只在流尾用 flush_remaining 收尾。
            // 不冲刷时解码器按**显示序**输出画面，与上面按显示时间弹出的映射一致。
            let decoded = match self.decoder.decode_with_options(
                &packet,
                DecodeOptions::new().flush_after_decode(Flush::NoFlush),
            ) {
                Ok(decoded) => decoded,
                Err(_) => return None,
            };
            if let Some(frame) = decoded {
                let time_ms = self.pending_times_ms.first().copied().unwrap_or(time_ms);
                if !self.pending_times_ms.is_empty() {
                    self.pending_times_ms.remove(0);
                }
                return Some((time_ms, Arc::new(decoded_picture(&frame))));
            }
        }
    }

    /// 把解码器重排缓冲里压着的最后几帧按显示序放进 `tail`。
    fn flush_decoder_tail(&mut self) {
        let mut times = self.pending_times_ms.iter().copied();
        let Ok(frames) = self.decoder.flush_remaining() else {
            return;
        };
        for frame in frames {
            let time_ms = times.next().unwrap_or(self.duration_ms);
            self.tail.push_back((time_ms, Arc::new(decoded_picture(&frame))));
        }
    }

    /// 视频在谱面时间 `chart_ms` 的可见度（osu! 的 500ms 淡入淡出）。
    ///
    /// 窗口外返回 0.0（回退静态背景图）；淡入/淡出窗口内线性变化。
    pub(crate) fn visibility_alpha(&self, chart_ms: i64) -> f64 {
        visibility_alpha(chart_ms - self.start_ms, self.duration_ms)
    }
}

/// 视频自身时间轴 `video_ms` 处的可见度；开始处 [`FADE_MS`] 淡入、结束前
/// [`FADE_MS`] 淡出，窗口外为 0（视频短于两段淡入淡出时按三角形取更低值）。
fn visibility_alpha(video_ms: i64, duration_ms: i64) -> f64 {
    if video_ms < 0 || video_ms >= duration_ms.max(1) {
        return 0.0;
    }
    let fade_in = video_ms as f64 / FADE_MS as f64;
    let fade_out = (duration_ms - video_ms) as f64 / FADE_MS as f64;
    fade_in.min(fade_out).clamp(0.0, 1.0)
}

/// 把 MP4 sample（4 字节长度前缀的 NAL 组）转成 openh264 需要的 Annex-B。
///
/// `header` 是待消耗的头包（SPS/PPS，Annex-B），只在首个 sample 非空；sample
/// 结构损坏时返回 `None`，由调用方停止解码。
fn sample_to_annexb(sample: &[u8], header: &mut Vec<u8>) -> Option<Vec<u8>> {
    let mut out = std::mem::take(header);
    let mut rest = sample;
    while !rest.is_empty() {
        let length = rest
            .get(..4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_be_bytes)
            .map(|value| value as usize)?;
        rest = &rest[4..];
        if length == 0 || length > rest.len() {
            return None;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&rest[..length]);
        rest = &rest[length..];
    }
    (!out.is_empty()).then_some(out)
}

/// 解码画面（YUV420）转 RGBA。
fn decoded_picture(frame: &DecodedYUV<'_>) -> Img {
    let (width, height) = frame.dimensions();
    let mut img = Img::new(width as u32, height as u32, [0, 0, 0, 255]);
    frame.write_rgba8(&mut img.data);
    img
}

/// 按输出帧时间轴逐帧给出最终背景。
///
/// 静态背景图先按输出画布缩放暗化一次；视频帧顺序解码、同样只准备一次，
/// 相邻输出帧共享同一张 [`Arc`]。淡入淡出窗口内把视频帧与底层背景线性混合。
pub(crate) struct FrameBackgrounds {
    static_background: Option<Arc<Img>>,
    video: Option<VideoFrames>,
    width: u32,
    height: u32,
    style: VideoStyle,
}

impl FrameBackgrounds {
    pub(crate) fn new(
        background: MediaBackground,
        width: u32,
        height: u32,
        style: VideoStyle,
    ) -> Self {
        let MediaBackground { image, video } = background;
        Self {
            static_background: image.map(|image| Arc::new(prepare_video_background(&image, width, height, style))),
            video: video.map(VideoFrames::new),
            width,
            height,
            style,
        }
    }

    /// 谱面时间 `chart_ms` 处的最终背景；都没有时返回 `None`（纯色画布）。
    pub(crate) fn background_at(&mut self, chart_ms: i64) -> Option<Arc<Img>> {
        let Some(frames) = self.video.as_mut() else {
            return self.static_background.clone();
        };
        let alpha = frames.video.visibility_alpha(chart_ms);
        if alpha <= 0.0 {
            return self.static_background.clone();
        }
        let video_ms = chart_ms - frames.video.start_ms;
        let Some(frame) = frames.frame_at_or_before(video_ms, self.width, self.height, self.style)
        else {
            return self.static_background.clone();
        };
        if alpha >= 1.0 {
            return Some(frame);
        }
        // 淡入淡出窗口：与底层背景图（没有则纯色）按 alpha 线性混合。
        let base = self.static_background.clone().unwrap_or_else(|| {
            Arc::new(Img::new(self.width, self.height, self.style.black_opaque))
        });
        Some(Arc::new(blend_background(&base, &frame, alpha)))
    }
}

/// 视频画面的顺序解码 + 已准备帧缓存。
struct VideoFrames {
    video: BackgroundVideo,
    /// 已消费的最新画面（时间 ≤ 目标，原始未缩放）。
    current: Option<(i64, Arc<Img>)>,
    /// `current` 的缩放暗化结果；时间未变时直接复用。被超越的中间帧不做
    /// 缩放暗化——一轮推进常连吃好几帧，逐帧准备纯属浪费。
    prepared: Option<(i64, Arc<Img>)>,
    /// 已解码但时间还在目标之后的画面。
    next: Option<(i64, Arc<Img>)>,
    finished: bool,
}

impl VideoFrames {
    fn new(video: BackgroundVideo) -> Self {
        Self {
            video,
            current: None,
            prepared: None,
            next: None,
            finished: false,
        }
    }

    /// 取显示时间不晚于 `video_ms` 的最新画面（已缩放暗化）。
    ///
    /// 目标时间回退到当前帧之前时直接沿用当前帧（导出时间轴单调递增，
    /// 正常不会发生；这里只防抖，不做昂贵的重解码）。
    fn frame_at_or_before(
        &mut self,
        video_ms: i64,
        width: u32,
        height: u32,
        style: VideoStyle,
    ) -> Option<Arc<Img>> {
        while self.next.as_ref().is_none_or(|(time, _)| *time <= video_ms) {
            if let Some(picture) = self.next.take() {
                self.current = Some(picture);
            }
            if self.finished {
                break;
            }
            match self.video.next_picture() {
                Some(picture) => self.next = Some(picture),
                None => self.finished = true,
            }
        }
        let (time, raw) = self.current.as_ref()?;
        if self
            .prepared
            .as_ref()
            .is_none_or(|(prepared_time, _)| prepared_time != time)
        {
            let prepared = prepare_video_background(raw, width, height, style);
            self.prepared = Some((*time, Arc::new(prepared)));
        }
        self.prepared.as_ref().map(|(_, frame)| Arc::clone(frame))
    }
}

/// 两张同尺寸不透明 RGBA 按 alpha 线性混合（`base * (1 - alpha) + overlay * alpha`）。
fn blend_background(base: &Img, overlay: &Img, alpha: f64) -> Img {
    let mut result = base.clone();
    let alpha = alpha.clamp(0.0, 1.0);
    for (dst, src) in result
        .data
        .chunks_exact_mut(4)
        .zip(overlay.data.chunks_exact(4))
    {
        for (dst_channel, src_channel) in dst.iter_mut().zip(src.iter()).take(3) {
            *dst_channel =
                (*dst_channel as f64 * (1.0 - alpha) + *src_channel as f64 * alpha).round() as u8;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    /// 用 CPU 编码器 + mp4 容器造一段纯色小视频；返回文件字节与各画面的显示时间。
    fn solid_video_bytes(frame_count: u32, fps: u32) -> (Vec<u8>, Vec<i64>) {
        use super::super::cpu::CpuEncoder;
        use super::super::video::{FrameEncoder, EncodedFrame};

        let (width, height) = (16u32, 16u32);
        let timescale = 1000_u32;
        let duration = timescale / fps;
        let mut encoder = CpuEncoder::new(width, height, fps).expect("测试编码器必须可用");
        let mut frames = Vec::new();
        for index in 0..frame_count {
            let shade = (40 + index * 60) as u8;
            let img = Img::new(width, height, [shade, shade, shade, 255]);
            frames.push(encoder.encode(&img).expect("测试帧必须可编码"));
        }
        let EncodedFrame {
            sps, pps, ..
        } = frames.first().expect("至少一帧");
        let config = mp4::Mp4Config {
            major_brand: mp4::FourCC::from(*b"isom"),
            minor_version: 512,
            compatible_brands: vec![mp4::FourCC::from(*b"isom")],
            timescale,
        };
        let mut writer =
            mp4::Mp4Writer::write_start(std::io::Cursor::new(Vec::new()), &config)
                .expect("mp4 写入必须可用");
        writer
            .add_track(&mp4::TrackConfig {
                track_type: mp4::TrackType::Video,
                timescale,
                language: "und".to_string(),
                media_conf: mp4::MediaConfig::AvcConfig(mp4::AvcConfig {
                    width: width as u16,
                    height: height as u16,
                    seq_param_set: sps.clone().expect("首帧必须带 SPS"),
                    pic_param_set: pps.clone().expect("首帧必须带 PPS"),
                }),
            })
            .expect("视频轨必须可添加");
        let mut times = Vec::new();
        for (index, frame) in frames.iter().enumerate() {
            let start_time = index as u64 * duration as u64;
            times.push(start_time as i64);
            writer
                .write_sample(
                    1,
                    &mp4::Mp4Sample {
                        start_time,
                        duration,
                        rendering_offset: 0,
                        is_sync: frame.is_keyframe,
                        bytes: Bytes::copy_from_slice(&frame.slice),
                    },
                )
                .expect("sample 必须可写");
        }
        writer.write_end().expect("mp4 收尾必须成功");
        (writer.into_writer().into_inner(), times)
    }

    /// 测试用的视频合成样式：与默认 `BACKGROUND_DIM` 一致（暗化 70%）。
    fn test_style() -> VideoStyle {
        VideoStyle {
            enable_background_image: true,
            enable_background_video: true,
            background_dim: 0.7,
            label_color: [255, 255, 255, 255],
            label_font_size: 18,
            label_pad: 16,
            black_opaque: [0, 0, 0, 255],
            fallback_background: [0, 0, 0, 255],
        }
    }

    /// 淡入淡出可见度：窗口外为 0，窗口内线性，短视频按三角形取低值。
    #[test]
    fn visibility_alpha_matches_osu_fade_windows() {
        assert_eq!(visibility_alpha(-1, 10_000), 0.0);
        assert_eq!(visibility_alpha(0, 10_000), 0.0);
        assert_eq!(visibility_alpha(250, 10_000), 0.5);
        assert_eq!(visibility_alpha(500, 10_000), 1.0);
        assert_eq!(visibility_alpha(5_000, 10_000), 1.0);
        assert_eq!(visibility_alpha(9_750, 10_000), 0.5);
        assert_eq!(visibility_alpha(10_000, 10_000), 0.0);
        // 时长不足 2×FADE_MS 时整体压在淡出窗口内：取淡入/淡出的更低值。
        assert_eq!(visibility_alpha(250, 400), 0.3);
        assert_eq!(visibility_alpha(200, 400), 0.4);
    }

    /// sample（长度前缀 NAL）与 Annex-B 的互转，损坏结构返回 None。
    #[test]
    fn sample_and_annexb_round_trip() {
        let annexb = [0, 0, 0, 1, 0x65, 0xAA, 0, 0, 0, 1, 0x41, 0xBB];
        let (_, _, sample, _) = super::super::mux::extract_nals_from_annexb(&annexb);
        let mut header = vec![0, 0, 0, 1, 0x67, 0x01];
        let restored = sample_to_annexb(&sample, &mut header).expect("sample 必须可还原");
        // 首包是头包，随后是两个 NAL 的 Annex-B。
        assert_eq!(&restored[..6], &[0, 0, 0, 1, 0x67, 0x01]);
        assert!(restored.windows(4).any(|window| window == [0, 0, 0, 1]));
        // 头包只消耗一次。
        assert!(header.is_empty());

        // 长度越界视为损坏。
        assert!(sample_to_annexb(&[0, 0, 0, 9, 0x41], &mut Vec::new()).is_none());
    }

    /// 真实解码：画面数、显示时间与流尾行为都正确。
    #[test]
    fn background_video_decodes_pictures_in_display_order() {
        let (bytes, times) = solid_video_bytes(4, 4);
        let mut video =
            BackgroundVideo::open(bytes, -250).expect("测试视频必须可打开");
        assert_eq!(video.start_ms, -250);
        assert_eq!(video.duration_ms, 1000);

        let mut decoded = Vec::new();
        while let Some((time, frame)) = video.next_picture() {
            assert_eq!((frame.w, frame.h), (16, 16));
            decoded.push(time);
        }
        assert_eq!(decoded, times);
        // 流尾重复取帧返回 None，不会 panic。
        assert!(video.next_picture().is_none());
    }

    /// 时间轴取帧：窗口外回退背景图，淡入淡出窗口线性混合。
    #[test]
    fn frame_backgrounds_follow_fade_windows() {
        let (bytes, _times) = solid_video_bytes(4, 4);
        let video = BackgroundVideo::open(bytes, 1000).expect("测试视频必须可打开");
        let style = test_style();
        let mut backgrounds = FrameBackgrounds::new(
            MediaBackground {
                image: Some(Img::new(16, 16, [200, 200, 200, 255])),
                video: Some(video),
            },
            16,
            16,
            style,
        );

        // 暗化 70%：背景图 200 → 60；视频四帧灰度 40/100/160/220 → 12/30/48/66。
        // 视频开始前是静态背景图。
        let before = backgrounds.background_at(500).expect("必须有背景");
        assert_eq!(before.get(0, 0), [60, 60, 60, 255]);
        // 淡入窗口中点（视频时间 250，第 2 帧）：与背景各占一半，(60+30)/2 = 45。
        let fading = backgrounds.background_at(1250).expect("必须有背景");
        let pixel = fading.get(0, 0);
        assert!(
            (pixel[0] as i64 - 45).abs() <= 12,
            "淡入窗口中点应是两者混合：{pixel:?}"
        );
        // 完全可见：第 3 帧（视频时间 500，灰度 160 → 48）。
        let visible = backgrounds.background_at(1600).expect("必须有背景");
        let pixel = visible.get(0, 0);
        assert!((pixel[0] as i64 - 48).abs() <= 8, "应显示第 3 帧：{pixel:?}");
        // 视频结束后回退背景图。
        let after = backgrounds.background_at(3000).expect("必须有背景");
        assert_eq!(after.get(0, 0), [60, 60, 60, 255]);
    }
}
