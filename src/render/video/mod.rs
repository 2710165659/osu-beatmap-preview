//! MP4（H.264）视频编码器：将回调生成的帧通过 H.264 和 `mp4` crate
//! 流式写入 MP4 文件。流程类似 `save_animated_gif_streamed`：
//! rayon 分块并行渲染，再顺序编码以保持帧顺序。
//!
//! 每帧游戏区域会放入 16:9 黑色画布，在右上角绘制“当前 / 总时长”标签，
//! 转换为后端所需格式后编码为 H.264，并写入一个 MP4 sample。
//! 完整动画不会同时驻留内存，最多保留 `PAR_CHUNK_SIZE` 个原始帧。
//!
//! ## GPU 加速
//!
//! 编码按顺序分派给第一个可用的后端：
//!   1. **NVENC**（NVIDIA）：运行时动态加载 `nvEncodeAPI64.dll`。
//!   2. **AMF**（AMD）：运行时动态加载 `amfrt64.dll`。
//!   3. **openh264**（CPU）：始终可用的单线程软件编码回退（原始实现）。
//!
//! 所有后端都输出 Annex-B H.264 NAL，并交给共享封装层
//!（`extract_nals` + `mp4` writer），因此输出文件结构一致。GPU DLL 通过
//! `libloading` 加载；构建或运行时缺少 DLL 都不会影响程序，编码器会静默回退到 CPU。

use crate::common::time_selection::TimeAxis;
use crate::core::errors::{PreviewError, Result};
use crate::core::models::Beatmap;
use crate::core::timeout::RequestDeadline;
use crate::core::validate::TimePoint;
use crate::parser::round_half_even;
use crate::pipeline::cache::with_atomic_output_deadline;
use crate::render::canvas::Img;
use crate::render::text::{draw_text, text_size};
use crate::render::video::audio::{encode_audio_segment, full_video_start_time, AudioSourceJob};
use bytes::Bytes;
use rayon::prelude::*;
use std::io::BufWriter;
use std::path::Path;
use std::time::Instant;

pub(crate) mod audio;

#[allow(non_camel_case_types, dead_code)]
mod amf;
mod cpu;
mod mux;
#[cfg(windows)]
mod nvenc;

/// 并行渲染分块大小（与 GIF 一致：一次约 8 帧）。
/// 限制异常大游戏区域带来的临时 RGBA 帧内存。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VideoTimeRange {
    pub(crate) start: i64,
    pub(crate) end: i64,
}

pub(crate) fn resolve_video_time_range(
    beatmap: &Beatmap,
    first_object_ms: i64,
    last_object_ms: i64,
    start_time: Option<TimePoint>,
    duration_time: Option<f64>,
    speed: f64,
) -> Result<VideoTimeRange> {
    let full_start = full_video_start_time(first_object_ms, beatmap.audio_lead_in_ms());
    let full_end = last_object_ms
        .checked_add(crate::config::current().video.video.VIDEO_END_PADDING_MS)
        .ok_or_else(|| PreviewError::new("mp4 time range is outside the supported range"))?;
    let full_range = validate_video_time_range(VideoTimeRange {
        start: full_start,
        end: full_end,
    })?;

    let duration = duration_time.unwrap_or(600.0);
    if !duration.is_finite() || duration <= 0.0 {
        return Err(PreviewError::new(
            "duration time must be a positive finite number",
        ));
    }
    let span = chart_span_for_actual_duration(round_half_even(duration * 1000.0), speed)?;
    let full_duration = full_range
        .end
        .checked_sub(full_range.start)
        .ok_or_else(|| PreviewError::new("mp4 time range is outside the supported range"))?;
    if full_duration <= span {
        return Ok(full_range);
    }

    let requested_start = match start_time.unwrap_or(TimePoint::Seconds(0.0)) {
        TimePoint::Preview => {
            preview_time_or_first_object(beatmap, first_object_ms).max(full_range.start)
        }
        TimePoint::Seconds(seconds) => {
            if !seconds.is_finite() {
                return Err(PreviewError::new("time point must be finite"));
            }
            let offset_f64 = seconds * 1000.0;
            if !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&offset_f64) {
                return Err(PreviewError::new(
                    "time point is outside the supported range",
                ));
            }
            let offset = round_half_even(offset_f64);
            first_object_ms
                .checked_add(offset)
                .ok_or_else(|| PreviewError::new("start time is outside the supported range"))?
        }
    };

    // 请求区间超出谱面尾部时整体向前移动，在完整谱面足够长时保留请求时长。
    // 早于可播放范围的起点仍然保留，以便输出对应的前置静音。
    let latest_start = full_range
        .end
        .checked_sub(span)
        .ok_or_else(|| PreviewError::new("mp4 time range is outside the supported range"))?;
    let start = requested_start.min(latest_start);
    validate_video_time_range(VideoTimeRange {
        start,
        end: start
            .checked_add(span)
            .ok_or_else(|| PreviewError::new("video range is outside the supported range"))?,
    })
}

