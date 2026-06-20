//! Output encoders: optimized PNG and GIF (global palette + delta frames).
//! The GIF writer streams frames from a callback so the full animation never
//! resides in memory at once.  Frames are rendered in parallel chunks (rayon)
//! and encoded sequentially to preserve delta-frame ordering.

use crate::canvas::Img;
use crate::errors::{PreviewError, Result};
use rayon::prelude::*;
use std::path::Path;

pub fn save_png(image: &Img, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| PreviewError::new(format!("failed to create output dir: {e}")))?;
    }

    // Subsample 1 of every 4 pixels for the NeuQuant palette (same strategy as
    // GIF), bounding both memory and quantizer cost.  For large images (mania
    // grids can exceed 60M pixels) this cuts the sample buffer from ~240 MB to
    // ~60 MB with negligible perceptual difference.
    let mut sample = Vec::with_capacity(((image.w * image.h / 4 + 1) * 4) as usize);
    for px in image.data.chunks_exact(16) {
        sample.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    // Catch any remainder pixels so very small images still get a sample.
    let rem_start = (image.data.len() / 16) * 16;
    if rem_start < image.data.len() && sample.is_empty() {
        sample.extend_from_slice(&[
            image.data[rem_start],
            image.data[rem_start + 1],
            image.data[rem_start + 2],
            255,
        ]);
    }

    // Build 256-color palette with NeuQuant (same quantizer as GIF).
    let nq = color_quant::NeuQuant::new(10, 255, &sample);
    let palette_rgba = nq.color_map_rgba();
    let mut palette_rgb = Vec::with_capacity(256 * 3);
    for px in palette_rgba.chunks_exact(4) {
        palette_rgb.extend_from_slice(&px[..3]);
    }
    while palette_rgb.len() < 256 * 3 {
        palette_rgb.extend_from_slice(&[0, 0, 0]);
    }

    // Map every RGBA pixel to the nearest palette index.
    let mut indexed = vec![0u8; (image.w * image.h) as usize];
    for (i, px) in image.data.chunks_exact(4).enumerate() {
        indexed[i] = nq.index_of(&[px[0], px[1], px[2], 255]) as u8;
    }

    let file = std::fs::File::create(path)
        .map_err(|e| PreviewError::new(format!("failed to write png: {e}")))?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, image.w, image.h);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(&palette_rgb);
    encoder.set_compression(png::Compression::Best);
    encoder.set_filter(png::FilterType::Paeth);
    let mut writer = encoder
        .write_header()
        .map_err(|e| PreviewError::new(format!("failed to write png: {e}")))?;
    writer
        .write_image_data(&indexed)
        .map_err(|e| PreviewError::new(format!("failed to write png: {e}")))?;
    Ok(())
}

/// Posterize a channel to 5 bits (32 levels), replicating high bits so the
/// full 0..255 range is preserved. Stabilizes AA/gradient pixels across
/// frames → smaller delta regions and longer LZW runs.
#[inline]
fn posterize(v: u8) -> u8 {
    (v & 0xF0) | (v >> 4)
}

/// GIF parallel-render chunk size: balance memory (~8 frames × ~2 MB)
/// against parallelism (keep all cores busy).
const PAR_CHUNK_SIZE: usize = 8;

