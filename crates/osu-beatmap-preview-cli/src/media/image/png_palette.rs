//! 纯色 PNG 的精确调色板；颜色超限时交回通用量化器。

use super::*;
use std::collections::HashMap;

pub(super) fn exact_palette(
    image: &Img,
    deadline: &RequestDeadline,
) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
    let mut colors = HashMap::with_capacity(256);
    let mut palette = Vec::with_capacity(256 * 3);
    let mut indexed = Vec::with_capacity(image.data.len() / 4);
    let mut previous = None;
    for (i, pixel) in image.data.chunks_exact(4).enumerate() {
        if i % 1_000_000 == 0 {
            deadline.check()?;
        }
        // PNG 原有路径忽略 alpha；这里只按 RGB 建表，保持相同的输出语义。
        let rgb = [pixel[0], pixel[1], pixel[2]];
        let index = if let Some((_, index)) = previous.filter(|&(color, _)| color == rgb) {
            index
        } else {
            let next = colors.len();
            match colors.entry(rgb) {
                std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    if next == 256 {
                        return Ok(None);
                    }
                    palette.extend_from_slice(&rgb);
                    *entry.insert(next as u8)
                }
            }
        };
        indexed.push(index);
        previous = Some((rgb, index));
    }
    Ok(Some((palette, indexed)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn exact_colors_include_256_entries_and_fall_back_on_overflow() {
        let deadline = RequestDeadline::new(Instant::now(), "png", Duration::from_secs(10));
        let mut image = Img::new(257, 1, [0; 4]);
        for x in 0..256 {
            image.put(x, 0, [x as u8, 10, 20, x as u8]);
        }
        image.put(256, 0, [0, 10, 20, 255]);
        let (palette, indices) = exact_palette(&image, &deadline).unwrap().unwrap();
        assert_eq!(palette.len(), 256 * 3);
        for (pixel, &index) in image.data.chunks_exact(4).zip(&indices) {
            assert_eq!(
                &palette[index as usize * 3..index as usize * 3 + 3],
                &pixel[..3]
            );
        }
        image.put(256, 0, [0, 11, 20, 255]);
        assert!(exact_palette(&image, &deadline).unwrap().is_none());
    }

    #[test]
    fn png_round_trip_preserves_exact_rgb_and_encodes_quantized_fallback() {
        let deadline = RequestDeadline::new(Instant::now(), "png", Duration::from_secs(10));
        for width in [1, 256, 257] {
            let mut image = Img::new(width, 2, [0; 4]);
            for x in 0..width {
                let color = [x as u8, (x / 256) as u8, 83, 255];
                image.put(x, 0, color);
                image.put(x, 1, color);
            }
            let path = std::env::temp_dir().join(format!(
                "osu-preview-png-palette-{}-{width}.png",
                std::process::id()
            ));
            save_png_exact_colors(&image, &path, &deadline).unwrap();
            let decoded = image::open(&path).unwrap().into_rgba8();
            assert_eq!(decoded.dimensions(), (width, 2));
            if width <= 256 {
                assert_eq!(decoded.as_raw(), &image.data);
            }
            std::fs::remove_file(path).unwrap();
        }
    }
}
