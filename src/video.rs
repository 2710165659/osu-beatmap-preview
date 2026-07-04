//! MP4 (H.264) video encoder: streams frames produced by a callback into an
//! MP4 file via openh264 + the `mp4` crate. Mirrors `save_animated_gif_streamed`:
//! parallel render chunks (rayon) + sequential encode to preserve frame order.
//!
//! Each rendered playfield frame is letterboxed into a 16:9 black canvas with a
//! "current / total" time label in the top-right, converted to YUV420, encoded
//! as H.264, and written as one MP4 sample. The full animation never resides in
//! memory at once — at most `PAR_CHUNK_SIZE` raw frames are held.

use crate::canvas::Img;
use crate::errors::{PreviewError, Result};
use crate::text::{draw_text, text_size};
use bytes::Bytes;
use openh264::encoder::{EncodedBitStream, Encoder, EncoderConfig, QpRange, RateControlMode};
use openh264::formats::{RgbaSliceU8, YUVBuffer};
use rayon::prelude::*;
use std::io::BufWriter;
use std::path::Path;

/// Parallel-render chunk size (matches GIF: ~8 frames at once).
const PAR_CHUNK_SIZE: usize = 8;

const LABEL_COLOR: [u8; 4] = [232, 232, 232, 255];
const LABEL_FONT_SIZE: u32 = 24;
const LABEL_PAD: i64 = 12;
const BLACK_OPAQUE: [u8; 4] = [0, 0, 0, 255];

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

    // ── openh264 encoder: quality mode, bounded QP, single-threaded encode ──
    // (the render side already fans out across rayon; keep encode on one core
    //  to avoid oversubscription at these small resolutions).
    let keyframe_period = (fps as usize * 2).max(1);
    let config = EncoderConfig::new()
        .rate_control_mode(RateControlMode::Quality)
        .qp(QpRange::new(22, 30))
        .num_threads(1);
    let api = openh264::OpenH264API::from_source();
    let mut encoder = Encoder::with_api_config(api, config)
        .map_err(|e| PreviewError::render(format!("failed to init openh264 encoder: {e}")))?;

    // ── encode first frame, extract SPS/PPS for the mp4 track config ──
    let first_comp = compose_frame(first_frame, first_time, chart_end_ms, out_w, out_h);
    let first_yuv = rgba_to_yuv(&first_comp);
    let (sps, pps, first_sample) = {
        let first_bs = encoder
            .encode(&first_yuv)
            .map_err(|e| PreviewError::render(format!("openh264 encode failed: {e}")))?;
        extract_nals(&first_bs)
    };
    let sps = sps.ok_or_else(|| PreviewError::render("missing SPS in first encoded frame"))?;
    let pps = pps.ok_or_else(|| PreviewError::render("missing PPS in first encoded frame"))?;

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
        bytes: Bytes::copy_from_slice(&first_sample),
    };
    mp4_writer
        .write_sample(1, &sample)
        .map_err(|e| PreviewError::render(format!("mp4 write_sample failed: {e}")))?;

    // ── render + encode remaining frames in parallel chunks ──
    for chunk_start in (1..frame_count).step_by(PAR_CHUNK_SIZE) {
        let chunk_end = (chunk_start + PAR_CHUNK_SIZE).min(frame_count);
        let frames: Vec<(Img, i64)> = (chunk_start..chunk_end)
            .into_par_iter()
            .map(&render)
            .collect();

        for (i, (frame, time)) in (chunk_start..).zip(frames) {
            if i % keyframe_period == 0 {
                encoder.force_intra_frame();
            }
            let comp = compose_frame(frame, time, chart_end_ms, out_w, out_h);
            let yuv = rgba_to_yuv(&comp);
            let bs = encoder
                .encode(&yuv)
                .map_err(|e| PreviewError::render(format!("openh264 encode failed: {e}")))?;
            let (_, _, sample_bytes) = extract_nals(&bs);
            let sample = mp4::Mp4Sample {
                start_time: i as u64,
                duration: 1,
                rendering_offset: 0,
                is_sync: i % keyframe_period == 0,
                bytes: Bytes::copy_from_slice(&sample_bytes),
            };
            mp4_writer
                .write_sample(1, &sample)
                .map_err(|e| PreviewError::render(format!("mp4 write_sample failed: {e}")))?;
        }
    }

    mp4_writer
        .write_end()
        .map_err(|e| PreviewError::render(format!("mp4 write_end failed: {e}")))?;
    Ok(())
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

/// Convert an RGBA8 image to a YUV420 buffer for openh264.
fn rgba_to_yuv(img: &Img) -> YUVBuffer {
    let rgba = RgbaSliceU8::new(&img.data, (img.w as usize, img.h as usize));
    YUVBuffer::from_rgb_source(rgba)
}

/// NAL unit type is the lower 5 bits of the first byte (start code already
/// stripped by `nal_units`).
#[inline]
fn nal_type(nal: &[u8]) -> u8 {
    if nal.is_empty() {
        0
    } else {
        nal[0] & 0x1F
    }
}

/// Strip the Annex-B start-code prefix (`00 00 00 01` or `00 00 01`) that
/// openh264 prepends to every NAL returned by `Layer::nal_unit`.
fn nal_payload(nal: &[u8]) -> &[u8] {
    if nal.len() >= 4 && nal[0..4] == [0, 0, 0, 1] {
        &nal[4..]
    } else if nal.len() >= 3 && nal[0..3] == [0, 0, 1] {
        &nal[3..]
    } else {
        nal
    }
}

/// Extract SPS (type 7), PPS (type 8), and length-prefixed slice NALs from an
/// encoded bitstream. openh264 concatenates NALs without start-code delimiters,
/// so we walk the bitstream's layers/NALs directly rather than scanning bytes.
fn extract_nals(bs: &EncodedBitStream) -> (Option<Vec<u8>>, Option<Vec<u8>>, Vec<u8>) {
    let mut sps = None;
    let mut pps = None;
    let mut slice = Vec::new();
    for l in 0..bs.num_layers() {
        let Some(layer) = bs.layer(l) else { continue };
        for n in 0..layer.nal_count() {
            let Some(nal) = layer.nal_unit(n) else { continue };
            let payload = nal_payload(nal);
            match nal_type(payload) {
                7 => sps = Some(payload.to_vec()),
                8 => pps = Some(payload.to_vec()),
                _ => {
                    slice.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                    slice.extend_from_slice(payload);
                }
            }
        }
    }
    (sps, pps, slice)
}

fn format_mmss(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}
