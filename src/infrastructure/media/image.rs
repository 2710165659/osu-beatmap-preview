//! 输出编码器：优化的 PNG 与 GIF（全局调色板 + 差分帧）。
//! GIF 写入器通过回调流式处理帧，因此完整动画不会同时驻留内存。
//! 帧由 rayon 分块并行渲染，再按顺序编码以保持差分帧顺序。

use crate::domain::errors::{PreviewError, Result};
use crate::domain::timeout::RequestDeadline;
use crate::render::canvas::Img;
use rayon::prelude::*;
use std::path::Path;

pub fn save_png(image: &Img, path: &Path, deadline: &RequestDeadline) -> Result<()> {
    deadline.check()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| PreviewError::render(format!("failed to create output dir: {e}")))?;
    }

    // NeuQuant 调色板每 16 个像素采样 1 个。PNG（尤其是 mania 网格）
    // 主要由少于 256 种颜色的纯色区域组成，激进采样几乎不影响调色板，
    // 却能将样本缓冲区相较原先四分之一采样再缩小 4 倍，并按比例加快训练。
    let mut sample = Vec::with_capacity(((image.w * image.h / 16 + 1) * 4) as usize);
    for px in image.data.chunks_exact(64) {
        sample.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    deadline.check()?;
    // 覆盖余数像素，确保很小的图像也至少采到一个样本。
    let rem_start = (image.data.len() / 64) * 64;
    if rem_start < image.data.len() && sample.is_empty() {
        sample.extend_from_slice(&[
            image.data[rem_start],
            image.data[rem_start + 1],
            image.data[rem_start + 2],
            255,
        ]);
    }

    // 使用 NeuQuant 构建 256 色调色板（与 GIF 使用相同量化器）。
    let nq = color_quant::NeuQuant::new(10, 255, &sample);
    deadline.check()?;
    let palette_rgba = nq.color_map_rgba();
    let mut palette_rgb = Vec::with_capacity(256 * 3);
    for px in palette_rgba.chunks_exact(4) {
        palette_rgb.extend_from_slice(&px[..3]);
    }
    while palette_rgb.len() < 256 * 3 {
        palette_rgb.extend_from_slice(&[0, 0, 0]);
    }

    // 通过 32³ 查找表将每个 RGBA 像素映射到最近的调色板索引。
    // PNG 不做海报化，但每个通道量化为 32 级（>>3）最多产生 ±4 LSB 误差，
    // 远小于 NeuQuant 自身误差，因此实际索引与逐像素 index_of() 相同。
    // 查找表替代原先的 HashMap：只需一次数组访问，无哈希开销。
    let lut = build_png_lut(&nq);
    deadline.check()?;
    let mut indexed = vec![0u8; (image.w * image.h) as usize];
    for (i, px) in image.data.chunks_exact(4).enumerate() {
        indexed[i] = lut[px[0] as usize >> 3][px[1] as usize >> 3][px[2] as usize >> 3];
        if i % 1_000_000 == 0 {
            deadline.check()?;
        }
    }

    // 写入同目录临时文件，PNG 完整编码后再原子替换最终路径，
    // 确保中断渲染不会留下可被缓存误用的残缺文件。
    crate::infrastructure::cache::with_atomic_output_deadline(
        path,
        "png.tmp",
        deadline,
        |tmp_path| {
            let file = std::fs::File::create(tmp_path)
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
            drop(writer); // flush buffered bytes before the temp file is renamed
            deadline.check()?;
            Ok(())
        },
    )
}

/// 将通道海报化为 5 位（32 级），复制高位以保持完整 0..255 范围。
/// 这能稳定帧间抗锯齿/渐变像素，缩小差分区域并延长 LZW 连续段。
#[inline]
fn posterize(v: u8) -> u8 {
    (v & 0xF0) | (v >> 4)
}

/// 预计算将海报化 RGB 映射到调色板索引的 32³ 查找表。
///
/// posterize() 每个通道产生 16 个不同值（0x00、0x11、…、0xFF）；
/// `>> 3` 将其无冲突地映射到 32 个槽位中的 16 个，因此完整颜色空间
/// 可以放入 `32*32*32 = 32768` 项数组。每项存储对应颜色的 NeuQuant 最近索引，
/// 并将 `transparent_idx` 映射到前一个调色板项，避免作为普通像素索引输出。
///
/// 每个槽位由 `posterize(ri << 3)` 构建，正好对应查找时的颜色
/// （`posterize(px) >> 3` 会映射到同一槽位）。因此每次查找都能命中精确颜色的
/// `index_of()` 结果，与旧的逐像素 HashMap 路径一致且不会产生量化漂移。
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

