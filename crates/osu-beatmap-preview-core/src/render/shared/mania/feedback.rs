//! 以代码轮廓近似皮肤字形，复现 Legacy Mania 判定的缩放、淡入淡出与 Miss 轻微旋转。
use crate::{gameplay::events::HitResult, render::shared::replay::overlay::OverlayCanvas};

pub(super) const LIFETIME: f64 = 220.0;
pub(super) const MAX_HEIGHT: f64 = 44.16 * 0.85;

#[derive(Debug)]
struct Animation {
    scale: f64,
    alpha: f64,
    rotation: f64,
}

fn animation(result: HitResult, age: f64) -> Option<Animation> {
    if !(0.0..LIFETIME).contains(&age) {
        return None;
    }
    // LegacyManiaJudgementPiece：20ms 淡入，停留 160ms，再用 40ms 淡出。
    let alpha = if age < 20.0 {
        1.0 - (1.0 - age / 20.0).powi(2)
    } else if age < 180.0 {
        1.0
    } else {
        1.0 - ((age - 180.0) / 40.0).powi(2)
    };
    if result == HitResult::Miss {
        let progress = 1.0 - (1.0 - (age / 100.0).min(1.0)).powi(2);
        Some(Animation {
            scale: 1.2 - 0.2 * progress,
            alpha,
            rotation: (-3.5_f64).to_radians() * progress,
        })
    } else {
        // 40ms 时的 1 → 0.85 跳变来自 stable 重叠的缩放变换，lazer 明确保留。
        let scale = if age < 40.0 {
            0.8 + 0.2 * age / 40.0
        } else if age < 80.0 {
            0.85 - 0.15 * (age - 40.0) / 40.0
        } else if age < 180.0 {
            0.7
        } else {
            0.7 - 0.3 * ((age - 180.0) / 40.0).powi(2)
        };
        Some(Animation {
            scale,
            alpha,
            rotation: 0.0,
        })
    }
}

pub(super) fn draw(
    target: &mut impl OverlayCanvas,
    result: HitResult,
    center: [i64; 2],
    scale: f64,
    age: f64,
) {
    let text = match result {
        HitResult::Perfect | HitResult::Great => "300",
        HitResult::Good => "200",
        HitResult::Ok => "100",
        HitResult::Meh => "50",
        HitResult::Miss => "MISS",
        _ => return,
    };
    let Some(animation) = animation(result, age) else {
        return;
    };
    if animation.alpha == 0.0 {
        return;
    }
    let factor = scale * animation.scale * 0.92 * 0.85;
    let width = text.chars().map(glyph_width).sum::<f64>() + (text.len() - 1) as f64 * 12.0;
    let (sin, cos) = animation.rotation.sin_cos();
    let bound_x = ((width * cos.abs() + 40.0 * sin.abs()) * factor / 2.0 + 2.0).ceil() as i64;
    let bound_y = ((40.0 * cos.abs() + width * sin.abs()) * factor / 2.0 + 2.0).ceil() as i64;
    // 对轮廓做 2×2 覆盖采样；输出按行合并的几何矩形，CPU 与实时渲染使用相同字形。
    for row in -bound_y..=bound_y {
        // 未旋转时同一采样行共享渐变与边缘色，避免对每个像素重复插值。
        let row_colours = (sin == 0.0).then(|| {
            [0.25, 0.75].map(|sy| {
                std::array::from_fn::<_, 4, _>(|index| {
                    SampleColours::new(result, index, (row as f64 + sy) / factor + 20.0)
                })
            })
        });
        let mut run: Option<(i64, [u8; 4])> = None;
        for column in -bound_x..=bound_x + 1 {
            let mut sum = [0_u32; 4];
            if column <= bound_x {
                for (sample, (sx, sy)) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]
                    .into_iter()
                    .enumerate()
                {
                    let dx = column as f64 + sx;
                    let dy = row as f64 + sy;
                    let x = (dx * cos + dy * sin) / factor + width / 2.0;
                    let y = (-dx * sin + dy * cos) / factor + 20.0;
                    if let Some(glyph) = glyph_at(text, x, y) {
                        let colours = row_colours.as_ref().map_or_else(
                            || SampleColours::new(result, glyph.index, y),
                            |rows| rows[sample / 2][glyph.index],
                        );
                        let color = if !glyph.contains(y - 1.0) {
                            colours.top
                        } else if !glyph.contains(y + 1.0) {
                            colours.bottom
                        } else {
                            colours.body
                        };
                        for channel in 0..3 {
                            sum[channel] += u32::from(color[channel]);
                        }
                        sum[3] += 255;
                    } else if glyph_at(text, x - 1.0, y - 1.0).is_some() {
                        sum[0] += 29;
                        sum[1] += 16;
                        sum[2] += 40;
                        sum[3] += 220;
                    }
                }
            }
            let color = if sum[3] == 0 {
                [0; 4]
            } else {
                let samples = (sum[3] as f64 / 255.0).max(1.0);
                [
                    (sum[0] as f64 / samples).min(255.0) as u8,
                    (sum[1] as f64 / samples).min(255.0) as u8,
                    (sum[2] as f64 / samples).min(255.0) as u8,
                    (sum[3] as f64 / 4.0 * animation.alpha).round() as u8,
                ]
            };
            if let Some((start, previous)) = run {
                if color != previous {
                    if previous[3] > 0 {
                        target.fill_rect_size(
                            center[0] + start,
                            center[1] + row,
                            column - start,
                            1,
                            previous,
                        );
                    }
                    run = Some((column, color));
                }
            } else {
                run = Some((column, color));
            }
        }
    }
}

