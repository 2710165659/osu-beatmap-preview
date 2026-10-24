//! GIF 调色板补色、颜色分桶和有界量化；稳定已有颜色索引以保留帧间差分。

use crate::export::canvas::Img;

/// 将通道海报化为 5 位（32 级），复制高位以保持完整 0..255 范围。
/// 这能稳定帧间抗锯齿/渐变像素，缩小差分区域并延长 LZW 连续段。
#[inline]
pub(super) fn posterize(v: u8) -> u8 {
    (v & 0xF8) | (v >> 5)
}

/// 预计算将海报化 RGB 映射到调色板索引的 32³ 查找表。
///
/// `posterize()` 每通道产生 32 个值，`>> 3` 把它们无冲突地映射到 32 个
/// 槽位，因此完整颜色空间可放入 32768 项数组；`transparent_idx` 映射到前
/// 一个调色板项，避免被当作普通像素索引。每个槽位由 `posterize(ri << 3)` 构建，与查
/// 找时的颜色精确对齐，命中与逐像素 `index_of()` 相同的结果、无量化漂移。
pub(super) fn build_gif_lut(
    nq: &color_quant::NeuQuant,
    transparent_idx: u8,
) -> [[[u8; 32]; 32]; 32] {
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

pub(super) struct AdaptivePalette {
    colors: Vec<u8>,
    base_len: usize,
    lut: [[[u8; 32]; 32]; 32],
    base_lut: [[[u8; 32]; 32]; 32],
    // 0：尚未验证，1：桶内误差有界，2：容量不足；避免重复搜索无法容纳的颜色。
    resolved: [u8; 32768],
    transparent_idx: u8,
    exact_colors: Vec<([u8; 3], u8)>,
    max_color_error: u8,
}

impl AdaptivePalette {
    pub(super) fn new(
        colors: Vec<u8>,
        lut: [[[u8; 32]; 32]; 32],
        transparent_idx: u8,
        exact_colors: Vec<([u8; 3], u8)>,
        max_color_error: u8,
    ) -> Self {
        Self {
            base_len: colors.len(),
            colors,
            lut,
            base_lut: lut,
            resolved: [0; 32768],
            transparent_idx,
            exact_colors,
            max_color_error,
        }
    }

    pub(super) fn frame_palette(&self) -> Option<Vec<u8>> {
        (self.colors.len() != self.base_len).then(|| self.colors.clone())
    }

    /// 返回是否重置索引，以及当前帧是否仍需独立量化。
    pub(super) fn index_frame(&mut self, image: &Img, indexed: &mut Vec<u8>) -> (bool, bool) {
        let mut overflow = self.map_frame(image, indexed);
        let reset = overflow && self.colors.len() != self.base_len;
        if reset {
            // 回收旧场景的补色，不能让一次容量耗尽导致后续所有帧持续回退。
            self.colors.truncate(self.base_len);
            self.lut = self.base_lut;
            self.resolved.fill(0);
            overflow = self.map_frame(image, indexed);
        }
        let distorted =
            overflow && palette_distorts_frame(image, indexed, &self.colors, self.max_color_error);
        (reset, distorted)
    }

    fn map_frame(&mut self, image: &Img, indexed: &mut Vec<u8>) -> bool {
        indexed.clear();
        let mut overflow = false;
        let mut previous = [0u8; 4];
        let mut previous_index = 0;
        let mut has_previous = false;
        for pixel in image.data.chunks_exact(4) {
            // 大片背景、音符填色和文字横线复用上个像素的映射，跳过固定色与补色检查。
            if has_previous && pixel == previous {
                indexed.push(previous_index);
                continue;
            }
            previous.copy_from_slice(pixel);
            has_previous = true;
            // 标签等固定颜色绕过分桶，同桶中的相近颜色仍按自己的 RGB 映射。
            if let Some(&(_, index)) = self
                .exact_colors
                .iter()
                .find(|(color, _)| pixel[..3] == *color)
            {
                previous_index = index;
                indexed.push(index);
                continue;
            }
            let r = pixel[0] as usize >> 3;
            let g = pixel[1] as usize >> 3;
            let b = pixel[2] as usize >> 3;
            let key = (r << 10) | (g << 5) | b;
            if self.resolved[key] == 0 {
                self.resolve_color(pixel, r, g, b, key);
            }
            overflow |= self.resolved[key] == 2;
            previous_index = self.lut[r][g][b];
            indexed.push(previous_index);
        }
        overflow
    }

    fn resolve_color(&mut self, pixel: &[u8], r: usize, g: usize, b: usize, key: usize) {
        let source = [
            posterize(pixel[0]),
            posterize(pixel[1]),
            posterize(pixel[2]),
        ];
        let bucket = [pixel[0] & 0xF8, pixel[1] & 0xF8, pixel[2] & 0xF8];
        // 验证整个桶的误差上界，后续帧只需查表，无需逐像素检查偏色。
        let max_color_error = self.max_color_error;
        let fits_bucket = |color: &[u8]| {
            bucket.iter().zip(color).all(|(&lo, &c)| {
                lo.abs_diff(c) <= max_color_error && (lo + 7).abs_diff(c) <= max_color_error
            })
        };
        let index = self.lut[r][g][b] as usize;
        if fits_bucket(&self.colors[index * 3..index * 3 + 3]) {
            self.resolved[key] = 1;
            return;
        }
        let existing = self
            .colors
            .chunks_exact(3)
            .enumerate()
            .find(|(index, color)| *index != self.transparent_idx as usize && fits_bucket(color));
        let index = if let Some((index, _)) = existing {
            index
        } else if self.colors.len() < 256 * 3 {
            let index = self.colors.len() / 3;
            self.colors.extend_from_slice(&source);
            index
        } else {
            self.resolved[key] = 2;
            return;
        };
        self.lut[r][g][b] = index as u8;
        self.resolved[key] = 1;
    }
}

/// 补色容量不足时检查当前帧的颜色误差，超出容差才回退到局部量化。
fn palette_distorts_frame(
    image: &Img,
    indexed: &[u8],
    palette: &[u8],
    max_color_error: u8,
) -> bool {
    image
        .data
        .chunks_exact(4)
        .zip(indexed)
        .any(|(pixel, &index)| {
            let color = &palette[index as usize * 3..index as usize * 3 + 3];
            pixel[..3]
                .iter()
                .zip(color)
                .any(|(&source, &encoded)| source.abs_diff(encoded) > max_color_error)
        })
}

/// 调色板容量耗尽时，以有界颜色样本量化当前帧，避免对整张 RGBA 图重复训练。
pub(super) fn local_palette_frame(image: Img, fixed_colors: &[[u8; 3]]) -> gif::Frame<'static> {
    let mut colors = fixed_colors.to_vec();
    colors.sort_unstable();
    colors.dedup();
    let mut exact: std::collections::HashMap<[u8; 3], u8> = colors
        .iter()
        .enumerate()
        .map(|(index, color)| (*color, index as u8))
        .collect();
    let mut fits_exactly = true;
    for pixel in image.data.chunks_exact(4) {
        let color = [pixel[0], pixel[1], pixel[2]];
        if let std::collections::hash_map::Entry::Vacant(entry) = exact.entry(color) {
            if colors.len() == 256 {
                fits_exactly = false;
                break;
            }
            entry.insert(colors.len() as u8);
            colors.push(color);
        }
    }
    // 颜色足够少时直接编码原色，既无需量化，也不能让少数颜色因采样权重过低而偏色。
    if fits_exactly {
        return gif::Frame {
            width: image.w as u16,
            height: image.h as u16,
            palette: Some(colors.into_iter().flatten().collect()),
            buffer: std::borrow::Cow::Owned(
                image
                    .data
                    .chunks_exact(4)
                    .map(|pixel| exact[&[pixel[0], pixel[1], pixel[2]]])
                    .collect(),
            ),
            ..Default::default()
        };
    }
    let mut counts = [0u8; 32768];
    let mut sample = Vec::new();
    for pixel in image.data.chunks_exact(4) {
        let key = ((pixel[0] as usize >> 3) << 10)
            | ((pixel[1] as usize >> 3) << 5)
            | (pixel[2] as usize >> 3);
        if counts[key] < 64 {
            counts[key] += 1;
            sample.extend_from_slice(pixel);
        }
    }
    let mut colors = fixed_colors.to_vec();
    colors.sort_unstable();
    colors.dedup();
    let learned = 255 - colors.len();
    let nq = color_quant::NeuQuant::new(10, learned, &sample);
    let mut palette = nq.color_map_rgb();
    let exact: Vec<_> = colors
        .into_iter()
        .enumerate()
        .map(|(index, color)| {
            palette.extend_from_slice(&color);
            (color, (learned + index) as u8)
        })
        .collect();
    let lut = build_gif_lut(&nq, 255);
    let mut indexed = Vec::new();
    rgba_to_indexed(
        &image,
        &lut,
        &mut indexed,
        (image.w * image.h) as usize,
        &exact,
    );
    gif::Frame {
        width: image.w as u16,
        height: image.h as u16,
        palette: Some(palette),
        buffer: std::borrow::Cow::Owned(indexed),
        ..Default::default()
    }
}

/// 将 RGBA 帧映射为 GIF 使用的 indexed 像素。
///
/// 帧尺寸由渲染器保证一致；输入数据不足时用 0 填充尾部。
fn rgba_to_indexed(
    frame: &Img,
    lut: &[[[u8; 32]; 32]; 32],
    indexed: &mut Vec<u8>,
    pixel_count: usize,
    exact_colors: &[([u8; 3], u8)],
) {
    indexed.clear();
    // 容量已在动画开始时预分配，resize 不会在正常帧尺寸下重新分配。
    indexed.resize(pixel_count, 0);
    for (i, px) in frame.data.chunks_exact(4).enumerate().take(pixel_count) {
        // 精确颜色绕过海报化和 5 位分桶；同桶中的相近颜色仍走原查找表，避免误染音符。
        indexed[i] = exact_colors
            .iter()
            .find(|(color, _)| px[..3] == *color)
            .map_or_else(
                || {
                    lut[posterize(px[0]) as usize >> 3][posterize(px[1]) as usize >> 3]
                        [posterize(px[2]) as usize >> 3]
                },
                |&(_, index)| index,
            );
    }
}
