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

use crate::canvas::Img;
use crate::errors::{PreviewError, Result};
use crate::text::{draw_text, text_size};
use bytes::Bytes;
use rayon::prelude::*;
use std::io::BufWriter;
use std::path::Path;
use std::time::Instant;

#[allow(non_camel_case_types, dead_code)]
mod amf;
mod cpu;
mod mux;
mod nvenc;

/// Parallel-render chunk size (matches GIF: ~8 frames at once).
const PAR_CHUNK_SIZE: usize = 8;

const LABEL_COLOR: [u8; 4] = [232, 232, 232, 255];
const LABEL_FONT_SIZE: u32 = 24;
const LABEL_PAD: i64 = 12;
const BLACK_OPAQUE: [u8; 4] = [0, 0, 0, 255];

/// An encoded H.264 frame ready to be muxed into the MP4 container.
///
/// `sps`/`pps` are `Some` only on the frame that carries them (the first IDR);
/// subsequent frames pass `None`. `slice` is the length-prefixed slice NAL data.
struct EncodedFrame {
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    slice: Vec<u8>,
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
/// `render` returns the playfield frame and the current chart time (ms) used
/// for the top-right "current / total" label. `chart_end_ms` is the absolute
/// end time of the rendered span; the label shows the current frame's chart
/// time against this end time so both values are on the same game time axis
/// (e.g. 0:03 / 1:13). Frames are rendered in parallel chunks and encoded
/// sequentially to preserve ordering; `fps` is both the encode frame rate and
/// the MP4 timescale (1 tick per frame).
pub(crate) fn save_mp4_streamed(
    frame_count: usize,
    chart_end_ms: i64,
    render: impl Fn(usize) -> (Img, i64) + Send + Sync,
    output_path: &Path,
    fps: u32,
) -> Result<()> {
    if frame_count == 0 {
        return Err(PreviewError::render("no frames to encode"));
    }
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| PreviewError::render(format!("failed to create output dir: {e}")))?;
    }

    // ── render first frame to discover playfield dimensions ──
    let (first_frame, first_time) = render(0);
    let (pf_w, pf_h) = (first_frame.w, first_frame.h);
    let (out_w, out_h) = letterbox_dims(pf_w, pf_h);

    // ── pick the best available encoder backend ──
    let mut encoder = create_encoder(out_w, out_h, fps)?;
    eprintln!("[video] using {} backend ({}x{}@{}fps)", encoder.name(), out_w, out_h, fps);

    let keyframe_period = (fps as usize * 2).max(1);

    // ── encode first frame, extract SPS/PPS for the mp4 track config ──
    let first_comp = compose_frame(first_frame, first_time, chart_end_ms, out_w, out_h);
    let first_encoded = encoder.encode(&first_comp)?;
    let sps = first_encoded
        .sps
        .ok_or_else(|| PreviewError::render("missing SPS in first encoded frame"))?;
    let pps = first_encoded
        .pps
        .ok_or_else(|| PreviewError::render("missing PPS in first encoded frame"))?;

    // ── mp4 writer ──
    let file = std::fs::File::create(output_path)
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
    // compose_frame is moved into the parallel loop so it runs alongside render
    // on rayon's thread pool — this eliminates the serial compose bottleneck
    // (~5s for 4000 frames) that previously halved the GPU speedup.
    let mut t_render = std::time::Duration::ZERO;
    let mut t_encode = std::time::Duration::ZERO;
    let mut t_mux = std::time::Duration::ZERO;
    let chart_end = chart_end_ms;
    for chunk_start in (1..frame_count).step_by(PAR_CHUNK_SIZE) {
        let chunk_end = (chunk_start + PAR_CHUNK_SIZE).min(frame_count);
        let t0 = Instant::now();
        let frames: Vec<Img> = (chunk_start..chunk_end)
            .into_par_iter()
            .map(|fi| {
                let (pf, time) = render(fi);
                // compose here, in parallel — avoids serial bottleneck
                compose_frame(pf, time, chart_end, out_w, out_h)
            })
            .collect();
        t_render += t0.elapsed();

        for (i, comp) in (chunk_start..).zip(frames) {
            let is_keyframe = i % keyframe_period == 0;

            let t2 = Instant::now();
            let encoded = encoder.encode(&comp)?;
            t_encode += t2.elapsed();

            let t3 = Instant::now();
            let sample = mp4::Mp4Sample {
                start_time: i as u64,
                duration: 1,
                rendering_offset: 0,
                is_sync: is_keyframe,
                bytes: Bytes::copy_from_slice(&encoded.slice),
            };
            mp4_writer
                .write_sample(1, &sample)
                .map_err(|e| PreviewError::render(format!("mp4 write_sample failed: {e}")))?;
            t_mux += t3.elapsed();
        }
    }
    eprintln!(
        "[video] timing: render+compose={:.1}s encode={:.1}s mux={:.1}s ({})",
        t_render.as_secs_f64(),
        t_encode.as_secs_f64(),
        t_mux.as_secs_f64(),
        encoder.name(),
    );

    mp4_writer
        .write_end()
        .map_err(|e| PreviewError::render(format!("mp4 write_end failed: {e}")))?;

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
    // 1. NVENC (NVIDIA)
    if !force_cpu {
        if let Some(enc) = nvenc::try_create(w, h, fps)? {
            return Ok(Box::new(enc));
        }
    }
    // 2. AMF (AMD)
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
/// "current / total" time label in the top-right corner. `current_ms` is the
/// frame's absolute chart time; `end_ms` is the absolute end of the rendered
/// span — both on the same game time axis.
fn compose_frame(pf: Img, current_ms: i64, end_ms: i64, out_w: u32, out_h: u32) -> Img {
    let mut canvas = Img::new(out_w, out_h, BLACK_OPAQUE);
    let ox = ((out_w - pf.w) / 2) as i64;
    let oy = ((out_h - pf.h) / 2) as i64;
    canvas.alpha_composite(&pf, ox, oy);

    let label = format!("{}/{}", format_mmss(current_ms), format_mmss(end_ms));
    let (lw, _) = text_size(&label, LABEL_FONT_SIZE);
    let lx = out_w as i64 - lw as i64 - LABEL_PAD;
    draw_text(&mut canvas, lx, LABEL_PAD, &label, LABEL_FONT_SIZE, LABEL_COLOR);
    canvas
}

fn format_mmss(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Temporarily redirect stdout → stderr so that third-party `println!` calls
/// (the `nvenc` crate's `Drop` debug messages: "Dropping bitstream buffer" /
/// "Dropping encoder") don't pollute stdout, which must contain only the JSON
/// result. After the closure returns, stdout is restored.
#[cfg(windows)]
fn drop_stdout_silence<F: FnOnce()>(f: F) {
    use std::io::Write;
    use windows::Win32::Foundation::{CloseHandle, DuplicateHandle, INVALID_HANDLE_VALUE, DUPLICATE_SAME_ACCESS};
    use windows::Win32::System::Console::{
        GetStdHandle, SetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    let stderr_handle = unsafe { GetStdHandle(STD_ERROR_HANDLE) }.unwrap_or(INVALID_HANDLE_VALUE);
    let cur_proc = unsafe { GetCurrentProcess() };
    let mut dup = INVALID_HANDLE_VALUE;
    let ok = unsafe {
        DuplicateHandle(cur_proc, stderr_handle, cur_proc, &mut dup, 0, false, DUPLICATE_SAME_ACCESS)
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