fn validate_video_time_range(range: VideoTimeRange) -> Result<VideoTimeRange> {
    let duration = range
        .end
        .checked_sub(range.start)
        .ok_or_else(|| PreviewError::new("mp4 time range is outside the supported range"))?;
    if duration <= 0 {
        return Err(PreviewError::new("mp4 time range is empty"));
    }
    Ok(range)
}

fn chart_span_for_actual_duration(actual_duration_ms: i64, speed: f64) -> Result<i64> {
    if !speed.is_finite() || speed <= 0.0 {
        return Err(PreviewError::render("invalid video speed multiplier"));
    }
    Ok(round_half_even(actual_duration_ms as f64 * speed).max(1))
}

fn preview_time_or_first_object(beatmap: &Beatmap, first_object_ms: i64) -> i64 {
    beatmap
        .general
        .get("PreviewTime")
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|&preview_time| preview_time >= 0)
        .unwrap_or(first_object_ms)
}

/// 已编码并可封装进 MP4 容器的 H.264 帧。
///
/// 仅携带 SPS/PPS 的帧对应字段为 `Some`。`slice` 是带长度前缀的 slice NAL 数据，
/// `is_keyframe` 表示是否实际包含 IDR NAL。
struct EncodedFrame {
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    slice: Vec<u8>,
    is_keyframe: bool,
}

/// 后端 H.264 编码器：接收合成后的 RGBA 帧并产生 Annex-B NAL。
/// 实现持有自身编码状态（GPU 会话、CPU codec 等），必须按顺序输入帧。
///
/// 安全约定：`encode` 仅由单线程（封装循环）顺序调用，因此后端无需实现 `Sync`。
/// trait 对象会跨 rayon 并行渲染分块持有，所以 `into_par_iter` 期间不得借用它。
/// `save_mp4_streamed` 在并行收集完成后才编码，因此符合约定。
trait FrameEncoder {
    /// 编码一帧合成 RGBA，返回拆分为 SPS / PPS / slice 的 NAL 供封装。
    fn encode(&mut self, rgba: &Img) -> Result<EncodedFrame>;

    /// 用于诊断的可读后端名称（例如 "NVENC"、"AMF"、"openh264"）。
    fn name(&self) -> &'static str;
}

