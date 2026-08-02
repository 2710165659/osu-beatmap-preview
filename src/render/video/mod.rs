//! MP4 (H.264) video encoder: streams frames produced by a callback into an
//! MP4 file via H.264 + the `mp4` crate. Mirrors `save_animated_gif_streamed`:
//! parallel render chunks (rayon) + sequential encode to preserve frame order.
//!
//! Each rendered playfield frame is letterboxed into a 16:9 black canvas with a
//! "current / total" time label in the top-right, converted to the format the
//! selected backend expects, encoded as H.264, and written as one MP4 sample.
//! The full animation never resides in memory at once — at most `PAR_CHUNK_SIZE`
//! raw frames are held.
//!
//! ## GPU acceleration
//!
//! Encoding is dispatched to the first available hardware backend:
//!   1. **NVENC** (NVIDIA) — dynamically loads `nvEncodeAPI64.dll` at runtime.
//!   2. **AMF** (AMD) — dynamically loads `amfrt64.dll` at runtime.
//!   3. **openh264** (CPU) — always available fallback, single-threaded software
//!      encoder (the original implementation).
//!
//! All backends emit Annex-B H.264 NALs which are fed through the shared mux
//! layer (`extract_nals` + `mp4` writer), so the output files are byte-for-byte
//! compatible in structure. GPU DLLs are loaded via `libloading`; their absence
//! at build time or runtime never breaks compilation or execution — the encoder
//! silently falls back to CPU.

use crate::common::time_selection::TimeAxis;
use crate::core::errors::{PreviewError, Result};
use crate::core::models::Beatmap;
use crate::parser::round_half_even;
use crate::render::canvas::Img;
use crate::render::text::{draw_text, text_size};
use crate::render::video::audio::{
    encode_audio_segment, full_video_start_time, AudioSourceJob, AUDIO_BITRATE, AUDIO_SAMPLE_RATE,
};
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

/// Parallel-render chunk size (matches GIF: ~8 frames at once).
const PAR_CHUNK_SIZE: usize = 8;
/// Bound transient RGBA frame memory for unusually large playfields.
const MAX_PAR_FRAME_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const VIDEO_BITRATE: u32 = 900_000;
pub(crate) const VIDEO_END_PADDING_MS: i64 = 2_000;
const PREVIEW_30S_ACTUAL_MS: i64 = 30_000;

const LABEL_COLOR: [u8; 4] = [232, 232, 232, 255];
const LABEL_FONT_SIZE: u32 = 24;
const LABEL_PAD: i64 = 12;
const BLACK_OPAQUE: [u8; 4] = [0, 0, 0, 255];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VideoTimeRange {
    pub(crate) start: i64,
    pub(crate) end: i64,
}

