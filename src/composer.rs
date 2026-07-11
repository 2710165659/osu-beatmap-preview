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
            .map_err(|e| PreviewError::render(format!("failed to create output dir: {e}")))?;
    }

    // Subsample 1 of every 16 pixels for the NeuQuant palette.  PNG images
    // (especially mania grids) are dominated by flat color fills with < 256
    // distinct colors, so aggressive subsampling barely changes the palette
    // while cutting the sample buffer 4× vs the previous 1/4 strategy (from
    // ~60 MB to ~15 MB for a 60M-pixel mania grid) and speeding NeuQuant
    // training proportionally.
    let mut sample = Vec::with_capacity(((image.w * image.h / 16 + 1) * 4) as usize);
    for px in image.data.chunks_exact(64) {
        sample.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    // Catch any remainder pixels so very small images still get a sample.
    let rem_start = (image.data.len() / 64) * 64;
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

    // Map every RGBA pixel to the nearest palette index via a 32³ LUT.
    // PNG does not posterize, but quantizing each channel to 32 levels (>>3)
    // introduces at most ±4 LSB error — far below NeuQuant's own quantization
    // error — so the LUT gives identical indices to a per-pixel index_of() call
    // in practice.  This replaces the previous HashMap approach with a single
    // array access: 32K entries built once, O(1) lookup per pixel, no hashing.
    let lut = build_png_lut(&nq);
    let mut indexed = vec![0u8; (image.w * image.h) as usize];
    for (i, px) in image.data.chunks_exact(4).enumerate() {
        indexed[i] = lut[px[0] as usize >> 3]
            [px[1] as usize >> 3]
            [px[2] as usize >> 3];
    }

    let file = std::fs::File::create(path)
        .map_err(|e| PreviewError::render(format!("failed to write png: {e}")))?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, image.w, image.h);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(&palette_rgb);
    encoder.set_compression(png::Compression::Default);
    encoder.set_filter(png::FilterType::Paeth);
    let mut writer = encoder
        .write_header()
        .map_err(|e| PreviewError::render(format!("failed to write png: {e}")))?;
    writer
        .write_image_data(&indexed)
        .map_err(|e| PreviewError::render(format!("failed to write png: {e}")))?;
    Ok(())
}

/// Posterize a channel to 5 bits (32 levels), replicating high bits so the
/// full 0..255 range is preserved. Stabilizes AA/gradient pixels across
/// frames → smaller delta regions and longer LZW runs.
#[inline]
fn posterize(v: u8) -> u8 {
    (v & 0xF0) | (v >> 4)
}

/// Precompute a 32³ lookup table mapping posterized RGB → palette index.
///
/// posterize() yields 16 distinct values per channel (0x00, 0x11, …, 0xFF);
/// `>> 3` buckets them into 16 of the 32 slots with no collisions, so the
/// full posterized color space fits in a `32*32*32 = 32768`-entry array.
/// Each entry is the NeuQuant nearest-palette index for the corresponding
/// posterized color, with `transparent_idx` remapped to the preceding palette
/// entry so it is never emitted as a regular pixel index.
///
/// Each slot is built from `posterize(ri << 3)`, which is exactly the
/// posterized color that lookups query (`posterize(px) >> 3` maps to the same
/// slot).  So every lookup hits the `index_of()` result for its precise
/// posterized color — identical to the old per-pixel HashMap path, with no
/// quantization drift.  (The other 16 slots per axis are never queried.)
fn build_gif_lut(nq: &color_quant::NeuQuant, transparent_idx: u8) -> [[[u8; 32]; 32]; 32] {
    let mut lut = [[[0u8; 32]; 32]; 32];
    for ri in 0..32u8 {
        let r = posterize(ri << 3);
        for gi in 0..32u8 {
            let g = posterize(gi << 3);
            for bi in 0..32u8 {
                let b = posterize(bi << 3);
                let idx = nq.index_of(&[r, g, b, 255]) as u8;
                lut[ri as usize][gi as usize][bi as usize] = if idx == transparent_idx {
                    transparent_idx.saturating_sub(1)
                } else {
                    idx
                };
            }
        }
    }
    lut
}

/// Precompute a 32³ LUT mapping `>>3`-bucketed RGB → palette index for PNG.
///
/// Unlike `build_gif_lut` there is no transparent index to remap.  Each channel
/// is quantized to 32 levels (`>>3`), covering the full 0..255 range with ±4
/// LSB error — well below NeuQuant's own quantization step, so the LUT yields
/// the same index a per-pixel `index_of()` would return.
fn build_png_lut(nq: &color_quant::NeuQuant) -> [[[u8; 32]; 32]; 32] {
    let mut lut = [[[0u8; 32]; 32]; 32];
    for ri in 0..32u8 {
        let r = ri << 3;
        for gi in 0..32u8 {
            let g = gi << 3;
            for bi in 0..32u8 {
                let b = bi << 3;
                lut[ri as usize][gi as usize][bi as usize] =
                    nq.index_of(&[r, g, b, 255]) as u8;
            }
        }
    }
    lut
}

