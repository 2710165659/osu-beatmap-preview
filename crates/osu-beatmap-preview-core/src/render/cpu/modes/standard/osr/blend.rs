//! Argon 判定使用 (SrcAlpha, One) 加法混合；只影响回放反馈层。
use crate::render::canvas::Img;

pub(super) fn composite(destination: &mut Img, source: &Img, x: i64, y: i64, alpha: f64) {
    let alpha = alpha.clamp(0.0, 1.0);
    let left = x.max(0);
    let top = y.max(0);
    let right = (x + i64::from(source.w)).min(i64::from(destination.w));
    let bottom = (y + i64::from(source.h)).min(i64::from(destination.h));
    for py in top..bottom {
        for px in left..right {
            let source_pixel = source.get((px - x) as u32, (py - y) as u32);
            let factor = f64::from(source_pixel[3]) / 255.0 * alpha;
            if factor == 0.0 {
                continue;
            }
            let index = destination.idx(px as u32, py as u32);
            let destination_pixel = &mut destination.data[index..index + 4];
            for channel in 0..3 {
                destination_pixel[channel] = (f64::from(destination_pixel[channel])
                    + f64::from(source_pixel[channel]) * factor)
                    .round()
                    .min(255.0) as u8;
            }
            destination_pixel[3] = (f64::from(destination_pixel[3]) + factor * 255.0)
                .round()
                .min(255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feedback_brightens_background_without_dimming_other_channels() {
        let mut background = Img::new(1, 1, [100, 110, 120, 255]);
        composite(
            &mut background,
            &Img::new(1, 1, [100, 0, 200, 128]),
            0,
            0,
            1.0,
        );
        assert_eq!(background.get(0, 0), [150, 110, 220, 255]);
        composite(&mut background, &Img::new(1, 1, [255; 4]), -1, 0, 1.0);
        assert_eq!(background.get(0, 0), [150, 110, 220, 255]);
    }
}
