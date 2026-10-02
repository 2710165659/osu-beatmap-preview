//! 谱面背景视频：mp4 解封装 + H.264 解码，按关键帧**分段并行**取帧。
//!
//! 行为对齐 osu!（`DrawableStoryboardVideo`）：
//! - 视频时间 = 谱面时间 − `Video` 事件偏移（[`BackgroundVideo::start_ms`]）；
//! - 取「显示时间不晚于目标」的最新帧；
//! - 静态背景图在最底、视频叠在其上，两者用同一 `BACKGROUND_DIM` 暗化；
//! - 开始处 [`FADE_MS`] 淡入、结束前 [`FADE_MS`] 淡出，窗口外回退背景图。
//!
//! ## 性能：为什么分段并行
//!
//! 视频解码是导出耗时的绝对大头（实测 143s 的 800×450 视频顺序解码约 60s，
//! 占整次导出的 ~88%）。H.264 的参考链让**段内**帧必须按顺序解，但每个
//! 关键帧（IDR）都会重置参考缓冲——两个关键帧之间的片段互相独立。因此按
//! stss 切段后各段可以并行解码：实测同一条视频 43 段、8 路并行，解码时间
//! 缩到约 1/6。
//!
//! 内存同样按段收敛：只有**输出时间轴会用到的画面**（15fps 输出从 24fps
//! 视频里取样，约六成）会被转换、缩放暗化并保留，其余画面解完即弃；一批
//! 只并行 `batch_size` 段，峰值 ≈ 批次 × 段内选中帧 × 输出画布字节数。
//!
//! 解码栈是 openh264（与导出编码侧共用依赖）。注意它**不能逐次 flush**：
//! 带 B 帧的流上提前冲刷重排缓冲会直接报错断流（实测第 10 个样本即失败，
//! 表现为视频只动几帧后全程定格），只能在每段结尾 `flush_remaining` 收尾。

use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::Arc;

use openh264::decoder::{DecodeOptions, DecodedYUV, Flush};
use openh264::formats::YUVSource;
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
use rayon::prelude::*;

use crate::export::canvas::Img;

use super::video::{prepare_video_background, VideoStyle};

/// osu! 的淡入淡出时长（`DrawableStoryboardVideo` 的 `FadeIn(500)` / `FadeOut(500)`）。
pub const FADE_MS: i64 = 500;

/// 并行解码的内存预算：一批里所有段的选中画面之和不超过它。
const MEMORY_BUDGET_BYTES: usize = 768 * 1024 * 1024;

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

/// 并行解码的共享上下文（只读，跨线程安全）。
struct DecodeContext {
    /// 完整 mp4 字节：每个解码线程按借用切片自建 reader，不复制文件。
    bytes: Arc<Vec<u8>>,
    track_id: u32,
    /// Annex-B 的 SPS+PPS 头包；每段开头随首个 access unit 喂入。
    header_packet: Vec<u8>,
    /// 每个样本（= 画面）的显示时间（毫秒，视频自身时间轴）。
    times_ms: Vec<i64>,
    /// 对应样本是否被输出时间轴选中（只有选中的画面才转换与缩放暗化）。
    chosen: Vec<bool>,
    width: u32,
    height: u32,
    style: VideoStyle,
}

impl DecodeContext {
    /// 解一段样本（左闭右开，段间独立），返回本段**被选中**的画面。
    ///
    /// 段内解码顺序 = 样本顺序，解码器按显示序吐画面，因此第 k 个画面对应
    /// 段内显示序第 k 个样本；未选中的画面只解码、不转换不缩放。
    fn decode_segment(&self, start: usize, end: usize) -> Vec<(i64, Arc<Img>)> {
        let mut out = Vec::new();
        let mut order: Vec<usize> = (start..end).collect();
        order.sort_by_key(|&index| self.times_ms[index]);
        let Ok(mut reader) = mp4::Mp4Reader::read_header(
            Cursor::new(self.bytes.as_slice()),
            self.bytes.len() as u64,
        ) else {
            return out;
        };
        let Ok(mut decoder) = openh264::decoder::Decoder::new() else {
            return out;
        };
        let mut header = self.header_packet.clone();
        let mut emitted = 0_usize;
        // 不要逐次 flush（见模块文档）：只在段尾冲刷重排缓冲。
        let options = DecodeOptions::new().flush_after_decode(Flush::NoFlush);
        let mut consume = |yuv: DecodedYUV<'_>, out: &mut Vec<(i64, Arc<Img>)>| {
            let Some(&sample_index) = order.get(emitted) else {
                return;
            };
            emitted += 1;
            if !self.chosen[sample_index] {
                return;
            }
            let raw = decoded_picture(&yuv);
            let prepared = prepare_video_background(&raw, self.width, self.height, self.style);
            out.push((self.times_ms[sample_index], Arc::new(prepared)));
        };
        for sample_index in start..end {
            let Ok(Some(sample)) = reader.read_sample(self.track_id, sample_index as u32 + 1)
            else {
                return out;
            };
            let Some(packet) = sample_to_annexb(&sample.bytes, &mut header) else {
                return out;
            };
            match decoder.decode_with_options(&packet, options.clone()) {
                Ok(Some(yuv)) => consume(yuv, &mut out),
                Ok(None) => {}
                Err(_) => return out,
            }
        }
        if let Ok(frames) = decoder.flush_remaining() {
            for yuv in frames {
                consume(yuv, &mut out);
            }
        }
        out
    }
}