/// Stream `frame_count` frames produced by `render(i)` into a looping GIF.
///
/// Frames are rendered in parallel chunks (rayon) then encoded sequentially
/// so delta-frame ordering is preserved.  `render` must be `Fn` (not `FnMut`)
/// so it can be shared across threads; use `Mutex<RenderCache>` for any
/// per-mode caches that need interior mutability.
///
/// Strategy for size + memory:
/// - global 255-color palette built from a few sampled frames (NeuQuant),
///   index 255 reserved for inter-frame transparency
/// - per-frame delta rect vs previous frame, unchanged pixels transparent
/// - at most PAR_CHUNK_SIZE raw frames + two indexed frames held at once
pub fn save_animated_gif_streamed(
    frame_count: usize,
    render: impl Fn(usize) -> Img + Send + Sync,
    path: &Path,
    frame_duration_ms: u32,
) -> Result<()> {
    if frame_count == 0 {
        return Err(PreviewError::new("no frames to encode"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| PreviewError::new(format!("failed to create output dir: {e}")))?;
    }

    // ── palette pass: sample up to 4 frames (sequential — cheap) ──
    let mut sample_indices: Vec<usize> = if frame_count <= 4 {
        (0..frame_count).collect()
    } else {
        vec![0, frame_count / 3, frame_count * 2 / 3, frame_count - 1]
    };
    sample_indices.dedup();

    let mut sample: Vec<u8> = Vec::new();
    let mut first_dims = (0u32, 0u32);
    for &si in &sample_indices {
        let frame = render(si);
        if first_dims == (0, 0) {
            first_dims = (frame.w, frame.h);
        }
        // subsample 1 of 4 pixels to bound quantizer cost
        for px in frame.data.chunks_exact(16) {
            sample.extend_from_slice(&[posterize(px[0]), posterize(px[1]), posterize(px[2]), 255]);
        }
        if sample.len() > 4 * 1_500_000 {
            break;
        }
    }
    if sample.is_empty() {
        let frame = render(0);
        first_dims = (frame.w, frame.h);
        for px in frame.data.chunks_exact(4) {
            sample.extend_from_slice(&[posterize(px[0]), posterize(px[1]), posterize(px[2]), 255]);
        }
    }
    let nq = color_quant::NeuQuant::new(10, 255, &sample);
    let mut palette: Vec<u8> = Vec::with_capacity(256 * 3);
    for px in nq.color_map_rgba().chunks_exact(4) {
        palette.extend_from_slice(&px[..3]);
    }
    while palette.len() < 256 * 3 {
        palette.extend_from_slice(&[0, 0, 0]);
    }
    let transparent_idx: u8 = 255;

    let (w, h) = (first_dims.0 as usize, first_dims.1 as usize);

    let file = std::fs::File::create(path)
        .map_err(|e| PreviewError::new(format!("failed to write gif: {e}")))?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = gif::Encoder::new(writer, w as u16, h as u16, &palette)
        .map_err(|e| PreviewError::new(format!("failed to write gif: {e}")))?;
    encoder
        .set_repeat(gif::Repeat::Infinite)
        .map_err(|e| PreviewError::new(format!("failed to write gif: {e}")))?;

    let delay = (frame_duration_ms / 10) as u16; // GIF delay unit = 10ms

    let mut lookup_cache: std::collections::HashMap<u32, u8> = std::collections::HashMap::new();
    let mut prev_indexed: Vec<u8> = Vec::new();

    // ── render + encode in parallel chunks ──
    for chunk_start in (0..frame_count).step_by(PAR_CHUNK_SIZE) {
        let chunk_end = (chunk_start + PAR_CHUNK_SIZE).min(frame_count);

        // Render this chunk in parallel; each thread calls `render(i)` independently.
        let frames: Vec<Img> = (chunk_start..chunk_end)
            .into_par_iter()
            .map(&render)
            .collect();

        // Encode sequentially (delta frames must stay in order).
        for (fi, frame) in (chunk_start..).zip(frames) {
            let mut indexed = vec![0u8; w * h];
            for (i, px) in frame.data.chunks_exact(4).enumerate().take(w * h) {
                let (r, g, b) = (posterize(px[0]), posterize(px[1]), posterize(px[2]));
                let key = (r as u32) << 16 | (g as u32) << 8 | b as u32;
                let idx = *lookup_cache.entry(key).or_insert_with(|| {
                    let idx = nq.index_of(&[r, g, b, 255]) as u8;
                    if idx == transparent_idx {
                        254
                    } else {
                        idx
                    }
                });
                indexed[i] = idx;
            }
            drop(frame);

            let (rect, buffer, transparent) = if fi == 0 {
                ((0usize, 0usize, w, h), indexed.clone(), None)
            } else {
                let prev = &prev_indexed;
                let mut min_x = w;
                let mut min_y = h;
                let mut max_x = 0usize;
                let mut max_y = 0usize;
                for y in 0..h {
                    let row = y * w;
                    for x in 0..w {
                        if indexed[row + x] != prev[row + x] {
                            if x < min_x {
                                min_x = x;
                            }
                            if x > max_x {
                                max_x = x;
                            }
                            if y < min_y {
                                min_y = y;
                            }
                            if y > max_y {
                                max_y = y;
                            }
                        }
                    }
                }
                if min_x > max_x {
                    ((0, 0, 1, 1), vec![transparent_idx], Some(transparent_idx))
                } else {
                    let rw = max_x - min_x + 1;
                    let rh = max_y - min_y + 1;
                    let mut buf = Vec::with_capacity(rw * rh);
                    for y in min_y..=max_y {
                        let row = y * w;
                        for x in min_x..=max_x {
                            let v = indexed[row + x];
                            buf.push(if v == prev[row + x] {
                                transparent_idx
                            } else {
                                v
                            });
                        }
                    }
                    ((min_x, min_y, rw, rh), buf, Some(transparent_idx))
                }
            };

            let mut gframe = gif::Frame::<'_> {
                width: rect.2 as u16,
                height: rect.3 as u16,
                left: rect.0 as u16,
                top: rect.1 as u16,
                delay,
                dispose: gif::DisposalMethod::Keep,
                transparent,
                needs_user_input: false,
                interlaced: false,
                palette: None,
                buffer: std::borrow::Cow::Owned(buffer),
            };
            gframe.make_lzw_pre_encoded();
            encoder
                .write_lzw_pre_encoded_frame(&gframe)
                .map_err(|e| PreviewError::new(format!("failed to write gif: {e}")))?;
            prev_indexed = indexed;
        }
    }
    Ok(())
}
