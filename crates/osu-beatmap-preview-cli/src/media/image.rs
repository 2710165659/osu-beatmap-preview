//! 输出编码器：PNG 与流式 GIF；GIF 复用调色板并按需补色，保留帧间差分。
//! GIF 写入器通过回调流式处理帧，因此完整动画不会同时驻留内存。
//! 帧由 rayon 分块并行渲染，再按顺序编码以保持差分帧顺序。

mod palette;

use crate::export::canvas::Img;
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
use osu_beatmap_preview_core::support::timeout::RequestDeadline;
use palette::{build_gif_lut, local_palette_frame, posterize, AdaptivePalette};
use rayon::prelude::*;
use std::path::Path;

pub fn save_png(image: &Img, path: &Path, deadline: &RequestDeadline) -> Result<()> {
    deadline.check()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| PreviewError::render(format!("failed to create output dir: {e}")))?;
    }

    // NeuQuant 每 16 个像素采样 1 个：PNG（尤其 mania 网格）以纯色区为主、调色板
    // 少于 256 色，激进采样几乎不影响质量，样本缓冲再缩小 4 倍并加快训练。
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

    // 通过 32³ 查找表将每个 RGBA 像素映射到最近的调色板索引：PNG 不做海报化，但每
    // 个通道量化为 32 级（>>3）最多 ±4 LSB 误差，远小于 NeuQuant 自身误差，结果与
    // 逐像素 `index_of()` 相同；一次数组访问，没有哈希开销。
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
    crate::cache::with_atomic_output_deadline(path, "png.tmp", deadline, |tmp_path| {
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
    })
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
/// 固定元素保留精确颜色，遗漏的颜色按需补入局部调色板并跨帧复用；容量耗尽时回收补色。
/// `precise_fades` 收紧连续淡出的颜色误差，必要时以局部量化换取精度。
pub fn save_animated_gif_streamed_with_colors(
    frame_count: usize,
    render: impl Fn(usize) -> Img + Send + Sync,
    path: &Path,
    frame_rate: f64,
    deadline: &RequestDeadline,
    fixed_colors: &[[u8; 3]],
    precise_fades: bool,
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
    // 背景面积远大于淡出中的音符；限制同色样本权重，避免 HD/FL 的暗色音符
    // 被 NeuQuant 忽略后映射到灰色。海报化后的颜色空间只有 32³ 项。
    let mut color_samples = [0u8; 32768];
    let mut first_dims = (0u32, 0u32);
    for frame in palette_frames {
        if first_dims == (0, 0) {
            first_dims = (frame.w, frame.h);
        }
        // 每 4 个像素采样 1 个，以限制量化器开销。
        for px in frame.data.chunks_exact(16) {
            let key = ((px[0] as usize >> 3) << 10)
                | ((px[1] as usize >> 3) << 5)
                | (px[2] as usize >> 3);
            if color_samples[key] >= 64 {
                continue;
            }
            color_samples[key] += 1;
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
    let mut exact_colors = Vec::new();
    for color in fixed_colors {
        if !exact_colors.contains(color) {
            exact_colors.push(*color);
        }
    }
    let palette_colors = crate::config::current().advance.gif.PALETTE_COLORS;
    let learned_colors = palette_colors
        .checked_sub(exact_colors.len())
        .filter(|&count| (2..=255).contains(&count) && palette_colors <= 255)
        .ok_or_else(|| PreviewError::render("GIF 调色板容量不足以保留固定颜色"))?;
    // 固定颜色占用原预算内的槽位，不增加调色板位数，也不覆盖量化器仍会引用的颜色。
    let nq = color_quant::NeuQuant::new(10, learned_colors, &sample);
    deadline.check()?;
    let mut palette: Vec<u8> =
        Vec::with_capacity((crate::config::current().advance.gif.PALETTE_COLORS + 1) * 3);
    for px in nq.color_map_rgba().chunks_exact(4) {
        palette.extend_from_slice(&px[..3]);
    }
    let exact_colors: Vec<_> = exact_colors
        .into_iter()
        .enumerate()
        .map(|(index, color)| {
            palette.extend_from_slice(&color);
            (color, (learned_colors + index) as u8)
        })
        .collect();
    while palette.len() < (crate::config::current().advance.gif.PALETTE_COLORS + 1) * 3 {
        palette.extend_from_slice(&[0, 0, 0]);
    }
    let transparent_idx: u8 = crate::config::current().advance.gif.PALETTE_COLORS as u8;

    // 预计算将海报化 RGB 映射到调色板索引的 32³ 三维查找表。
    // 每个通道会缩减为 32 个值，>>3 后无冲突地落入 32 个槽位，
    // 因此数组覆盖全部颜色空间，以一次数组访问替代逐像素 HashMap 和神经网络查找。
    // 构建成本为一次 32768 × index_of()，单像素查找为 O(1)。
    let lut = build_gif_lut(&nq, transparent_idx);

    let (w, h) = (first_dims.0 as usize, first_dims.1 as usize);

    // 写入同目录临时文件，所有帧编码完成后才原子替换最终路径，
    // 确保中断渲染不会留下可被缓存误用的残缺文件。
    crate::cache::with_atomic_output_deadline(path, "gif.tmp", deadline, |tmp_path| {
        let file = std::fs::File::create(tmp_path)
            .map_err(|e| PreviewError::render(format!("failed to write gif: {e}")))?;
        let writer = std::io::BufWriter::new(file);
        let mut encoder = gif::Encoder::new(writer, w as u16, h as u16, &palette)
            .map_err(|e| PreviewError::render(format!("failed to write gif: {e}")))?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(|e| PreviewError::render(format!("failed to write gif: {e}")))?;

        // 普通预览复用较宽的颜色桶以维持吞吐；连续淡出需更小误差，避免暗色音符转灰。
        let max_color_error = if precise_fades { 8 } else { 24 };
        let mut palette =
            AdaptivePalette::new(palette, lut, transparent_idx, exact_colors, max_color_error);
        let pixel_count = w.saturating_mul(h);
        // 两块缓冲区交替保存当前帧和上一帧，避免每帧重新分配并清零整张 indexed 图像。
        let mut prev_indexed: Vec<u8> = Vec::with_capacity(pixel_count);
        let mut indexed: Vec<u8> = Vec::with_capacity(pixel_count);
        // 差分矩形在 LZW 压缩前只需短暂存在。编码器会把借用的数据替换为独立的
        // 压缩缓冲区，因此可跨帧复用这块原始索引缓冲，避免反复分配大矩形。
        let mut delta_buffer: Vec<u8> = Vec::with_capacity(pixel_count);
        let frame_bytes = w.saturating_mul(h).saturating_mul(4).max(1);
        let par_chunk_size = (crate::config::current().advance.gif.MAX_PAR_FRAME_BYTES
            / frame_bytes)
            .clamp(1, crate::config::current().advance.gif.PAR_CHUNK_SIZE);

        // 编码当前帧时并行准备后续帧，消费者与分块共用原有帧数预算。
        encode_rendered_frames(
            frame_count,
            par_chunk_size,
            &render,
            deadline,
            |fi, frame| {
                deadline.check()?;
                let (reset, needs_local_palette) = palette.index_frame(&frame, &mut indexed);
                if reset {
                    prev_indexed.clear();
                }
                if needs_local_palette {
                    let mut local_frame = local_palette_frame(frame, fixed_colors);
                    local_frame.delay = (((fi + 1) as f64 * 100.0 / frame_rate).round()
                        - (fi as f64 * 100.0 / frame_rate).round())
                    .max(1.0) as u16;
                    local_frame.dispose = gif::DisposalMethod::Keep;
                    encoder
                        .write_frame(&local_frame)
                        .map_err(|e| PreviewError::render(format!("failed to write gif: {e}")))?;
                    // 局部调色板的索引与全局索引不同；下一张全局帧必须完整覆盖，不能沿用旧差分。
                    prev_indexed.clear();
                    return Ok(());
                }
                drop(frame);

                let (rect, buffer, transparent) = if prev_indexed.is_empty() {
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
                    // GIF 延迟以厘秒保存。对累计时间取整后作差，将不可整除的
                    // 帧周期分摊到相邻帧，任意帧前缀的误差都不超过半厘秒。
                    delay: (((fi + 1) as f64 * 100.0 / frame_rate).round()
                        - (fi as f64 * 100.0 / frame_rate).round())
                    .max(1.0) as u16,
                    dispose: gif::DisposalMethod::Keep,
                    transparent,
                    needs_user_input: false,
                    interlaced: false,
                    palette: palette.frame_palette(),
                    buffer,
                };
                gframe.make_lzw_pre_encoded();
                encoder
                    .write_lzw_pre_encoded_frame(&gframe)
                    .map_err(|e| PreviewError::render(format!("failed to write gif: {e}")))?;
                // gframe 在此之后已不再借用 indexed，安全地交换两块帧缓冲区。
                std::mem::swap(&mut indexed, &mut prev_indexed);
                Ok(())
            },
        )?;
        drop(encoder); // flush buffered bytes before the temp file is renamed
        deadline.check()?;
        Ok(())
    })
}

/// 零容量通道只交接当前帧；失败时先断开接收端，再等待生产者退出，避免阻塞或泄漏线程。
fn encode_rendered_frames(
    frame_count: usize,
    max_frames: usize,
    render: &(impl Fn(usize) -> Img + Send + Sync),
    deadline: &RequestDeadline,
    mut encode: impl FnMut(usize, Img) -> Result<()>,
) -> Result<()> {
    if max_frames == 1 {
        for index in 0..frame_count {
            deadline.check()?;
            encode(index, render(index))?;
        }
        return Ok(());
    }
    let chunk_size = max_frames - 1;
    std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(0);
        let producer = scope.spawn(move || -> Result<()> {
            for start in (0..frame_count).step_by(chunk_size) {
                deadline.check()?;
                let end = (start + chunk_size).min(frame_count);
                let frames: Vec<_> = (start..end).into_par_iter().map(render).collect();
                for item in (start..).zip(frames) {
                    if sender.send(item).is_err() {
                        return Ok(());
                    }
                }
            }
            Ok(())
        });
        let result = receiver
            .iter()
            .try_for_each(|(index, frame)| encode(index, frame));
        drop(receiver);
        let rendered = producer
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        result.and(rendered)
    })
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
    fn render_pipeline_stops_producer_when_encoding_fails() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for max_frames in [1, 3] {
            let rendered = AtomicUsize::new(0);
            let deadline = RequestDeadline::new(Instant::now(), "gif", Duration::from_secs(10));
            let mut seen = Vec::new();
            let error = encode_rendered_frames(
                100,
                max_frames,
                &|index| {
                    rendered.fetch_add(1, Ordering::Relaxed);
                    Img::new(1, 1, [index as u8, 0, 0, 255])
                },
                &deadline,
                |index, frame| {
                    seen.push(index);
                    assert_eq!(frame.data[0], index as u8);
                    if index == 2 {
                        Err(PreviewError::render("encoding stopped"))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            assert_eq!(seen, [0, 1, 2]);
            assert!(rendered.load(Ordering::Relaxed) < 100);
            assert!(error.to_string().contains("encoding stopped"));
        }
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
        let error = save_animated_gif_streamed_with_colors(
            1,
            |_| {
                rendered.store(true, Ordering::Relaxed);
                Img::new(1, 1, [0, 0, 0, 255])
            },
            &path,
            10.0,
            &expired("gif"),
            &[],
            false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("GIF preview request timed out"));
        assert!(!rendered.load(Ordering::Relaxed));
        assert!(!path.exists());
    }

    #[test]
    fn gif_preserves_note_rgb_during_fade() {
        let path =
            std::env::temp_dir().join(format!("osu-preview-fade-rgb-{}.gif", std::process::id()));
        let deadline = RequestDeadline::new(Instant::now(), "gif", Duration::from_secs(60));
        let render = |index| {
            let mut image = Img::new(64, 16, [51, 51, 51, 255]);
            for (position, color) in [[235, 69, 44, 255], [67, 142, 172, 255], [232, 198, 61, 255]]
                .into_iter()
                .enumerate()
            {
                image.alpha_composite_scaled(
                    &Img::new(16, 16, color),
                    position as i64 * 16,
                    0,
                    1.0 - index as f64 / 20.0,
                );
            }
            image
        };
        save_animated_gif_streamed_with_colors(21, render, &path, 20.0, &deadline, &[], true)
            .unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options
            .read_info(std::fs::File::open(&path).unwrap())
            .unwrap();
        let mut decoded = Img::new(64, 16, [0; 4]);
        for index in 0..21 {
            let frame = decoder.read_next_frame().unwrap().unwrap();
            decoded.alpha_composite(
                &Img {
                    w: u32::from(frame.width),
                    h: u32::from(frame.height),
                    data: frame.buffer.to_vec(),
                },
                i64::from(frame.left),
                i64::from(frame.top),
            );
            for (actual, expected) in decoded.data.iter().zip(&render(index).data) {
                assert!(
                    actual.abs_diff(*expected) <= 8,
                    "第 {index} 帧淡出颜色失真：{actual} != {expected}"
                );
            }
        }
        drop(decoder);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn gif_preserves_unsampled_colors_and_keeps_delta_with_palette_extensions() {
        let path = std::env::temp_dir().join(format!(
            "osu-preview-gif-late-color-{}.gif",
            std::process::id()
        ));
        let deadline = RequestDeadline::new(Instant::now(), "gif", Duration::from_secs(60));
        // 全局只采样 0、2、4、6；1、3 帧中的黄色和绿色完全不参与调色板训练。
        let render = |index| {
            let mut image = Img::new(64, 32, [0, 0, 0, 255]);
            let color = match index {
                1 => [232, 198, 61, 255],
                3 => [82, 204, 180, 255],
                _ => [235, 69, 44, 255],
            };
            image.fill_rect_size(8, 8, 40, 16, color);
            image.fill_rect_size(0, 0, 8, 8, [95, 221, 108, 255]);
            image
        };
        save_animated_gif_streamed_with_colors(
            7,
            render,
            &path,
            20.0,
            &deadline,
            &[[95, 221, 108]],
            false,
        )
        .unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options
            .read_info(std::fs::File::open(&path).unwrap())
            .unwrap();
        let mut decoded = Img::new(64, 32, [0; 4]);
        for index in 0..7 {
            let frame = decoder.read_next_frame().unwrap().unwrap();
            if [1, 3].contains(&index) {
                assert!(frame.palette.is_some(), "补充颜色必须随帧提供调色板");
            } else if [2, 4, 5].contains(&index) {
                assert!(frame.transparent.is_some(), "补色后仍应使用差分帧");
                assert_ne!(
                    (frame.left, frame.top, frame.width, frame.height),
                    (0, 0, 64, 32)
                );
            }
            decoded.alpha_composite(
                &Img {
                    w: u32::from(frame.width),
                    h: u32::from(frame.height),
                    data: frame.buffer.to_vec(),
                },
                i64::from(frame.left),
                i64::from(frame.top),
            );
            let expected = render(index);
            for (x, y) in [(20, 16), (2, 2)] {
                let actual = decoded.get(x, y);
                let source = expected.get(x, y);
                assert!(
                    actual.iter().zip(source).all(|(a, b)| a.abs_diff(b) <= 8),
                    "帧 {index}: {actual:?} != {source:?}"
                );
            }
        }
        drop(decoder);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn gif_palette_overflow_preserves_labels_and_resumes_delta() {
        let path = std::env::temp_dir().join(format!(
            "osu-preview-gif-palette-overflow-{}.gif",
            std::process::id()
        ));
        let deadline = RequestDeadline::new(Instant::now(), "gif", Duration::from_secs(60));
        let label = [95, 221, 108];
        let render = |index| {
            let mut image = Img::new(64, 32, [0, 0, 0, 255]);
            image.fill_rect_size(8, 8, 48, 16, [235, 69, 44, 255]);
            if index == 1 {
                // 抽样帧只有红色；216 种相距 51 的新颜色超过补色容量。
                for i in 0..216 {
                    image.fill_rect_size(
                        8 + i % 24 * 2,
                        8 + i / 24,
                        2,
                        1,
                        [
                            (i / 36 * 51) as u8,
                            (i / 6 % 6 * 51) as u8,
                            (i % 6 * 51) as u8,
                            255,
                        ],
                    );
                }
            }
            image.fill_rect_size(0, 0, 8, 8, [label[0], label[1], label[2], 255]);
            image
        };
        save_animated_gif_streamed_with_colors(7, render, &path, 20.0, &deadline, &[label], false)
            .unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options
            .read_info(std::fs::File::open(&path).unwrap())
            .unwrap();
        let mut decoded = Img::new(64, 32, [0; 4]);
        for index in 0..7 {
            let frame = decoder.read_next_frame().unwrap().unwrap();
            if index == 1 {
                assert!(frame.palette.is_some());
                assert_eq!((frame.width, frame.height), (64, 32));
            } else if index == 2 {
                assert!(frame.transparent.is_none(), "独立量化后必须完整覆盖画面");
                assert_eq!((frame.width, frame.height), (64, 32));
            } else if index > 2 {
                assert!(frame.transparent.is_some());
                assert_eq!((frame.width, frame.height), (1, 1));
            }
            decoded.alpha_composite(
                &Img {
                    w: u32::from(frame.width),
                    h: u32::from(frame.height),
                    data: frame.buffer.to_vec(),
                },
                i64::from(frame.left),
                i64::from(frame.top),
            );
            assert_eq!(decoded.get(2, 2), [label[0], label[1], label[2], 255]);
            let expected = render(index);
            for (actual, source) in decoded
                .data
                .chunks_exact(4)
                .zip(expected.data.chunks_exact(4))
            {
                assert!(
                    actual.iter().zip(source).all(|(a, b)| a.abs_diff(*b) <= 32),
                    "帧 {index}: {actual:?} != {source:?}"
                );
            }
        }
        drop(decoder);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn gif_preserves_dim_note_colors_on_large_gray_background() {
        let path = std::env::temp_dir().join(format!(
            "osu-preview-gif-note-colors-{}.gif",
            std::process::id()
        ));
        let deadline = RequestDeadline::new(Instant::now(), "gif", Duration::from_secs(60));
        let colors = [[68, 51, 51, 255], [51, 68, 85, 255]];
        save_animated_gif_streamed_with_colors(
            4,
            |_| {
                let mut image = Img::new(700, 500, [51, 51, 51, 255]);
                for (index, color) in colors.iter().enumerate() {
                    image.fill_rect_size(100 + index as i64 * 100, 100, 20, 20, *color);
                }
                image
            },
            &path,
            20.0,
            &deadline,
            &[],
            false,
        )
        .unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options
            .read_info(std::fs::File::open(&path).unwrap())
            .unwrap();
        let frame = decoder.read_next_frame().unwrap().unwrap();
        for (index, expected) in colors.iter().enumerate() {
            let offset = (110 * 700 + 110 + index * 100) * 4;
            let actual = &frame.buffer[offset..offset + 3];
            for channel in 0..3 {
                assert!(
                    (actual[channel] as i16 - expected[channel] as i16).abs() <= 8,
                    "暗色音符被错误量化：{expected:?} -> {actual:?}"
                );
            }
        }
        drop(decoder);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn gif_frame_delays_preserve_duration_at_fractional_centiseconds() {
        let deadline = RequestDeadline::new(
            std::time::Instant::now(),
            "gif",
            std::time::Duration::from_secs(60),
        );
        for fps in [15, 20, 30, 60] {
            let path = std::env::temp_dir().join(format!(
                "osu-preview-gif-timing-{}-{fps}.gif",
                std::process::id()
            ));
            save_animated_gif_streamed_with_colors(
                fps,
                |index| Img::new(4, 4, [(index * 16) as u8, 100, 200, 255]),
                &path,
                fps as f64,
                &deadline,
                &[],
                false,
            )
            .unwrap();
            let mut decoder = gif::DecodeOptions::new()
                .read_info(std::fs::File::open(&path).unwrap())
                .unwrap();
            let mut frame_count = 0;
            let mut elapsed_ms = 0;
            while let Some(frame) = decoder.read_next_frame().unwrap() {
                elapsed_ms += u64::from(frame.delay) * 10;
                frame_count += 1;
                let expected_ms = frame_count as f64 * 1000.0 / fps as f64;
                assert!(
                    (elapsed_ms as f64 - expected_ms).abs() <= 5.01,
                    "{fps}fps 的第 {frame_count} 帧累积时长错误：{elapsed_ms}ms"
                );
            }
            assert_eq!(frame_count, fps);
            assert_eq!(elapsed_ms, 1_000);
            drop(decoder);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn gif_preserves_exact_label_colors_through_quantization_and_delta_frames() {
        for (name, colors, background) in [
            (
                "default",
                [
                    [95, 221, 108, 255],
                    [232, 232, 232, 255],
                    [170, 170, 170, 255],
                ],
                [0, 0, 0, 255],
            ),
            // 前两个颜色落在同一个 5 位分桶，第三个验证与画布合成后的半透明颜色。
            (
                "custom",
                [
                    [94, 220, 109, 255],
                    [95, 221, 108, 255],
                    [203, 77, 142, 128],
                ],
                [17, 29, 43, 255],
            ),
        ] {
            let path = std::env::temp_dir().join(format!(
                "osu-preview-label-colors-{}-{name}.gif",
                std::process::id()
            ));
            let fixed = colors.map(|color| {
                let mut pixel = Img::new(1, 1, background);
                pixel.blend_px(0, 0, color);
                [pixel.data[0], pixel.data[1], pixel.data[2]]
            });
            let render = |index| {
                let mut image = Img::new(256, 64, background);
                for (row, color) in colors.iter().enumerate() {
                    crate::export::text::draw_text(
                        &mut image,
                        4,
                        4 + row as i64 * 12,
                        "Preview Time",
                        8,
                        *color,
                    );
                }
                // 丰富的动态颜色会挤占量化调色板，不能依赖“画面足够简单”碰巧保留文字原色。
                for y in 40..56 {
                    for x in 0..256 {
                        image.put(
                            x,
                            y,
                            [
                                x as u8,
                                (y * 4) as u8,
                                (x + y + index as u32 * 17) as u8,
                                255,
                            ],
                        );
                    }
                }
                image
            };
            let deadline = RequestDeadline::new(Instant::now(), "gif", Duration::from_secs(30));
            let mut reserved = fixed.to_vec();
            reserved.push(fixed[0]);
            save_animated_gif_streamed_with_colors(
                3, render, &path, 15.0, &deadline, &reserved, false,
            )
            .unwrap();
            let mut options = gif::DecodeOptions::new();
            options.set_color_output(gif::ColorOutput::RGBA);
            let mut decoder = options
                .read_info(std::fs::File::open(&path).unwrap())
                .unwrap();
            assert!(decoder.global_palette().unwrap().len() <= 128 * 3);
            let mut decoded = Img::new(256, 64, [0; 4]);
            for index in 0..3 {
                let frame = decoder.read_next_frame().unwrap().unwrap();
                let image = Img {
                    w: u32::from(frame.width),
                    h: u32::from(frame.height),
                    data: frame.buffer.to_vec(),
                };
                decoded.alpha_composite(&image, i64::from(frame.left), i64::from(frame.top));
                let expected = render(index);
                for color in fixed {
                    let mut count = 0;
                    for (source, actual) in expected
                        .data
                        .chunks_exact(4)
                        .zip(decoded.data.chunks_exact(4))
                    {
                        if source[..3] == color {
                            assert_eq!(
                                &actual[..3],
                                &color,
                                "{name} 的第 {index} 帧文字颜色发生改变"
                            );
                            count += 1;
                        }
                    }
                    assert!(count > 0);
                }
            }
            assert!(decoder.read_next_frame().unwrap().is_none());
            drop(decoder);
            std::fs::remove_file(path).unwrap();
        }
    }
}