/// GIF parallel-render chunk size: balance memory (~8 frames × ~2 MB)
/// against parallelism (keep all cores busy).
const PAR_CHUNK_SIZE: usize = 8;
/// Prevent unusually large canvases from multiplying peak memory by eight.
const MAX_PAR_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Stream `frame_count` frames produced by `render(i)` into a looping GIF.
///
/// Frames are rendered in parallel chunks (rayon) then encoded sequentially
/// so delta-frame ordering is preserved.  `render` must be `Fn` (not `FnMut`)
/// so it can be shared across threads; use `Mutex<RenderCache>` for any
/// per-mode caches that need interior mutability.
///
/// Strategy for size + memory:
/// - global 127-color palette built from a few sampled frames (NeuQuant),
///   index 127 reserved for inter-frame transparency and 7-bit LZW codes
/// - per-frame delta rect vs previous frame, unchanged pixels transparent
/// - render chunk shrinks dynamically for unusually large canvases
pub fn save_animated_gif_streamed(
    frame_count: usize,
    render: impl Fn(usize) -> Img + Send + Sync,
    path: &Path,
    frame_duration_ms: u32,
) -> Result<()> {
    if frame_count == 0 {
        return Err(PreviewError::render("no frames to encode"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| PreviewError::render(format!("failed to create output dir: {e}")))?;
    }

    // ── palette pass: sample up to 4 frames ──
    let mut sample_indices: Vec<usize> = if frame_count <= 4 {
        (0..frame_count).collect()
    } else {
        vec![0, frame_count / 3, frame_count * 2 / 3, frame_count - 1]
    };
    sample_indices.dedup();

    // Palette frames are independent, so render them concurrently. This pass
    // holds at most four frames, no more than the old fixed render chunk.
    let palette_frames: Vec<Img> = sample_indices
        .par_iter()
        .map(|&si| render(si))
        .collect();

    let mut sample: Vec<u8> = Vec::new();
    let mut first_dims = (0u32, 0u32);
    for frame in palette_frames {
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
    // Reserve one index for GIF delta-frame transparency. Keeping 127 actual
    // colors makes the largest emitted index 127, so LZW can use a 7-bit
    // initial code instead of being forced to 8 bits by index 255.
    const GIF_PALETTE_COLORS: usize = 127;
    let nq = color_quant::NeuQuant::new(10, GIF_PALETTE_COLORS, &sample);
    let mut palette: Vec<u8> = Vec::with_capacity((GIF_PALETTE_COLORS + 1) * 3);
    for px in nq.color_map_rgba().chunks_exact(4) {
        palette.extend_from_slice(&px[..3]);
    }
    while palette.len() < (GIF_PALETTE_COLORS + 1) * 3 {
        palette.extend_from_slice(&[0, 0, 0]);
    }
    let transparent_idx: u8 = GIF_PALETTE_COLORS as u8;

    // Precompute a 32³ 3D LUT mapping posterized RGB → palette index.
    // posterize() reduces each channel to 16 distinct values (0x00..0xFF step 0x11);
    // >>3 buckets these into 16 of 32 slots with no collisions, so a 32³ array
    // covers the whole posterized color space. This replaces the per-pixel
    // HashMap lookup + NeuQuant neural-net search with a single array access.
    //
    // Build cost: 32768 × index_of() once. Lookup cost: O(1) per pixel.
    let lut = build_gif_lut(&nq, transparent_idx);

    let (w, h) = (first_dims.0 as usize, first_dims.1 as usize);

    let file = std::fs::File::create(path)
        .map_err(|e| PreviewError::render(format!("failed to write gif: {e}")))?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = gif::Encoder::new(writer, w as u16, h as u16, &palette)
        .map_err(|e| PreviewError::render(format!("failed to write gif: {e}")))?;
    encoder
        .set_repeat(gif::Repeat::Infinite)
        .map_err(|e| PreviewError::render(format!("failed to write gif: {e}")))?;

    let delay = (frame_duration_ms / 10) as u16; // GIF delay unit = 10ms

    let mut prev_indexed: Vec<u8> = Vec::new();
    let frame_bytes = w.saturating_mul(h).saturating_mul(4).max(1);
    let par_chunk_size = (MAX_PAR_FRAME_BYTES / frame_bytes).clamp(1, PAR_CHUNK_SIZE);

    // ── render + encode in parallel chunks ──
    for chunk_start in (0..frame_count).step_by(par_chunk_size) {
        let chunk_end = (chunk_start + par_chunk_size).min(frame_count);

        // Render this chunk in parallel; each thread calls `render(i)` independently.
        let frames: Vec<Img> = (chunk_start..chunk_end)
            .into_par_iter()
            .map(&render)
            .collect();

        // Encode sequentially (delta frames must stay in order).
        for (fi, frame) in (chunk_start..).zip(frames) {
            let mut indexed = vec![0u8; w * h];
            for (i, px) in frame.data.chunks_exact(4).enumerate().take(w * h) {
                indexed[i] = lut[posterize(px[0]) as usize >> 3]
                    [posterize(px[1]) as usize >> 3]
                    [posterize(px[2]) as usize >> 3];
            }
            drop(frame);

            let (rect, buffer, transparent) = if fi == 0 {
                ((0usize, 0usize, w, h), indexed.clone(), None)
            } else {
                match find_delta_rect(&indexed, &prev_indexed, w, h) {
                    None => ((0, 0, 1, 1), vec![transparent_idx], Some(transparent_idx)),
                    Some((min_x, min_y, max_x, max_y)) => {
                        let rw = max_x - min_x + 1;
                        let rh = max_y - min_y + 1;
                        let mut buf = Vec::with_capacity(rw * rh);
                        for y in min_y..=max_y {
                            let row = y * w;
                            for x in min_x..=max_x {
                                let v = indexed[row + x];
                                buf.push(if v == prev_indexed[row + x] {
                                    transparent_idx
                                } else {
                                    v
                                });
                            }
                        }
                        ((min_x, min_y, rw, rh), buf, Some(transparent_idx))
                    }
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
                .map_err(|e| PreviewError::render(format!("failed to write gif: {e}")))?;
            prev_indexed = indexed;
        }
    }
    Ok(())
}

/// Find the bounding box of differing bytes between `cur` and `prev`.
///
/// Returns `Some((min_x, min_y, max_x, max_y))` (inclusive) or `None` if the
/// two buffers are identical.  On x86_64 this uses SSE2 to compare 16 bytes at
/// a time (`_mm_cmpeq_epi8` + `_mm_movemask_epi8`), finding the first and last
/// differing byte per row in O(w/16) instead of O(w).  SSE2 is baseline on all
/// x86_64 CPUs so no runtime detection is needed; other architectures fall
/// back to a scalar byte-by-byte scan.
#[cfg(target_arch = "x86_64")]
fn find_delta_rect(cur: &[u8], prev: &[u8], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    use std::arch::x86_64::*;
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let chunks = w / 16;
    let rem = w % 16;
    for y in 0..h {
        let row = y * w;
        unsafe {
            for c in 0..chunks {
                let off = row + c * 16;
                let a = _mm_loadu_si128(cur.as_ptr().add(off) as *const __m128i);
                let b = _mm_loadu_si128(prev.as_ptr().add(off) as *const __m128i);
                let cmp = _mm_cmpeq_epi8(a, b);
                let mask = _mm_movemask_epi8(cmp) as u32;
                // mask bit = 1 means bytes are EQUAL; invert to find diffs.
                let diff = (!mask) & 0xFFFF;
                if diff != 0 {
                    let diff16 = diff as u16;
                    let first = c * 16 + diff16.trailing_zeros() as usize;
                    let last = c * 16 + 15 - diff16.leading_zeros() as usize;
                    if first < min_x { min_x = first; }
                    if last > max_x { max_x = last; }
                    if y < min_y { min_y = y; }
                    if y > max_y { max_y = y; }
                }
            }
        }
        // Handle the trailing bytes (if w is not a multiple of 16).
        if rem != 0 {
            let off = row + chunks * 16;
            for x in 0..rem {
                if cur[off + x] != prev[off + x] {
                    let gx = chunks * 16 + x;
                    if gx < min_x { min_x = gx; }
                    if gx > max_x { max_x = gx; }
                    if y < min_y { min_y = y; }
                    if y > max_y { max_y = y; }
                }
            }
        }
    }
    if min_x > max_x {
        None
    } else {
        Some((min_x, min_y, max_x, max_y))
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn find_delta_rect(cur: &[u8], prev: &[u8], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            if cur[row + x] != prev[row + x] {
                if x < min_x { min_x = x; }
                if x > max_x { max_x = x; }
                if y < min_y { min_y = y; }
                if y > max_y { max_y = y; }
            }
        }
    }
    if min_x > max_x { None } else { Some((min_x, min_y, max_x, max_y)) }
}