/// 为 PNG 预计算将 `>>3` 分桶 RGB 映射到调色板索引的 32³ 查找表。
///
/// 与 `build_gif_lut` 不同，这里没有需要重映射的透明索引。每个通道量化为 32 级
/// （`>>3`），覆盖完整 0..255 范围，±4 LSB 误差远低于 NeuQuant 的量化步长，
/// 因此查找表会得到逐像素 `index_of()` 的相同结果。
fn build_png_lut(nq: &color_quant::NeuQuant) -> [[[u8; 32]; 32]; 32] {
    let mut lut = [[[0u8; 32]; 32]; 32];
    for ri in 0..32u8 {
        let r = ri << 3;
        for gi in 0..32u8 {
            let g = gi << 3;
            for bi in 0..32u8 {
                let b = bi << 3;
                lut[ri as usize][gi as usize][bi as usize] = nq.index_of(&[r, g, b, 255]) as u8;
            }
        }
    }
    lut
}

/// GIF 并行渲染分块大小：在内存（约 8 帧 × 2 MB）和并行度之间取得平衡，
/// 并防止异常大的画布使峰值内存扩大八倍。
/// 将 `render(i)` 生成的 `frame_count` 帧流式写入循环播放的 GIF。
///
/// 帧先由 rayon 分块并行渲染，再顺序编码以保持差分帧顺序。
/// `render` 必须是 `Fn`（而不是 `FnMut`）以便跨线程共享；需要内部可变性的
/// 模式缓存应使用 `Mutex<RenderCache>`。
///
/// 尺寸与内存策略：
/// - 从少量采样帧构建全局 127 色调色板（NeuQuant），索引 127 保留给帧间透明度和 7 位 LZW 编码；
/// - 每帧只编码相对上一帧的差分矩形，未改变像素设为透明；
/// - 对异常大的画布动态缩小渲染分块。
pub fn save_animated_gif_streamed(
    frame_count: usize,
    render: impl Fn(usize) -> Img + Send + Sync,
    path: &Path,
    frame_duration_ms: u32,
    deadline: &RequestDeadline,
) -> Result<()> {
    deadline.check()?;
    if frame_count == 0 {
        return Err(PreviewError::render("no frames to encode"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| PreviewError::render(format!("failed to create output dir: {e}")))?;
    }

    // ── 调色板阶段：最多采样 4 帧 ──
    let mut sample_indices: Vec<usize> = if frame_count <= 4 {
        (0..frame_count).collect()
    } else {
        vec![0, frame_count / 3, frame_count * 2 / 3, frame_count - 1]
    };
    sample_indices.dedup();

    // 调色板采样帧彼此独立，因此并行渲染。该阶段最多保留四帧，
    // 不超过旧版固定分块的内存占用。
    let palette_frames: Vec<Img> = sample_indices.par_iter().map(|&si| render(si)).collect();
    deadline.check()?;

    let mut sample: Vec<u8> = Vec::new();
    let mut first_dims = (0u32, 0u32);
    for frame in palette_frames {
        if first_dims == (0, 0) {
            first_dims = (frame.w, frame.h);
        }
        // 每 4 个像素采样 1 个，以限制量化器开销。
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
    // 为 GIF 差分帧透明度保留一个索引。使用 127 个实际颜色可使最大输出索引为 127，
    // LZW 初始码只需 7 位，不会因索引 255 被迫使用 8 位。
    let nq = color_quant::NeuQuant::new(
        10,
        crate::infrastructure::config::current()
            .advance
            .gif
            .PALETTE_COLORS,
        &sample,
    );
    deadline.check()?;
    let mut palette: Vec<u8> = Vec::with_capacity(
        (crate::infrastructure::config::current()
            .advance
            .gif
            .PALETTE_COLORS
            + 1)
            * 3,
    );
    for px in nq.color_map_rgba().chunks_exact(4) {
        palette.extend_from_slice(&px[..3]);
    }
    while palette.len()
        < (crate::infrastructure::config::current()
            .advance
            .gif
            .PALETTE_COLORS
            + 1)
            * 3
    {
        palette.extend_from_slice(&[0, 0, 0]);
    }
    let transparent_idx: u8 = crate::infrastructure::config::current()
        .advance
        .gif
        .PALETTE_COLORS as u8;

    // 预计算将海报化 RGB 映射到调色板索引的 32³ 三维查找表。
    // 每个通道会缩减为 16 个值，>>3 后无冲突地落入 32 个槽位中的 16 个，
    // 因此数组覆盖全部颜色空间，以一次数组访问替代逐像素 HashMap 和神经网络查找。
    // 构建成本为一次 32768 × index_of()，单像素查找为 O(1)。
    let lut = build_gif_lut(&nq, transparent_idx);

    let (w, h) = (first_dims.0 as usize, first_dims.1 as usize);

    // 写入同目录临时文件，所有帧编码完成后才原子替换最终路径，
    // 确保中断渲染不会留下可被缓存误用的残缺文件。
    crate::infrastructure::cache::with_atomic_output_deadline(
        path,
        "gif.tmp",
        deadline,
        |tmp_path| {
            let file = std::fs::File::create(tmp_path)
                .map_err(|e| PreviewError::render(format!("failed to write gif: {e}")))?;
            let writer = std::io::BufWriter::new(file);
            let mut encoder = gif::Encoder::new(writer, w as u16, h as u16, &palette)
                .map_err(|e| PreviewError::render(format!("failed to write gif: {e}")))?;
            encoder
                .set_repeat(gif::Repeat::Infinite)
                .map_err(|e| PreviewError::render(format!("failed to write gif: {e}")))?;

            let delay = (frame_duration_ms / 10) as u16; // GIF delay unit = 10ms

            let pixel_count = w.saturating_mul(h);
            // 两块缓冲区交替保存当前帧和上一帧，避免每帧重新分配并清零整张 indexed 图像。
            let mut prev_indexed: Vec<u8> = Vec::with_capacity(pixel_count);
            let mut indexed: Vec<u8> = Vec::with_capacity(pixel_count);
            // 差分矩形在 LZW 压缩前只需短暂存在。编码器会把借用的数据替换为独立的
            // 压缩缓冲区，因此可跨帧复用这块原始索引缓冲，避免反复分配大矩形。
            let mut delta_buffer: Vec<u8> = Vec::with_capacity(pixel_count);
            let frame_bytes = w.saturating_mul(h).saturating_mul(4).max(1);
            let par_chunk_size = (crate::infrastructure::config::current()
                .advance
                .gif
                .MAX_PAR_FRAME_BYTES
                / frame_bytes)
                .clamp(
                    1,
                    crate::infrastructure::config::current()
                        .advance
                        .gif
                        .PAR_CHUNK_SIZE,
                );

            // ── 分块并行渲染与编码 ──
            for chunk_start in (0..frame_count).step_by(par_chunk_size) {
                deadline.check()?;
                let chunk_end = (chunk_start + par_chunk_size).min(frame_count);

                // 并行渲染当前分块；每个线程独立调用 `render(i)`。
                let frames: Vec<Img> = (chunk_start..chunk_end)
                    .into_par_iter()
                    .map(&render)
                    .collect();
                deadline.check()?;

                // 顺序编码（差分帧必须保持顺序）。
                for (fi, frame) in (chunk_start..).zip(frames) {
                    deadline.check()?;
                    rgba_to_indexed(&frame, &lut, &mut indexed, pixel_count);
                    drop(frame);

                    let (rect, buffer, transparent) = if fi == 0 {
                        // make_lzw_pre_encoded 会立即把借用的原始 buffer 替换为压缩数据，
                        // 因此首帧可以直接借用 indexed，省去一次整帧复制。
                        (
                            (0usize, 0usize, w, h),
                            std::borrow::Cow::Borrowed(indexed.as_slice()),
                            None,
                        )
                    } else {
                        match find_delta_rect(&indexed, &prev_indexed, w, h) {
                            None => (
                                (0, 0, 1, 1),
                                {
                                    delta_buffer.clear();
                                    delta_buffer.push(transparent_idx);
                                    std::borrow::Cow::Borrowed(delta_buffer.as_slice())
                                },
                                Some(transparent_idx),
                            ),
                            Some((min_x, min_y, max_x, max_y)) => {
                                let rw = max_x - min_x + 1;
                                let rh = max_y - min_y + 1;
                                delta_buffer.clear();
                                delta_buffer.reserve(rw.saturating_mul(rh));
                                for y in min_y..=max_y {
                                    let row = y * w;
                                    for x in min_x..=max_x {
                                        let v = indexed[row + x];
                                        delta_buffer.push(if v == prev_indexed[row + x] {
                                            transparent_idx
                                        } else {
                                            v
                                        });
                                    }
                                }
                                (
                                    (min_x, min_y, rw, rh),
                                    std::borrow::Cow::Borrowed(delta_buffer.as_slice()),
                                    Some(transparent_idx),
                                )
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
                        buffer,
                    };
                    gframe.make_lzw_pre_encoded();
                    encoder
                        .write_lzw_pre_encoded_frame(&gframe)
                        .map_err(|e| PreviewError::render(format!("failed to write gif: {e}")))?;
                    // gframe 在此之后已不再借用 indexed，安全地交换两块帧缓冲区。
                    std::mem::swap(&mut indexed, &mut prev_indexed);
                }
            }
            drop(encoder); // flush buffered bytes before the temp file is renamed
            deadline.check()?;
            Ok(())
        },
    )
}

/// 将 RGBA 帧映射为 GIF 使用的 indexed 像素。
///
/// 帧尺寸由渲染器保证一致；若输入数据不足，保留旧实现的行为，用 0 填充尾部。
fn rgba_to_indexed(
    frame: &Img,
    lut: &[[[u8; 32]; 32]; 32],
    indexed: &mut Vec<u8>,
    pixel_count: usize,
) {
    indexed.clear();
    // 保持旧路径对异常短输入的行为：未覆盖的尾部仍为索引 0。
    // 容量已在动画开始时预分配，resize 不会在正常帧尺寸下重新分配。
    indexed.resize(pixel_count, 0);
    for (i, px) in frame.data.chunks_exact(4).enumerate().take(pixel_count) {
        indexed[i] = lut[posterize(px[0]) as usize >> 3][posterize(px[1]) as usize >> 3]
            [posterize(px[2]) as usize >> 3];
    }
}

/// 查找 `cur` 与 `prev` 之间不同字节的包围盒。
///
/// 返回包含端点的 `Some((min_x, min_y, max_x, max_y))`；两个缓冲区相同时返回 `None`。
/// x86_64 使用 SSE2 每次比较 16 字节（`_mm_cmpeq_epi8` + `_mm_movemask_epi8`），
/// 以 O(w/16) 找到每行首尾差异，而不是 O(w)。SSE2 是所有 x86_64 CPU 的基线，
/// 无需运行时检测；其它架构回退到逐字节扫描。
#[cfg(target_arch = "x86_64")]
fn find_delta_rect(
    cur: &[u8],
    prev: &[u8],
    w: usize,
    h: usize,
) -> Option<(usize, usize, usize, usize)> {
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
                // 掩码位为 1 表示字节相等；取反后即可找到差异。
                let diff = (!mask) & 0xFFFF;
                if diff != 0 {
                    let diff16 = diff as u16;
                    let first = c * 16 + diff16.trailing_zeros() as usize;
                    let last = c * 16 + 15 - diff16.leading_zeros() as usize;
                    if first < min_x {
                        min_x = first;
                    }
                    if last > max_x {
                        max_x = last;
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
        // 处理尾部字节（当 w 不是 16 的倍数时）。
        if rem != 0 {
            let off = row + chunks * 16;
            for x in 0..rem {
                if cur[off + x] != prev[off + x] {
                    let gx = chunks * 16 + x;
                    if gx < min_x {
                        min_x = gx;
                    }
                    if gx > max_x {
                        max_x = gx;
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
    }
    if min_x > max_x {
        None
    } else {
        Some((min_x, min_y, max_x, max_y))
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn find_delta_rect(
    cur: &[u8],
    prev: &[u8],
    w: usize,
    h: usize,
) -> Option<(usize, usize, usize, usize)> {
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            if cur[row + x] != prev[row + x] {
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
        None
    } else {
        Some((min_x, min_y, max_x, max_y))
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    fn expired(format: &str) -> RequestDeadline {
        RequestDeadline::new(
            Instant::now() - Duration::from_secs(2),
            format,
            Duration::from_secs(1),
        )
    }

    #[test]
    fn png_timeout_does_not_replace_existing_output() {
        let path = std::env::temp_dir().join(format!(
            "osu-preview-png-timeout-test-{}.png",
            std::process::id()
        ));
        std::fs::write(&path, b"existing").unwrap();
        let image = Img::new(1, 1, [0, 0, 0, 255]);
        let error = save_png(&image, &path, &expired("png")).unwrap_err();
        assert!(error.to_string().contains("PNG preview request timed out"));
        assert_eq!(std::fs::read(&path).unwrap(), b"existing");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn gif_timeout_stops_before_rendering_or_writing() {
        let path = std::env::temp_dir().join(format!(
            "osu-preview-gif-timeout-test-{}.gif",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let rendered = AtomicBool::new(false);
        let error = save_animated_gif_streamed(
            1,
            |_| {
                rendered.store(true, Ordering::Relaxed);
                Img::new(1, 1, [0, 0, 0, 255])
            },
            &path,
            100,
            &expired("gif"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("GIF preview request timed out"));
        assert!(!rendered.load(Ordering::Relaxed));
        assert!(!path.exists());
    }
}