/// `.osz` 里的背景视频：mp4 解封装 + 分段并行解码 + 时间轴取帧。
pub struct BackgroundVideo {
    /// 解码段（样本下标，左闭右开）：每个关键帧起一段。
    segments: Vec<(usize, usize)>,
    context: Arc<DecodeContext>,
    /// 下一批要解的段。
    next_segment: usize,
    /// 一批并行解几段（由 `begin_decode` 按核数与内存预算算出）。
    batch_size: usize,
    /// 已解码、按时间序排好的目标画面（段内与段间天然有序）。
    queue: VecDeque<(i64, Arc<Img>)>,
    /// 最近一个「显示时间 ≤ 目标」的画面。
    current: Option<(i64, Arc<Img>)>,
    /// `Video` 事件偏移：视频第 0 帧对应的谱面时间（毫秒）。
    pub start_ms: i64,
    /// 视频总时长（毫秒，容器声明值），决定淡出窗口。
    pub duration_ms: i64,
}

impl BackgroundVideo {
    /// 打开背景视频：解析容器、显示时间表与解码段。容器/轨道/编码不受支持
    /// 时返回错误，调用方回退静态背景。
    pub fn open(bytes: Vec<u8>, start_ms: i64) -> Result<Self> {
        let size = bytes.len() as u64;
        let reader = mp4::Mp4Reader::read_header(Cursor::new(bytes.clone()), size)
            .map_err(|e| PreviewError::render(format!("failed to parse background video: {e}")))?;
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
        })? as usize;
        if sample_count == 0 {
            return Err(PreviewError::render("background video has no samples"));
        }
        let sps = track
            .sequence_parameter_set()
            .map_err(|e| PreviewError::render(format!("background video is missing SPS: {e}")))?;
        let pps = track
            .picture_parameter_set()
            .map_err(|e| PreviewError::render(format!("background video is missing PPS: {e}")))?;
        let mut header_packet = Vec::with_capacity(sps.len() + pps.len() + 8);
        for nal in [sps, pps] {
            header_packet.extend_from_slice(&[0, 0, 0, 1]);
            header_packet.extend_from_slice(nal);
        }

        // 显示时间 = stts 的解码时间 + ctts 的合成偏移，从轨道时基换算成毫秒。
        let stbl = &track.trak.mdia.minf.stbl;
        let mut times_ms = Vec::with_capacity(sample_count);
        let mut elapsed = 0_u64;
        for entry in &stbl.stts.entries {
            for _ in 0..entry.sample_count {
                times_ms.push((elapsed * 1000 / timescale as u64) as i64);
                elapsed += entry.sample_delta as u64;
            }
        }
        times_ms.resize(sample_count, 0);
        if let Some(ctts) = &stbl.ctts {
            let mut index = 0;
            for entry in &ctts.entries {
                for _ in 0..entry.sample_count {
                    if let Some(time) = times_ms.get_mut(index) {
                        *time += entry.sample_offset as i64 * 1000 / timescale as i64;
                    }
                    index += 1;
                }
            }
        }

        // 解码段：每个关键帧起一段（IDR 重置参考缓冲，段间解码独立）。
        // stss 缺失视为全帧内编码，整体一段。
        let mut segments: Vec<(usize, usize)> = Vec::new();
        let mut starts: Vec<usize> = match &stbl.stss {
            Some(stss) => stss
                .entries
                .iter()
                .filter_map(|number| number.checked_sub(1).map(|index| index as usize))
                .filter(|index| *index < sample_count)
                .collect(),
            None => vec![0],
        };
        if starts.first() != Some(&0) {
            starts.insert(0, 0);
        }
        for (position, &segment_start) in starts.iter().enumerate() {
            let segment_end = starts
                .get(position + 1)
                .copied()
                .unwrap_or(sample_count)
                .max(segment_start + 1);
            segments.push((segment_start, segment_end.min(sample_count)));
        }

        let duration_ms = track.duration().as_millis().max(1) as i64;
        Ok(Self {
            segments,
            context: Arc::new(DecodeContext {
                bytes: Arc::new(bytes),
                track_id,
                header_packet,
                times_ms,
                chosen: Vec::new(),
                width: 0,
                height: 0,
                style: VideoStyle::default(),
            }),
            next_segment: 0,
            // 真正的并行度由 begin_decode 按核数与内存预算算出。
            batch_size: 1,
            queue: VecDeque::new(),
            current: None,
            start_ms,
            duration_ms,
        })
    }

    /// 开始按需批量解码：`chart_times_ms` 是导出时间轴上的谱面时间（毫秒）。
    ///
    /// 只有会被这些时间点选中的画面才转换 RGBA 并缩放暗化（15fps 输出从
    /// 24fps 视频取样时约六成），其余画面解码后即弃。目标时间必须与
    /// [`Self::frame_at_or_before`] 的查询时间一致——导出按固定帧率推进，
    /// 天然满足。
    pub fn begin_decode(
        &mut self,
        width: u32,
        height: u32,
        style: VideoStyle,
        chart_times_ms: &[i64],
    ) {
        let context = Arc::get_mut(&mut self.context).expect("解码开始前上下文独占");
        // 选中集：画面 i 被选中 iff 它是某个目标时间之前最新的画面。
        let mut order: Vec<(i64, usize)> = context
            .times_ms
            .iter()
            .copied()
            .zip(0..context.times_ms.len())
            .collect();
        order.sort_unstable();
        let mut chosen = vec![false; context.times_ms.len()];
        for &chart_ms in chart_times_ms {
            let video_ms = chart_ms - self.start_ms;
            let position = order.partition_point(|(time, _)| *time <= video_ms);
            if position > 0 {
                chosen[order[position - 1].1] = true;
            }
        }
        // 批次并行度 = 核数与内存预算的较小者：一批里每段的选中画面都要
        // 常驻到被消费为止，1080p 输出时一段就可能上百 MB，必须按字节封顶。
        let chosen_count = chosen.iter().filter(|keep| **keep).count();
        let per_segment_bytes = (chosen_count / self.segments.len().max(1)).max(1)
            * width as usize
            * height as usize
            * 4;
        let parallel = std::thread::available_parallelism()
            .map(|threads| threads.get())
            .unwrap_or(4)
            .min(16);
        self.batch_size = (MEMORY_BUDGET_BYTES / per_segment_bytes.max(1)).clamp(1, parallel);
        context.chosen = chosen;
        context.width = width;
        context.height = height;
        context.style = style;
    }

    /// 视频在谱面时间 `chart_ms` 的可见度（osu! 的 500ms 淡入淡出）。
    pub fn visibility_alpha(&self, chart_ms: i64) -> f64 {
        visibility_alpha(chart_ms - self.start_ms, self.duration_ms)
    }

    /// 取谱面时间不晚于 `chart_ms` 的最新**选中**画面（已缩放暗化）。
    ///
    /// 队列按时间序供给（一批解一段组），目标时间回退时沿用当前帧
    ///（导出时间轴单调递增，正常不会发生；这里只防抖，不做昂贵的重解码）。
    pub fn frame_at_or_before(&mut self, chart_ms: i64) -> Option<Arc<Img>> {
        let video_ms = chart_ms - self.start_ms;
        loop {
            match self.queue.front() {
                Some((time, _)) if *time <= video_ms => {
                    let picture = self.queue.pop_front().expect("队首存在");
                    self.current = Some(picture);
                }
                Some(_) => break,
                None => {
                    if !self.decode_next_batch() {
                        break;
                    }
                }
            }
        }
        self.current
            .as_ref()
            .filter(|(time, _)| *time <= video_ms)
            .map(|(_, frame)| Arc::clone(frame))
    }

    /// 并行解下一批段并按时间序入队；没有剩余段时返回 `false`。
    fn decode_next_batch(&mut self) -> bool {
        if self.next_segment >= self.segments.len() {
            return false;
        }
        let end = (self.next_segment + self.batch_size).min(self.segments.len());
        let batch = self.segments[self.next_segment..end]
            .par_iter()
            .map(|&(start, end)| self.context.decode_segment(start, end))
            .collect::<Vec<_>>();
        self.next_segment = end;
        for segment in batch {
            self.queue.extend(segment);
        }
        true
    }
}