/// 将 `render(i)` 产生的 `frame_count` 帧流式写入 `output_path` 指定的 MP4 文件。
///
/// `render` 返回游戏区域帧和当前绝对谱面时间（毫秒）；`last_object_ms` 是最后一个
/// 可玩音符的绝对结束时间。绘制右上角“当前 / 总时长”标签前，两者都通过
/// `time_axis` 转换，因此总时长独立于所选导出范围及首尾留白。
/// 帧分块并行渲染并顺序编码以保持顺序；`fps` 同时作为编码帧率与 MP4 时间尺度
///（每帧一个 tick）。
pub(crate) fn save_mp4_streamed(
    frame_count: usize,
    chart_start_ms: i64,
    last_object_ms: i64,
    speed: f64,
    render: impl Fn(usize) -> (Img, i64) + Send + Sync,
    output_path: &Path,
    fps: u32,
    audio_job: AudioSourceJob,
    time_axis: TimeAxis,
    deadline: &RequestDeadline,
) -> Result<()> {
    deadline.check()?;
    if frame_count == 0 {
        return Err(PreviewError::render("no frames to encode"));
    }
    let video_started = Instant::now();
    let audio_sample_rate = crate::config::current().audio.video_audio.AUDIO_SAMPLE_RATE;
    let audio_bitrate = crate::config::current().audio.video_audio.AUDIO_BITRATE;
    let audio_freq_index = sample_freq_index(audio_sample_rate)?;
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| PreviewError::render(format!("failed to create output dir: {e}")))?;
    }

    let audio_deadline = deadline.clone();
    let mut audio_task = JoinedAudioTask::new(
        std::thread::spawn(move || {
            audio_deadline.check()?;
            let source = audio_job.wait()?;
            let encoded = encode_audio_segment(
                &source,
                chart_start_ms,
                frame_count,
                fps,
                speed,
                &audio_deadline,
            )?;
            crate::log::event(
                "audio-encode",
                "done",
                None,
                &format!(
                    "{} AAC frames from {} (lead-in={}ms)",
                    encoded.frames.len(),
                    source.path.display(),
                    source.lead_in_ms,
                ),
            );
            eprintln!(
                "[audio] encoded {} AAC frames from {} (lead-in={}ms)",
                encoded.frames.len(),
                source.path.display(),
                source.lead_in_ms,
            );
            Ok::<_, PreviewError>(encoded)
        }),
        deadline.clone(),
    );

    // ── 渲染首帧以确定游戏区域尺寸 ──
    let (first_frame, first_time) = render(0);
    deadline.check()?;
    let (pf_w, pf_h) = (first_frame.w, first_frame.h);
    let (out_w, out_h) = letterbox_dims(pf_w, pf_h);

    // ── 选择最佳可用编码后端 ──
    let mut encoder = create_encoder(out_w, out_h, fps)?;
    deadline.check()?;
    crate::log::event(
        "video-backend",
        "done",
        None,
        &format!("{} {}x{}@{}fps", encoder.name(), out_w, out_h, fps),
    );
    eprintln!(
        "[video] using {} backend ({}x{}@{}fps)",
        encoder.name(),
        out_w,
        out_h,
        fps
    );

    let frame_bytes = (out_w as usize)
        .saturating_mul(out_h as usize)
        .saturating_mul(4)
        .max(1);
    let par_chunk_size = (crate::config::current().video.video.MAX_PAR_FRAME_BYTES / frame_bytes)
        .clamp(1, crate::config::current().video.video.PAR_CHUNK_SIZE);

    // ── 编码首帧并提取 SPS/PPS，供 MP4 轨道配置使用 ──
    let first_comp = compose_frame(
        first_frame,
        time_axis.to_display(first_time),
        time_axis.to_display(last_object_ms),
        out_w,
        out_h,
    );
    let first_encoded = encoder.encode(&first_comp)?;
    deadline.check()?;
    if first_encoded.slice.is_empty() {
        return Err(PreviewError::render(format!(
            "{} returned an empty H.264 sample for frame 0",
            encoder.name()
        )));
    }
    let sps = first_encoded
        .sps
        .ok_or_else(|| PreviewError::render("missing SPS in first encoded frame"))?;
    let pps = first_encoded
        .pps
        .ok_or_else(|| PreviewError::render("missing PPS in first encoded frame"))?;

    // ── MP4 写入器 ──
    // 写入同目录临时文件，MP4 完成收尾后才原子替换最终路径，
    // 避免中断渲染留下可被缓存误用的残缺文件。编码器移入闭包再返回，
    // 使重命名后仍可抑制其会打印到 stdout 的 Drop 输出。
    let encoder = with_atomic_output_deadline(output_path, "mp4.tmp", deadline, |tmp_path| {
        let file = std::fs::File::create(tmp_path)
            .map_err(|e| PreviewError::render(format!("failed to write mp4: {e}")))?;
        let writer = BufWriter::new(file);
        let mp4_config = mp4::Mp4Config {
            major_brand: mp4::FourCC::from(*b"isom"),
            minor_version: 512,
            compatible_brands: vec![
                mp4::FourCC::from(*b"isom"),
                mp4::FourCC::from(*b"iso2"),
                mp4::FourCC::from(*b"avc1"),
                mp4::FourCC::from(*b"mp41"),
            ],
            timescale: fps,
        };
        let mut mp4_writer = mp4::Mp4Writer::write_start(writer, &mp4_config)
            .map_err(|e| PreviewError::render(format!("mp4 write_start failed: {e}")))?;

        let track_config = mp4::TrackConfig {
            track_type: mp4::TrackType::Video,
            timescale: fps,
            language: "und".to_string(),
            media_conf: mp4::MediaConfig::AvcConfig(mp4::AvcConfig {
                width: out_w as u16,
                height: out_h as u16,
                seq_param_set: sps,
                pic_param_set: pps,
            }),
        };
        mp4_writer
            .add_track(&track_config)
            .map_err(|e| PreviewError::render(format!("mp4 add_track failed: {e}")))?;

        let audio_track_config = mp4::TrackConfig {
            track_type: mp4::TrackType::Audio,
            timescale: audio_sample_rate,
            language: "und".to_string(),
            media_conf: mp4::MediaConfig::AacConfig(mp4::AacConfig {
                bitrate: audio_bitrate,
                profile: mp4::AudioObjectType::AacLowComplexity,
                freq_index: audio_freq_index,
                chan_conf: mp4::ChannelConfig::Stereo,
            }),
        };
        mp4_writer
            .add_track(&audio_track_config)
            .map_err(|e| PreviewError::render(format!("mp4 add audio track failed: {e}")))?;

        // 首个 sample（IDR，start_time = 0 tick）。
        let sample = mp4::Mp4Sample {
            start_time: 0,
            duration: 1,
            rendering_offset: 0,
            is_sync: true,
            bytes: Bytes::copy_from_slice(&first_encoded.slice),
        };
        mp4_writer
            .write_sample(1, &sample)
            .map_err(|e| PreviewError::render(format!("mp4 write_sample failed: {e}")))?;

        // ── 分块并行渲染和合成，顺序编码 ──
        // 将 compose_frame 移入并行循环，使其在 rayon 线程池内与渲染同时执行，
        // 消除串行合成瓶颈（4000 帧约 5 秒），避免 GPU 加速收益减半。
        let mut t_render = std::time::Duration::ZERO;
        let mut t_encode = std::time::Duration::ZERO;
        let mut t_mux = std::time::Duration::ZERO;
        let gameplay_total = time_axis.to_display(last_object_ms);
        for chunk_start in (1..frame_count).step_by(par_chunk_size) {
            deadline.check()?;
            let chunk_end = (chunk_start + par_chunk_size).min(frame_count);
            let t0 = Instant::now();
            let frames: Vec<Img> = (chunk_start..chunk_end)
                .into_par_iter()
                .map(|fi| {
                    let (pf, time) = render(fi);
                    // 在此并行合成，避免串行瓶颈。
                    compose_frame(pf, time_axis.to_display(time), gameplay_total, out_w, out_h)
                })
                .collect();
            deadline.check()?;
            t_render += t0.elapsed();

            for (i, comp) in (chunk_start..).zip(frames) {
                deadline.check()?;
                let t2 = Instant::now();
                let encoded = encoder.encode(&comp)?;
                deadline.check()?;
                t_encode += t2.elapsed();
                if encoded.slice.is_empty() {
                    return Err(PreviewError::render(format!(
                        "{} returned an empty H.264 sample for frame {i}",
                        encoder.name()
                    )));
                }

                let t3 = Instant::now();
                let sample = mp4::Mp4Sample {
                    start_time: i as u64,
                    duration: 1,
                    rendering_offset: 0,
                    is_sync: encoded.is_keyframe,
                    bytes: Bytes::copy_from_slice(&encoded.slice),
                };
                mp4_writer
                    .write_sample(1, &sample)
                    .map_err(|e| PreviewError::render(format!("mp4 write_sample failed: {e}")))?;
                t_mux += t3.elapsed();
            }
        }

        let video_elapsed = video_started.elapsed();
        if !audio_task.is_finished() {
            crate::log::event(
                "audio-wait",
                "start",
                None,
                "video samples written; waiting for audio",
            );
        }
        let audio_wait_start = Instant::now();
        deadline.check()?;
        let encoded_audio = audio_task.join()?;
        deadline.check()?;
        let audio_wait = audio_wait_start.elapsed();
        let mut audio_start = 0_u64;
        for frame in encoded_audio.frames {
            deadline.check()?;
            let sample = mp4::Mp4Sample {
                start_time: audio_start,
                duration: frame.duration,
                rendering_offset: 0,
                is_sync: true,
                bytes: Bytes::from(frame.bytes),
            };
            mp4_writer
                .write_sample(2, &sample)
                .map_err(|e| PreviewError::render(format!("mp4 audio write_sample failed: {e}")))?;
            audio_start += frame.duration as u64;
        }
        eprintln!(
            "[video] timing: render+compose={:.1}s encode={:.1}s mux={:.1}s audio-wait={:.1}s ({})",
            t_render.as_secs_f64(),
            t_encode.as_secs_f64(),
            t_mux.as_secs_f64(),
            audio_wait.as_secs_f64(),
            encoder.name(),
        );
        crate::log::record_video_stats(crate::log::VideoStats {
            backend: Some(encoder.name().to_string()),
            resolution: Some(format!("{out_w}x{out_h}")),
            fps: Some(fps),
            frame_count: Some(frame_count),
            video_ms: Some(video_elapsed.as_secs_f64() * 1000.0),
            render_compose_ms: Some(t_render.as_secs_f64() * 1000.0),
            encode_ms: Some(t_encode.as_secs_f64() * 1000.0),
            mux_ms: Some(t_mux.as_secs_f64() * 1000.0),
            audio_ms: Some(audio_wait.as_secs_f64() * 1000.0),
        });

        mp4_writer
            .write_end()
            .map_err(|e| PreviewError::render(format!("mp4 write_end failed: {e}")))?;
        // 取回 BufWriter 并刷新，确保临时文件覆盖最终路径前所有字节都已落盘。
        let mut writer = mp4_writer.into_writer();
        std::io::Write::flush(&mut writer)
            .map_err(|e| PreviewError::render(format!("mp4 flush failed: {e}")))?;
        drop(writer);
        mux::make_mp4_faststart(tmp_path)?;
        deadline.check()?;

        Ok(encoder)
    })?;

    // 返回前显式释放编码器。`nvenc` crate 的 Drop 实现使用 `println!` 向 stdout
    // 输出调试信息，会污染 JSON 输出。临时将 stdout 切换到 stderr，使调试信息
    // 写入 stderr，从而保持 JSON 输出纯净。
    drop_stdout_silence(|| {
        drop(encoder);
    });

    Ok(())
}