#[derive(Clone, Copy)]
struct SampleColours {
    body: [u8; 3],
    top: [u8; 3],
    bottom: [u8; 3],
}

impl SampleColours {
    fn new(result: HitResult, index: usize, y: f64) -> Self {
        let body = colour(result, index, y / 40.0);
        Self {
            body,
            top: body.map(|c| (c as f64 * 0.8 + 51.0).min(255.0) as u8),
            bottom: body.map(|c| (c as f64 * 0.7) as u8),
        }
    }
}

fn glyph_width(ch: char) -> f64 {
    match ch {
        'M' => 32.0,
        'I' => 7.0,
        _ => 26.0,
    }
}
fn glyph_at(text: &str, mut x: f64, y: f64) -> Option<GlyphColumn> {
    if !(0.0..40.0).contains(&y) {
        return None;
    }
    for (index, ch) in text.chars().enumerate() {
        let width = glyph_width(ch);
        if (0.0..width).contains(&x) {
            let glyph = GlyphColumn {
                index,
                ch,
                x,
                width,
            };
            return glyph.contains(y).then_some(glyph);
        }
        x -= width + 12.0;
    }
    None
}

struct GlyphColumn {
    index: usize,
    ch: char,
    x: f64,
    width: f64,
}

impl GlyphColumn {
    fn contains(&self, y: f64) -> bool {
        if !(0.0..40.0).contains(&y) {
            return false;
        }
        let x = self.x;
        let top = y < 7.0;
        let middle = (16.5..23.5).contains(&y);
        let bottom = y >= 33.0;
        let left = x < 7.0;
        let right = x >= self.width - 7.0;
        match self.ch {
            '0' => top || bottom || left || right,
            '1' => (10.0..17.0).contains(&x) || ((top || bottom) && (2.0..24.0).contains(&x)),
            '2' => top || middle || bottom || (right && y < 20.0) || (left && y >= 20.0),
            '3' => top || middle || bottom || right,
            '5' | 'S' => top || middle || bottom || (left && y < 20.0) || (right && y >= 20.0),
            'I' => true,
            'M' => {
                left || right
                    || (y < 24.0
                        && ((x - (5.0 + y * 0.46)).abs() < 3.7
                            || (x - (27.0 - y * 0.46)).abs() < 3.7))
            }
            _ => false,
        }
    }
}
fn colour(result: HitResult, index: usize, t: f64) -> [u8; 3] {
    let (a, b) = match result {
        HitResult::Perfect => match index {
            0 => ([255, 238, 60], [100, 247, 78]),
            1 => ([255, 160, 70], [225, 252, 72]),
            _ => ([255, 84, 100], [255, 223, 90]),
        },
        HitResult::Great => ([255, 201, 35], [255, 237, 70]),
        HitResult::Good => ([207, 248, 255], [73, 188, 237]),
        HitResult::Ok => ([79, 236, 130], [67, 187, 152]),
        HitResult::Meh => ([255, 255, 255], [220, 218, 240]),
        _ => ([255, 107, 111], [255, 51, 47]),
    };
    [0, 1, 2].map(|c| (a[c] as f64 * (1.0 - t) + b[c] as f64 * t).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampled_colours_preserve_judgement_pixels_across_scales_and_animation_boundaries() {
        use crate::render::canvas::Img;

        // 原逐采样点绘制的像素快照，覆盖旋转、阴影、透明度与缩放变换边界。
        for (result, expected) in [
            (HitResult::Perfect, 17_225_438_832_574_883_641),
            (HitResult::Great, 17_393_788_238_787_792_583),
            (HitResult::Good, 11_281_343_374_749_794_310),
            (HitResult::Ok, 766_633_430_198_423_996),
            (HitResult::Meh, 10_711_917_906_043_983_304),
            (HitResult::Miss, 16_203_065_550_092_428_377),
        ] {
            let mut checksum = 0_u64;
            for scale in [0.75, 1.25, 2.8125] {
                for age in [
                    0.0, 0.125, 10.0, 20.0, 39.999, 40.0, 79.999, 80.0, 100.0, 179.999, 180.0,
                    200.0, 219.999, 220.0,
                ] {
                    let mut image = Img::new(384, 256, [17, 23, 31, 255]);
                    draw(&mut image, result, [192, 128], scale, age);
                    for byte in image.data {
                        checksum = checksum.wrapping_mul(16_777_619) ^ u64::from(byte);
                    }
                }
            }
            assert_eq!(checksum, expected, "{result:?}");
        }
    }

    #[test]
    fn legacy_animation_keeps_short_lifetime_and_overlapping_scale_transforms() {
        let at = |age| animation(HitResult::Perfect, age).unwrap();
        assert_eq!(at(0.0).alpha, 0.0);
        assert_eq!(at(20.0).alpha, 1.0);
        assert!((at(39.999).scale - 1.0).abs() < 0.001);
        assert_eq!(at(40.0).scale, 0.85);
        assert_eq!(at(80.0).scale, 0.7);
        assert!(at(200.0).scale < at(180.0).scale);
        assert!(animation(HitResult::Perfect, LIFETIME).is_none());
        let miss = animation(HitResult::Miss, 100.0).unwrap();
        assert_eq!(miss.scale, 1.0);
        assert!(miss.rotation.abs() < 5.73_f64.to_radians());
    }
}