pub(crate) fn resolve_video_time_range(
    beatmap: &Beatmap,
    first_object_ms: i64,
    last_object_ms: i64,
    times_ms: Option<&[i64]>,
    preview_30s: bool,
    speed: f64,
) -> Result<VideoTimeRange> {
    if preview_30s && times_ms.is_some() {
        return Err(PreviewError::new(
            "--preview-30s cannot be used together with --time",
        ));
    }

    if let Some(times) = times_ms {
        if times.len() != 2 {
            return Err(PreviewError::new(
                "--time for mp4 needs exactly 2 values t1+t2 (or omit for the full chart)",
            ));
        }
        return validate_video_time_range(VideoTimeRange {
            start: times[0],
            end: times[1],
        });
    }

    let full_start = full_video_start_time(first_object_ms, beatmap.audio_lead_in_ms());
    let full_end = last_object_ms + VIDEO_END_PADDING_MS;
    let full_range = validate_video_time_range(VideoTimeRange {
        start: full_start,
        end: full_end,
    })?;

    if !preview_30s {
        return Ok(full_range);
    }

    let span = chart_span_for_actual_duration(PREVIEW_30S_ACTUAL_MS, speed)?;
    if full_range.end - full_range.start <= span {
        return Ok(full_range);
    }

    let mut start = preview_time_or_first_object(beatmap, first_object_ms).max(full_range.start);
    if start + span > full_range.end {
        start = full_range.end - span;
    }
    start = start.max(full_range.start);
    validate_video_time_range(VideoTimeRange {
        start,
        end: start + span,
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

/// An encoded H.264 frame ready to be muxed into the MP4 container.
///
/// `sps`/`pps` are `Some` only on frames that carry them. `slice` is the
/// length-prefixed slice NAL data, and `is_keyframe` reflects an actual IDR NAL.
struct EncodedFrame {
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    slice: Vec<u8>,
    is_keyframe: bool,
}

/// A backend H.264 encoder that consumes composed RGBA frames and produces
/// Annex-B NALs. Implementations own their encoder state (GPU session, CPU
/// codec, etc.) and must be fed frames in order.
///
/// Safety contract: `encode` is called sequentially from a single thread (the
/// mux loop), so backends need not be `Sync` — but the trait object is held
/// across rayon parallel render chunks, so it must not be borrowed during
/// `into_par_iter`. The dispatch in `save_mp4_streamed` encodes *after* the
/// parallel collect, so this is safe.
trait FrameEncoder {
    /// Encode one composed RGBA frame. Returns the NAL units split into
    /// SPS / PPS / slice for muxing.
    fn encode(&mut self, rgba: &Img) -> Result<EncodedFrame>;

    /// Human-readable backend name for diagnostics (e.g. "NVENC", "AMF", "openh264").
    fn name(&self) -> &'static str;
}

/// Stream `frame_count` frames produced by `render(i)` into an MP4 file at
/// `output_path`.
///
/// `render` returns the playfield frame and the current absolute chart time
/// (ms). `last_object_ms` is the absolute end of the last playable object;
/// both are converted through `time_axis` before drawing the top-right
/// "current / total" gameplay label. The total is therefore independent of
/// the selected export range and its leading/trailing padding.
/// Frames are rendered in parallel chunks and encoded sequentially to preserve
/// ordering; `fps` is both the encode frame rate and the MP4 timescale (1 tick
/// per frame).
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
) -> Result<()> {
    if frame_count == 0 {
        return Err(PreviewError::render("no frames to encode"));
    }
    let video_started = Instant::now();
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| PreviewError::render(format!("failed to create output dir: {e}")))?;
    }

    let audio_task = std::thread::spawn(move || {
        let source = audio_job.wait()?;
        let encoded = encode_audio_segment(&source, chart_start_ms, frame_count, fps, speed)?;
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
    });

    // ── render first frame to discover playfield dimensions ──
    let (first_frame, first_time) = render(0);
    let (pf_w, pf_h) = (first_frame.w, first_frame.h);
    let (out_w, out_h) = letterbox_dims(pf_w, pf_h);

    // ── pick the best available encoder backend ──
    let mut encoder = create_encoder(out_w, out_h, fps)?;
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
    let par_chunk_size = (MAX_PAR_FRAME_BYTES / frame_bytes).clamp(1, PAR_CHUNK_SIZE);

    // ── encode first frame, extract SPS/PPS for the mp4 track config ──
    let first_comp = compose_frame(
        first_frame,
        time_axis.to_display(first_time),
        time_axis.to_display(last_object_ms),
        out_w,
        out_h,
    );
    let first_encoded = encoder.encode(&first_comp)?;
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

    // ── mp4 writer ──
    // Write to a sibling temp file and atomically replace the final path only
    // after the MP4 is fully finalized, so an interrupted render never leaves
    // a partial file that could be served from cache. The encoder is moved
    // into the closure and returned so its `Drop` (which prints to stdout)
    // can still be silenced after the rename.
    let encoder = crate::pipeline::cache::with_atomic_output(output_path, "mp4.tmp", |tmp_path| {
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
            timescale: AUDIO_SAMPLE_RATE,
            language: "und".to_string(),
            media_conf: mp4::MediaConfig::AacConfig(mp4::AacConfig {
                bitrate: AUDIO_BITRATE,
                profile: mp4::AudioObjectType::AacLowComplexity,
                freq_index: mp4::SampleFreqIndex::Freq48000,
                chan_conf: mp4::ChannelConfig::Stereo,
            }),
        };
        mp4_writer
            .add_track(&audio_track_config)
            .map_err(|e| PreviewError::render(format!("mp4 add audio track failed: {e}")))?;

        // first sample (IDR, start_time = 0 ticks)
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

        // ── render + compose in parallel chunks, encode sequentially ──
        // compose_frame is moved into the parallel loop so it runs alongside
        // render on rayon's thread pool — this eliminates the serial compose
        // bottleneck (~5s for 4000 frames) that previously halved the GPU
        // speedup.
        let mut t_render = std::time::Duration::ZERO;
        let mut t_encode = std::time::Duration::ZERO;
        let mut t_mux = std::time::Duration::ZERO;
        let gameplay_total = time_axis.to_display(last_object_ms);
        for chunk_start in (1..frame_count).step_by(par_chunk_size) {
            let chunk_end = (chunk_start + par_chunk_size).min(frame_count);
            let t0 = Instant::now();
            let frames: Vec<Img> = (chunk_start..chunk_end)
                .into_par_iter()
                .map(|fi| {
                    let (pf, time) = render(fi);
                    // compose here, in parallel — avoids serial bottleneck
                    compose_frame(pf, time_axis.to_display(time), gameplay_total, out_w, out_h)
                })
                .collect();
            t_render += t0.elapsed();

            for (i, comp) in (chunk_start..).zip(frames) {
                let t2 = Instant::now();
                let encoded = encoder.encode(&comp)?;
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
        let encoded_audio = audio_task
            .join()
            .map_err(|_| PreviewError::render("audio encoding worker panicked"))??;
        let audio_wait = audio_wait_start.elapsed();
        let mut audio_start = 0_u64;
        for frame in encoded_audio.frames {
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
        // Recover the BufWriter and flush it so every byte is on disk before
        // the temp file is renamed over the final path.
        let mut writer = mp4_writer.into_writer();
        std::io::Write::flush(&mut writer)
            .map_err(|e| PreviewError::render(format!("mp4 flush failed: {e}")))?;
        drop(writer);
        mux::make_mp4_faststart(tmp_path)?;

        Ok(encoder)
    })?;

    // Explicitly drop the encoder before returning. The `nvenc` crate's Drop
    // impl uses `println!` (stdout) for debug messages ("Dropping bitstream
    // buffer" / "Dropping encoder"), which would pollute the JSON output on
    // stdout. We temporarily swap stdout→stderr so those messages go to stderr
    // instead, keeping stdout clean for the JSON payload.
    drop_stdout_silence(|| {
        drop(encoder);
    });

    Ok(())
}

/// Try hardware encoders in priority order, fall back to CPU openh264.
fn create_encoder(w: u32, h: u32, fps: u32) -> Result<Box<dyn FrameEncoder>> {
    // `OSU_PREVIEW_NO_GPU=1` forces the CPU path (for benchmarking / fallback).
    let force_cpu = std::env::var("OSU_PREVIEW_NO_GPU").as_deref() == Ok("1");
    // 1. NVENC (NVIDIA) — Windows only
    #[cfg(windows)]
    if !force_cpu {
        if let Some(enc) = nvenc::try_create(w, h, fps)? {
            return Ok(Box::new(enc));
        }
    }
    // 2. AMF (AMD) — Windows only (amfrt64.dll)
    #[cfg(windows)]
    if !force_cpu {
        if let Some(enc) = amf::try_create(w, h, fps)? {
            return Ok(Box::new(enc));
        }
    }
    // 3. CPU fallback (always available)
    Ok(Box::new(cpu::CpuEncoder::new(w, h, fps)?))
}

/// Compute the 16:9 letterbox canvas size for a playfield frame, rounding out
/// to even dimensions (YUV420 requires width and height to be multiples of 2).
fn letterbox_dims(pf_w: u32, pf_h: u32) -> (u32, u32) {
    let (w, h) = if pf_w as f64 * 9.0 > pf_h as f64 * 16.0 {
        // playfield wider than 16:9 → pad top/bottom
        (pf_w, (pf_w as f64 * 9.0 / 16.0).round() as u32)
    } else {
        // playfield narrower than 16:9 → pad left/right
        ((pf_h as f64 * 16.0 / 9.0).round() as u32, pf_h)
    };
    (w.max(2) & !1, h.max(2) & !1)
}

/// Place the playfield frame centered on a black 16:9 canvas and draw the
/// "current / total" gameplay-time label in the top-right corner.
fn compose_frame(pf: Img, current_ms: i64, total_ms: i64, out_w: u32, out_h: u32) -> Img {
    let mut canvas = Img::new(out_w, out_h, BLACK_OPAQUE);
    let ox = ((out_w - pf.w) / 2) as i64;
    let oy = ((out_h - pf.h) / 2) as i64;
    canvas.alpha_composite(&pf, ox, oy);

    let label = format_progress_label(current_ms, total_ms);
    let (lw, _) = text_size(&label, LABEL_FONT_SIZE);
    let lx = out_w as i64 - lw as i64 - LABEL_PAD;
    draw_text(
        &mut canvas,
        lx,
        LABEL_PAD,
        &label,
        LABEL_FONT_SIZE,
        LABEL_COLOR,
    );
    canvas
}

fn format_progress_label(current_ms: i64, total_ms: i64) -> String {
    format!("{}/{}", format_mmss(current_ms), format_mmss(total_ms))
}

fn format_mmss(ms: i64) -> String {
    crate::render::text::format_mmss_floor(ms)
}

/// Temporarily redirect stdout → stderr so that third-party `println!` calls
/// (the `nvenc` crate's `Drop` debug messages: "Dropping bitstream buffer" /
/// "Dropping encoder") don't pollute stdout, which must contain only the JSON
/// result. After the closure returns, stdout is restored.
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
    fn preview_30s_uses_preview_time_when_available() {
        let beatmap = beatmap_with_preview(Some("45000"), None);
        let range = resolve_video_time_range(&beatmap, 10_000, 100_000, None, true, 1.0).unwrap();
        assert_eq!(
            range,
            VideoTimeRange {
                start: 45_000,
                end: 75_000
            }
        );
    }

    #[test]
    fn preview_30s_falls_back_to_first_object_for_missing_invalid_or_negative_preview_time() {
        for preview_time in [None, Some("abc"), Some("-1")] {
            let beatmap = beatmap_with_preview(preview_time, None);
            let range =
                resolve_video_time_range(&beatmap, 10_000, 100_000, None, true, 1.0).unwrap();
            assert_eq!(range.start, 10_000);
            assert_eq!(range.end, 40_000);
        }
    }

    #[test]
    fn preview_30s_scales_chart_span_to_keep_actual_video_duration() {
        let beatmap = beatmap_with_preview(Some("10000"), None);
        let dt = resolve_video_time_range(&beatmap, 10_000, 100_000, None, true, 1.5).unwrap();
        assert_eq!(
            dt,
            VideoTimeRange {
                start: 10_000,
                end: 55_000
            }
        );

        let ht = resolve_video_time_range(&beatmap, 10_000, 100_000, None, true, 0.75).unwrap();
        assert_eq!(
            ht,
            VideoTimeRange {
                start: 10_000,
                end: 32_500
            }
        );
    }

    #[test]
    fn preview_30s_shifts_back_when_preview_tail_is_too_short() {
        let beatmap = beatmap_with_preview(Some("40000"), None);
        let range = resolve_video_time_range(&beatmap, 10_000, 50_000, None, true, 1.0).unwrap();
        assert_eq!(
            range,
            VideoTimeRange {
                start: 22_000,
                end: 52_000
            }
        );
    }

    #[test]
    fn preview_30s_returns_short_clip_when_whole_chart_range_is_shorter_than_30s() {
        let beatmap = beatmap_with_preview(Some("15000"), None);
        let range = resolve_video_time_range(&beatmap, 10_000, 20_000, None, true, 1.0).unwrap();
        assert_eq!(
            range,
            VideoTimeRange {
                start: 8_000,
                end: 22_000
            }
        );
    }

    #[test]
    fn explicit_time_range_is_preserved() {
        let beatmap = beatmap_with_preview(Some("45000"), None);
        let range =
            resolve_video_time_range(&beatmap, 10_000, 100_000, Some(&[3_000, 9_000]), false, 1.0)
                .unwrap();
        assert_eq!(
            range,
            VideoTimeRange {
                start: 3_000,
                end: 9_000
            }
        );
    }

    #[test]
    fn explicit_time_range_rejects_unrepresentable_duration() {
        let beatmap = beatmap_with_preview(None, None);
        assert!(resolve_video_time_range(
            &beatmap,
            10_000,
            100_000,
            Some(&[i64::MIN, i64::MAX]),
            false,
            1.0,
        )
        .is_err());
    }

    #[test]
    fn preview_30s_conflicts_with_explicit_time_range() {
        let beatmap = beatmap_with_preview(Some("45000"), None);
        let err =
            resolve_video_time_range(&beatmap, 10_000, 100_000, Some(&[3_000, 9_000]), true, 1.0)
                .unwrap_err();
        assert!(err.to_string().contains("--preview-30s"));
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
}