struct JoinedAudioTask {
    handle: Option<std::thread::JoinHandle<Result<audio::EncodedAudio>>>,
    deadline: RequestDeadline,
}

impl JoinedAudioTask {
    fn new(
        handle: std::thread::JoinHandle<Result<audio::EncodedAudio>>,
        deadline: RequestDeadline,
    ) -> Self {
        Self {
            handle: Some(handle),
            deadline,
        }
    }

    fn is_finished(&self) -> bool {
        self.handle
            .as_ref()
            .is_none_or(|handle| handle.is_finished())
    }

    fn join(&mut self) -> Result<audio::EncodedAudio> {
        self.handle
            .take()
            .expect("audio task joined more than once")
            .join()
            .map_err(|_| PreviewError::render("audio encoding worker panicked"))?
    }
}

impl Drop for JoinedAudioTask {
    fn drop(&mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        self.deadline.cancel();
        let _ = handle.join();
    }
}

fn sample_freq_index(sample_rate: u32) -> Result<mp4::SampleFreqIndex> {
    let index = match sample_rate {
        96_000 => mp4::SampleFreqIndex::Freq96000,
        88_200 => mp4::SampleFreqIndex::Freq88200,
        64_000 => mp4::SampleFreqIndex::Freq64000,
        48_000 => mp4::SampleFreqIndex::Freq48000,
        44_100 => mp4::SampleFreqIndex::Freq44100,
        32_000 => mp4::SampleFreqIndex::Freq32000,
        24_000 => mp4::SampleFreqIndex::Freq24000,
        22_050 => mp4::SampleFreqIndex::Freq22050,
        16_000 => mp4::SampleFreqIndex::Freq16000,
        12_000 => mp4::SampleFreqIndex::Freq12000,
        11_025 => mp4::SampleFreqIndex::Freq11025,
        8_000 => mp4::SampleFreqIndex::Freq8000,
        7_350 => mp4::SampleFreqIndex::Freq7350,
        _ => {
            return Err(PreviewError::render(format!(
                "unsupported configured audio sample rate: {sample_rate}"
            )))
        }
    };
    Ok(index)
}