/// 视频自身时间轴 `video_ms` 处的可见度；开始处 [`FADE_MS`] 淡入、结束前
/// [`FADE_MS`] 淡出，窗口外为 0（短视频短于两段淡入淡出时取两者更低值）。
pub(crate) fn visibility_alpha(video_ms: i64, duration_ms: i64) -> f64 {
    if video_ms < 0 || video_ms >= duration_ms.max(1) {
        return 0.0;
    }
    let fade_in = video_ms as f64 / FADE_MS as f64;
    let fade_out = (duration_ms - video_ms) as f64 / FADE_MS as f64;
    fade_in.min(fade_out).clamp(0.0, 1.0)
}

/// 把 MP4 sample（4 字节长度前缀的 NAL 组）转成 openh264 需要的 Annex-B。
///
/// `header` 是待消耗的头包（SPS/PPS，Annex-B），只在每段首个 access unit 前
/// 非空；sample 结构损坏时返回 `None`，由调用方停止本段解码。
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
/// 静态背景图先按输出画布缩放暗化一次；视频画面由 [`BackgroundVideo`]
/// 分段并行解码，淡入淡出窗口内把视频帧与底层背景线性混合。
pub(crate) struct FrameBackgrounds {
    static_background: Option<Arc<Img>>,
    video: Option<BackgroundVideo>,
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
        chart_times_ms: &[i64],
    ) -> Self {
        let MediaBackground { image, mut video } = background;
        if let Some(video) = video.as_mut() {
            video.begin_decode(width, height, style, chart_times_ms);
        }
        Self {
            static_background: image
                .map(|image| Arc::new(prepare_video_background(&image, width, height, style))),
            video,
            width,
            height,
            style,
        }
    }

    /// 谱面时间 `chart_ms` 处的最终背景；都没有时返回 `None`（纯色画布）。
    pub(crate) fn background_at(&mut self, chart_ms: i64) -> Option<Arc<Img>> {
        let Some(video) = self.video.as_mut() else {
            return self.static_background.clone();
        };
        let alpha = video.visibility_alpha(chart_ms);
        if alpha <= 0.0 {
            return self.static_background.clone();
        }
        let Some(frame) = video.frame_at_or_before(chart_ms) else {
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
    use crate::media::video::VideoStyle;
    use bytes::Bytes;

    /// 用 CPU 编码器 + mp4 容器造一段纯色小视频；返回文件字节与各画面的显示时间。
    fn solid_video_bytes(frame_count: usize, fps: u32) -> (Vec<u8>, Vec<i64>) {
        use super::super::cpu::CpuEncoder;
        use super::super::video::{EncodedFrame, FrameEncoder};

        let (width, height) = (16_u32, 16_u32);
        let timescale = 1000_u32;
        let duration = timescale / fps;
        let mut encoder = CpuEncoder::new(width, height, fps).expect("测试编码器必须可用");
        let mut frames = Vec::new();
        for index in 0..frame_count {
            let shade = (40 + index * 60) as u8;
            let img = Img::new(width, height, [shade, shade, shade, 255]);
            frames.push(encoder.encode(&img).expect("测试帧必须可编码"));
        }
        let EncodedFrame { sps, pps, .. } = frames.first().expect("至少一帧");
        let config = mp4::Mp4Config {
            major_brand: mp4::FourCC::from(*b"isom"),
            minor_version: 512,
            compatible_brands: vec![mp4::FourCC::from(*b"isom")],
            timescale,
        };
        let mut writer = mp4::Mp4Writer::write_start(std::io::Cursor::new(Vec::new()), &config)
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
                        is_sync: index == 0,
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
            enable_storyboard: false,
            background_dim: 0.7,
            label_color: [255, 255, 255, 255],
            label_font_size: 18,
            label_pad: 16,
            black_opaque: [0, 0, 0, 255],
            fallback_background: [0, 0, 0, 255],
        }
    }

    /// 按画面灰度反查它在时间表里的位置。画面已按 `BACKGROUND_DIM` 暗化
    ///（夹具灰度 40/100/160/220 → 12/30/48/66），YUV 往返再有几级误差，
    /// 按最近的档位归位。
    fn frame_index(frame: &Img) -> usize {
        let shade = frame.data[0] as f64;
        (((shade - 12.0) / 18.0).round().max(0.0)) as usize
    }

    /// 淡入淡出可见度：窗口外为 0，窗口内线性，短视频取淡入/淡出更低值。
    #[test]
    fn visibility_alpha_matches_osu_fade_windows() {
        assert_eq!(visibility_alpha(-1, 10_000), 0.0);
        assert_eq!(visibility_alpha(0, 10_000), 0.0);
        assert_eq!(visibility_alpha(250, 10_000), 0.5);
        assert_eq!(visibility_alpha(500, 10_000), 1.0);
        assert_eq!(visibility_alpha(5_000, 10_000), 1.0);
        assert_eq!(visibility_alpha(9_750, 10_000), 0.5);
        assert_eq!(visibility_alpha(10_000, 10_000), 0.0);
        assert_eq!(visibility_alpha(250, 400), 0.3);
        assert_eq!(visibility_alpha(200, 400), 0.4);
    }

    /// sample（长度前缀 NAL）与 Annex-B 的互转，损坏结构返回 None。
    #[test]
    fn sample_and_annexb_round_trip() {
        let annexb = [0, 0, 0, 1, 0x65, 0xAA, 0, 0, 0, 1, 0x41, 0xBB];
        let sample = length_prefixed(&annexb);
        let mut header = vec![0, 0, 0, 1, 0x67, 0x01];
        let restored = sample_to_annexb(&sample, &mut header).expect("sample 必须可还原");
        assert_eq!(&restored[..6], &[0, 0, 0, 1, 0x67, 0x01]);
        assert!(restored.windows(4).any(|window| window == [0, 0, 0, 1]));
        assert!(header.is_empty());
        assert!(sample_to_annexb(&[0, 0, 0, 9, 0x41], &mut Vec::new()).is_none());
    }

    /// Annex-B 转长度前缀格式（构造 MP4 sample 用）。
    fn length_prefixed(annexb: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in crate::media::mux::split_nals(annexb) {
            out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            out.extend_from_slice(nal);
        }
        out
    }

    /// 时间轴取帧：目标之间取不晚于它的最新画面，视频未开始没有画面。
    #[test]
    fn frame_at_or_before_follows_the_timeline() {
        let (bytes, times) = solid_video_bytes(4, 4);
        let mut video = BackgroundVideo::open(bytes, 0).expect("测试视频必须可打开");
        video.begin_decode(16, 16, test_style(), &[0, 100, 300, 5_000]);
        assert_eq!(video.duration_ms, 1000);

        assert_eq!(frame_index(&video.frame_at_or_before(0).unwrap()), 0);
        assert_eq!(frame_index(&video.frame_at_or_before(100).unwrap()), 0);
        assert_eq!(frame_index(&video.frame_at_or_before(300).unwrap()), 1);
        // 超过最后一帧：保持最后一帧（osu! 的 clamp 行为）。
        assert_eq!(frame_index(&video.frame_at_or_before(5_000).unwrap()), 3);
        let _ = times;
    }

    /// 时间轴取帧：窗口外回退背景图，淡入淡出窗口线性混合。
    #[test]
    fn frame_backgrounds_follow_fade_windows() {
        let (bytes, _times) = solid_video_bytes(4, 4);
        let video = BackgroundVideo::open(bytes, 1000).expect("测试视频必须可打开");
        let mut backgrounds = FrameBackgrounds::new(
            MediaBackground {
                image: Some(Img::new(16, 16, [200, 200, 200, 255])),
                video: Some(video),
            },
            16,
            16,
            test_style(),
            &[500, 1_250, 1_600, 3_000],
        );

        // 暗化 70%：背景图 200 → 60；视频四帧灰度 40/100/160/220 → 12/30/48/66。
        let before = backgrounds.background_at(500).expect("必须有背景");
        assert_eq!(before.get(0, 0), [60, 60, 60, 255]);
        // 淡入窗口中点（视频时间 250，第 2 帧）：与背景各占一半，(60+30)/2 = 45。
        let fading = backgrounds.background_at(1_250).expect("必须有背景");
        let pixel = fading.get(0, 0);
        assert!(
            (pixel[0] as i64 - 45).abs() <= 12,
            "淡入窗口中点应是两者混合：{pixel:?}"
        );
        // 完全可见：第 3 帧（视频时间 500，灰度 160 → 48）。
        let visible = backgrounds.background_at(1_600).expect("必须有背景");
        let pixel = visible.get(0, 0);
        assert!(
            (pixel[0] as i64 - 48).abs() <= 8,
            "应显示第 3 帧：{pixel:?}"
        );
        // 视频结束后回退背景图。
        let after = backgrounds.background_at(3_000).expect("必须有背景");
        assert_eq!(after.get(0, 0), [60, 60, 60, 255]);
    }
}
