//! BMFont 字形与纹理；资源由调用端提供，core 不读取文件。
use crate::{render::canvas::Img, PreviewError, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy)]
struct Glyph {
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    offset: [i16; 2],
    advance: i16,
}
#[derive(Clone)]
pub struct ArgonFont {
    atlas: Img,
    glyphs: HashMap<char, Glyph>,
    kernings: HashMap<(char, char), i16>,
    nominal_size: f64,
    // HUD 常在相邻帧保持不变；共享精灵同时避免字形重建和 GPU 纹理重传。
    sprite_cache: Arc<Mutex<HashMap<SpriteKey, Arc<Img>>>>,
}

type SpriteKey = (String, u64, u64, [u8; 3]);

impl ArgonFont {
    /// Torus 官方资源使用 BMFont v3；显式指定的字体损坏时报错，不悄悄回退。
    pub fn from_bmfont(bytes: &[u8], atlas: Img) -> Result<Self> {
        let invalid =
            || PreviewError::new("InvalidArgonFont: expected a single-page BMFont v3 atlas");
        if bytes.get(..4) != Some(b"BMF\x03") {
            return Err(invalid());
        }
        let expected = (atlas.w as usize)
            .checked_mul(atlas.h as usize)
            .and_then(|size| size.checked_mul(4));
        if atlas.w == 0 || atlas.h == 0 || expected != Some(atlas.data.len()) {
            return Err(invalid());
        }
        let mut cursor = 4;
        let mut glyphs = HashMap::new();
        let mut kernings = HashMap::new();
        let mut nominal_size = 0.0;
        let mut single_page = false;
        while cursor < bytes.len() {
            let header = bytes.get(cursor..cursor + 5).ok_or_else(invalid)?;
            let kind = header[0];
            let length = u32::from_le_bytes(header[1..5].try_into().unwrap()) as usize;
            cursor += 5;
            let end = cursor.checked_add(length).ok_or_else(invalid)?;
            let block = bytes.get(cursor..end).ok_or_else(invalid)?;
            let word = |offset| u16::from_le_bytes(block[offset..offset + 2].try_into().unwrap());
            match kind {
                1 if block.len() >= 14 => nominal_size = (word(0) as i16).unsigned_abs() as f64,
                2 if block.len() >= 15 => {
                    // 官方纹理会裁掉未用边缘，实际 PNG 小于 BMFont 声明尺寸是合法的。
                    if word(8) != 1 {
                        return Err(invalid());
                    }
                    single_page = true;
                }
                4 => {
                    if !block.len().is_multiple_of(20) {
                        return Err(invalid());
                    }
                    for record in block.chunks_exact(20) {
                        let Some(character) =
                            char::from_u32(u32::from_le_bytes(record[..4].try_into().unwrap()))
                        else {
                            continue;
                        };
                        let word = |offset| {
                            u16::from_le_bytes(record[offset..offset + 2].try_into().unwrap())
                        };
                        let glyph = Glyph {
                            x: word(4),
                            y: word(6),
                            width: word(8),
                            height: word(10),
                            offset: [word(12) as i16, word(14) as i16],
                            advance: word(16) as i16,
                        };
                        if record[18] != 0
                            || glyph.x as u32 + glyph.width as u32 > atlas.w
                            || glyph.y as u32 + glyph.height as u32 > atlas.h
                        {
                            return Err(invalid());
                        }
                        if glyphs.insert(character, glyph).is_some() {
                            return Err(invalid());
                        }
                    }
                }
                5 => {
                    if !block.len().is_multiple_of(10) {
                        return Err(invalid());
                    }
                    for record in block.chunks_exact(10) {
                        let first =
                            char::from_u32(u32::from_le_bytes(record[..4].try_into().unwrap()));
                        let second =
                            char::from_u32(u32::from_le_bytes(record[4..8].try_into().unwrap()));
                        if let (Some(first), Some(second)) = (first, second) {
                            kernings.insert(
                                (first, second),
                                i16::from_le_bytes(record[8..10].try_into().unwrap()),
                            );
                        }
                    }
                }
                _ => {}
            }
            cursor = end;
        }
        if !single_page || nominal_size <= 0.0 || glyphs.is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            atlas,
            glyphs,
            kernings,
            nominal_size,
            sprite_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// 固定标签不能静默丢字；宿主载入图集后检查全部显示字符。
    pub fn supports(&self, text: &str) -> bool {
        text.chars()
            .all(|character| self.glyphs.contains_key(&character))
    }

    /// 字距 5 与字号 20 来自 ArgonJudgementPiece；缩放单个字形后再拼接。
    pub(super) fn sprite_arc(
        &self,
        text: &str,
        size: f64,
        spacing: f64,
        color: [u8; 3],
    ) -> Arc<Img> {
        let key = (text.to_owned(), size.to_bits(), spacing.to_bits(), color);
        if let Some(sprite) = self.sprite_cache.lock().unwrap().get(&key) {
            return Arc::clone(sprite);
        }
        let sprite = self.build_sprite(text, size, spacing, color);
        let sprite = Arc::new(sprite);
        let mut cache = self.sprite_cache.lock().unwrap();
        // 分数与时间会持续变化，限制缓存项数，长回放和任意 seek 都不会无限占用内存。
        if cache.len() >= 128 {
            cache.clear();
        }
        Arc::clone(cache.entry(key).or_insert(sprite))
    }

    fn build_sprite(&self, text: &str, size: f64, spacing: f64, color: [u8; 3]) -> Img {
        let scale = size / self.nominal_size;
        let mut positions = Vec::with_capacity(text.len());
        let mut pen = 0.0;
        let mut previous = None;
        for character in text.chars() {
            let Some(glyph) = self.glyphs.get(&character) else {
                continue;
            };
            if let Some(first) = previous {
                pen += self.kernings.get(&(first, character)).copied().unwrap_or(0) as f64 * scale
                    + spacing;
            }
            positions.push((
                glyph,
                pen + glyph.offset[0] as f64 * scale,
                glyph.offset[1] as f64 * scale,
            ));
            pen += glyph.advance as f64 * scale;
            previous = Some(character);
        }
        // BMFont 允许负偏移与负字距，画布按真实字形边界分配，避免截掉左侧笔画。
        let left = positions
            .iter()
            .map(|(_, x, _)| *x)
            .fold(0.0, f64::min)
            .floor();
        let top = positions
            .iter()
            .map(|(_, _, y)| *y)
            .fold(0.0, f64::min)
            .floor();
        let right = positions
            .iter()
            .map(|(g, x, _)| x + g.width as f64 * scale)
            .fold(1.0, f64::max)
            .ceil();
        let bottom = positions
            .iter()
            .map(|(g, _, y)| y + g.height as f64 * scale)
            .fold(1.0, f64::max)
            .ceil();
        let mut image = Img::new(
            (right - left).max(1.0) as u32,
            (bottom - top).max(1.0) as u32,
            [0; 4],
        );
        for (glyph, x, y) in positions {
            if glyph.width > 0 && glyph.height > 0 {
                let mut piece = self
                    .atlas
                    .crop(
                        glyph.x as u32,
                        glyph.y as u32,
                        glyph.x as u32 + glyph.width as u32,
                        glyph.y as u32 + glyph.height as u32,
                    )
                    .resize_bilinear(
                        (glyph.width as f64 * scale).round().max(1.0) as u32,
                        (glyph.height as f64 * scale).round().max(1.0) as u32,
                    );
                for pixel in piece.data.chunks_exact_mut(4) {
                    pixel[..3].copy_from_slice(&color);
                }
                image.alpha_composite(&piece, (x - left).round() as i64, (y - top).round() as i64);
            }
        }
        if let Some((x0, y0, x1, y1)) = image.alpha_bbox() {
            image.crop(x0, y0, x1, y1)
        } else {
            image
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_truncation_and_out_of_bounds_atlases() {
        for bytes in [b"BMF".as_slice(), b"BMF\x03\x04\xff\xff\xff\xff".as_slice()] {
            assert!(ArgonFont::from_bmfont(bytes, Img::new(1, 1, [255; 4])).is_err());
        }
        let bytes = fixture();
        assert!(ArgonFont::from_bmfont(&bytes, Img::new(1, 1, [255; 4])).is_err());
        let mut atlas = Img::new(3, 2, [255; 4]);
        atlas.data.pop();
        assert!(ArgonFont::from_bmfont(&bytes, atlas).is_err());
    }

    #[test]
    fn preserves_negative_offsets_kerning_and_scaling() {
        let font = ArgonFont::from_bmfont(&fixture(), Img::new(3, 2, [255; 4])).unwrap();
        assert!(font.supports("AA"));
        assert!(!font.supports("B"));
        let sprite = font.sprite_arc("AA", 10.0, 0.0, [102, 204, 255]);
        assert_eq!((sprite.w, sprite.h), (5, 2));
        assert!(sprite
            .data
            .chunks_exact(4)
            .all(|pixel| pixel == [102, 204, 255, 255]));
        let sprite = font.sprite_arc("AA", 20.0, 0.0, [255; 3]);
        assert_eq!((sprite.w, sprite.h), (10, 4));
    }

    #[test]
    fn repeated_labels_reuse_resources_and_dynamic_text_cache_is_bounded() {
        let font = ArgonFont::from_bmfont(&fixture(), Img::new(3, 2, [255; 4])).unwrap();
        let first = font.sprite_arc("AA", 10.0, 0.0, [255; 3]);
        assert!(Arc::ptr_eq(
            &first,
            &font.sprite_arc("AA", 10.0, 0.0, [255; 3])
        ));
        assert!(!Arc::ptr_eq(
            &first,
            &font.sprite_arc("AA", 20.0, 0.0, [255; 3])
        ));
        for size in 1..=256 {
            font.sprite_arc("A", f64::from(size), 0.0, [255; 3]);
        }
        assert!(font.sprite_cache.lock().unwrap().len() <= 128);
        // 驱逐后已有场景仍持有原图；重新生成必须保持像素一致。
        assert_eq!(first.data, font.sprite_arc("AA", 10.0, 0.0, [255; 3]).data);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = b"BMF\x03".to_vec();
        let mut block = |kind: u8, body: &[u8]| {
            bytes.push(kind);
            bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
            bytes.extend_from_slice(body);
        };
        let mut info = [0; 14];
        info[..2].copy_from_slice(&10_i16.to_le_bytes());
        block(1, &info);
        let mut common = [0; 15];
        common[8..10].copy_from_slice(&1_u16.to_le_bytes());
        block(2, &common);
        let mut glyph = [0; 20];
        glyph[..4].copy_from_slice(&('A' as u32).to_le_bytes());
        for (offset, value) in [(8, 3_i16), (10, 2), (12, -1), (14, -1), (16, 3)] {
            glyph[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
        block(4, &glyph);
        let mut kerning = [0; 10];
        kerning[..4].copy_from_slice(&('A' as u32).to_le_bytes());
        kerning[4..8].copy_from_slice(&('A' as u32).to_le_bytes());
        kerning[8..].copy_from_slice(&(-1_i16).to_le_bytes());
        block(5, &kerning);
        bytes
    }
}