/// 按优先级尝试硬件编码器，全部不可用时回退到 CPU openh264。
fn create_encoder(w: u32, h: u32, fps: u32) -> Result<Box<dyn FrameEncoder>> {
    // `OSU_PREVIEW_NO_GPU=1` 强制使用 CPU 路径（用于基准测试或回退）。
    let force_cpu = std::env::var("OSU_PREVIEW_NO_GPU").as_deref() == Ok("1");
    // 1. NVENC（NVIDIA）——仅 Windows。
    #[cfg(windows)]
    if !force_cpu {
        if let Some(enc) = nvenc::try_create(w, h, fps)? {
            return Ok(Box::new(enc));
        }
    }
    // 2. AMF（AMD）——仅 Windows（amfrt64.dll）。
    #[cfg(windows)]
    if !force_cpu {
        if let Some(enc) = amf::try_create(w, h, fps)? {
            return Ok(Box::new(enc));
        }
    }
    // 3. CPU 回退（始终可用）。
    Ok(Box::new(cpu::CpuEncoder::new(w, h, fps)?))
}

/// 计算游戏区域帧的 16:9 信箱画布尺寸，并取整到偶数
///（YUV420 要求宽高都是 2 的倍数）。
fn letterbox_dims(pf_w: u32, pf_h: u32) -> (u32, u32) {
    let (w, h) = if pf_w as f64 * 9.0 > pf_h as f64 * 16.0 {
        // 游戏区域宽于 16:9：上下补边。
        (pf_w, (pf_w as f64 * 9.0 / 16.0).round() as u32)
    } else {
        // 游戏区域窄于 16:9：左右补边。
        ((pf_h as f64 * 16.0 / 9.0).round() as u32, pf_h)
    };
    (w.max(2) & !1, h.max(2) & !1)
}

/// 将游戏区域帧居中放置到黑色 16:9 画布，并在右上角绘制
///“当前 / 总时长”游戏时间标签。
fn compose_frame(pf: Img, current_ms: i64, total_ms: i64, out_w: u32, out_h: u32) -> Img {
    let mut canvas = Img::new(
        out_w,
        out_h,
        crate::config::current().video.video.BLACK_OPAQUE,
    );
    let ox = ((out_w - pf.w) / 2) as i64;
    let oy = ((out_h - pf.h) / 2) as i64;
    canvas.alpha_composite(&pf, ox, oy);

    let label = format_progress_label(current_ms, total_ms);
    let (lw, _) = text_size(&label, crate::config::current().video.video.LABEL_FONT_SIZE);
    let lx = out_w as i64 - lw as i64 - crate::config::current().video.video.LABEL_PAD;
    draw_text(
        &mut canvas,
        lx,
        crate::config::current().video.video.LABEL_PAD,
        &label,
        crate::config::current().video.video.LABEL_FONT_SIZE,
        crate::config::current().video.video.LABEL_COLOR,
    );
    canvas
}

fn format_progress_label(current_ms: i64, total_ms: i64) -> String {
    format!("{}/{}", format_mmss(current_ms), format_mmss(total_ms))
}

fn format_mmss(ms: i64) -> String {
    crate::render::text::format_mmss_floor(ms)
}

/// 临时将 stdout 重定向到 stderr，避免第三方 `println!` 调用
///（`nvenc` crate 的 Drop 调试信息）污染 stdout。stdout 必须只包含 JSON 结果；
/// 闭包返回后恢复 stdout。
#[cfg(windows)]
fn drop_stdout_silence<F: FnOnce()>(f: F) {
    use std::io::Write;
    use windows::Win32::Foundation::{
        CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, INVALID_HANDLE_VALUE,
    };
    use windows::Win32::System::Console::{
        GetStdHandle, SetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    let stderr_handle = unsafe { GetStdHandle(STD_ERROR_HANDLE) }.unwrap_or(INVALID_HANDLE_VALUE);
    let cur_proc = unsafe { GetCurrentProcess() };
    let mut dup = INVALID_HANDLE_VALUE;
    let ok = unsafe {
        DuplicateHandle(
            cur_proc,
            stderr_handle,
            cur_proc,
            &mut dup,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if ok.is_err() || dup == INVALID_HANDLE_VALUE {
        f();
        return;
    }
    let old_stdout = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }.unwrap_or(INVALID_HANDLE_VALUE);
    let _ = std::io::stdout().flush();
    let _ = unsafe { SetStdHandle(STD_OUTPUT_HANDLE, dup) };
    f();
    let _ = std::io::stdout().flush();
    let _ = unsafe { SetStdHandle(STD_OUTPUT_HANDLE, old_stdout) };
    let _ = unsafe { CloseHandle(dup) };
}

#[cfg(not(windows))]
fn drop_stdout_silence<F: FnOnce()>(f: F) {
    f();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::{HitObjects, KvSection};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn beatmap_with_preview(preview_time: Option<&str>, lead_in: Option<&str>) -> Beatmap {
        let mut general = KvSection::default();
        if let Some(value) = preview_time {
            general.insert("PreviewTime", value.to_string());
        }
        if let Some(value) = lead_in {
            general.insert("AudioLeadIn", value.to_string());
        }
        Beatmap {
            metadata: KvSection::default(),
            difficulty: KvSection::default(),
            general,
            timing_points: Vec::new(),
            hit_objects: HitObjects::Standard(Vec::new()),
            break_periods: Vec::new(),
            combo_colors: Vec::new(),
            beat_divisor: 0,
        }
    }

    #[test]
    fn preview_start_uses_preview_time_and_duration() {
        let beatmap = beatmap_with_preview(Some("45000"), None);
        let range = resolve_video_time_range(
            &beatmap,
            10_000,
            100_000,
            Some(TimePoint::Preview),
            Some(30.0),
            1.0,
        )
        .unwrap();
        assert_eq!(
            range,
            VideoTimeRange {
                start: 45_000,
                end: 75_000
            }
        );
    }

    #[test]
    fn default_start_uses_requested_duration_on_a_long_chart() {
        let beatmap = beatmap_with_preview(None, None);
        let range =
            resolve_video_time_range(&beatmap, 10_000, 100_000, None, Some(60.0), 1.0).unwrap();
        assert_eq!(
            range,
            VideoTimeRange {
                start: 10_000,
                end: 70_000
            }
        );
    }

    #[test]
    fn numeric_start_shifts_backward_when_it_runs_past_chart_tail() {
        let beatmap = beatmap_with_preview(None, None);
        let range = resolve_video_time_range(
            &beatmap,
            10_000,
            100_000,
            Some(TimePoint::Seconds(50.0)),
            Some(60.0),
            1.0,
        )
        .unwrap();
        assert_eq!(
            range,
            VideoTimeRange {
                start: 42_000,
                end: 102_000
            }
        );
    }

    #[test]
    fn short_chart_returns_full_playable_range_instead_of_padding_to_duration() {
        let beatmap = beatmap_with_preview(None, None);
        let range =
            resolve_video_time_range(&beatmap, 10_000, 20_000, None, Some(60.0), 1.0).unwrap();
        assert_eq!(
            range,
            VideoTimeRange {
                start: 8_000,
                end: 22_000
            }
        );
    }

    #[test]
    fn short_chart_uses_full_range_even_with_a_negative_requested_start() {
        let beatmap = beatmap_with_preview(None, None);
        let range = resolve_video_time_range(
            &beatmap,
            10_000,
            20_000,
            Some(TimePoint::Seconds(-20.0)),
            Some(60.0),
            1.0,
        )
        .unwrap();
        assert_eq!(
            range,
            VideoTimeRange {
                start: 8_000,
                end: 22_000
            }
        );
    }

    #[test]
    fn negative_start_is_preserved_when_tail_adjustment_is_not_needed() {
        let beatmap = beatmap_with_preview(None, None);
        let range = resolve_video_time_range(
            &beatmap,
            10_000,
            100_000,
            Some(TimePoint::Seconds(-20.0)),
            Some(60.0),
            1.0,
        )
        .unwrap();
        assert_eq!(
            range,
            VideoTimeRange {
                start: -10_000,
                end: 50_000
            }
        );
    }

    #[test]
    fn speed_multiplier_scales_chart_span() {
        let beatmap = beatmap_with_preview(None, None);
        let range =
            resolve_video_time_range(&beatmap, 10_000, 100_000, None, Some(30.0), 1.5).unwrap();
        assert_eq!(
            range,
            VideoTimeRange {
                start: 10_000,
                end: 55_000
            }
        );
    }

    #[test]
    fn numeric_start_is_relative_to_first_object() {
        let beatmap = beatmap_with_preview(None, None);
        let range = resolve_video_time_range(
            &beatmap,
            10_000,
            100_000,
            Some(TimePoint::Seconds(-2.0)),
            Some(10.0),
            1.5,
        )
        .unwrap();
        assert_eq!(
            range,
            VideoTimeRange {
                start: 8_000,
                end: 23_000
            }
        );
    }

    #[test]
    fn progress_label_uses_current_skin_time_and_full_playable_duration() {
        let time_axis = TimeAxis::new(12_500);
        let total_ms = time_axis.to_display(102_500);

        assert_eq!(total_ms, 90_000);
        assert_eq!(
            format_progress_label(time_axis.to_display(12_000), total_ms),
            "-0:01/1:30"
        );
        assert_eq!(
            format_progress_label(time_axis.to_display(92_500), total_ms),
            "1:20/1:30"
        );
    }

    #[test]
    fn dropping_audio_task_cancels_and_joins_worker() {
        let deadline = RequestDeadline::new(Instant::now(), "mp4", Duration::from_secs(300));
        let worker_deadline = deadline.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = finished.clone();
        let task = JoinedAudioTask::new(
            std::thread::spawn(move || loop {
                if let Err(error) = worker_deadline.check() {
                    worker_finished.store(true, Ordering::Relaxed);
                    return Err(error);
                }
                std::thread::yield_now();
            }),
            deadline,
        );

        drop(task);
        assert!(finished.load(Ordering::Relaxed));
    }
}
