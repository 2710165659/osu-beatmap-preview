//! 回放判定文字与粒子资源；判定位置由具体模式提供。
use super::{font::ArgonFont, overlay::OverlayCanvas};
use crate::{
    gameplay::events::HitResult,
    render::{
        canvas::Img,
        text::{render_text_sprite, scaled_bitmap_font_height},
    },
};
use std::sync::Arc;

pub(crate) struct FeedbackVisuals {
    pub labels: [Arc<Img>; 4],
    pub rings: [[Arc<Img>; 2]; 3],
    pub tick_misses: [Arc<Img>; 2],
}

const COLORS: [[u8; 3]; 4] = [
    [102, 204, 255],
    [136, 179, 0],
    [255, 204, 34],
    [237, 17, 33],
];

impl FeedbackVisuals {
    pub(crate) fn new(scale: f64, font: Option<&ArgonFont>) -> Self {
        Self::with_spacing(scale, font, 5.0)
    }

    pub(crate) fn new_taiko(scale: f64, font: Option<&ArgonFont>) -> Self {
        Self::with_spacing(scale, font, 10.0)
    }

    fn with_spacing(scale: f64, font: Option<&ArgonFont>, spacing: f64) -> Self {
        let labels = std::array::from_fn(|i| {
            let text = ["GREAT", "OK", "MEH", "MISS"][i];
            let color = COLORS[i];
            font.map_or_else(
                || Arc::new(bitmap_label(text, scale, spacing, color)),
                |font| font.sprite_arc(text, 20.0 * scale, spacing * scale, color),
            )
        });
        let rings = std::array::from_fn(|i| {
            std::array::from_fn(|size| {
                let radius = [4.5, 7.0][size] * scale;
                let side = (radius * 2.0).ceil().max(1.0) as u32 + 2;
                let center = f64::from(side) / 2.0;
                let mut ring = Img::new(side, side, [0; 4]);
                let color = COLORS[i];
                crate::render::cpu::modes::standard::slider::draw_ring_aa(
                    &mut ring,
                    center,
                    center,
                    radius,
                    4.0 * scale,
                    [color[0], color[1], color[2], 255],
                );
                Arc::new(ring)
            })
        });
        let tick_misses = std::array::from_fn(|i| {
            let radius = 6.0 * scale;
            let side = (radius * 2.0).ceil() as u32 + 2;
            let center = f64::from(side) / 2.0;
            let mut piece = Img::new(side, side, [0; 4]);
            let color = if i == 0 { [128; 3] } else { COLORS[3] };
            crate::render::cpu::modes::standard::slider::fill_circle_gradient_aa(
                &mut piece, center, center, radius, color, color,
            );
            Arc::new(piece)
        });
        Self {
            labels,
            rings,
            tick_misses,
        }
    }
}

fn bitmap_label(text: &str, scale: f64, spacing: f64, color: [u8; 3]) -> Img {
    let height = scaled_bitmap_font_height(20, scale);
    if spacing <= 5.0 {
        return render_text_sprite(text, height, [color[0], color[1], color[2], 255]);
    }

    let glyphs: Vec<_> = text
        .chars()
        .map(|character| {
            render_text_sprite(
                &character.to_string(),
                height,
                [color[0], color[1], color[2], 255],
            )
        })
        .collect();
    let gap = (spacing * scale).round() as i64;
    let width = glyphs.iter().map(|glyph| i64::from(glyph.w)).sum::<i64>()
        + gap * glyphs.len().saturating_sub(1) as i64;
    let mut image = Img::new(width.max(1) as u32, height.max(1), [0; 4]);
    let mut x = 0;
    for glyph in &glyphs {
        image.alpha_composite(glyph, x, 0);
        x += i64::from(glyph.w) + gap;
    }
    image
}

pub(crate) fn draw_judgement(
    target: &mut impl OverlayCanvas,
    visuals: &FeedbackVisuals,
    result: HitResult,
    event_id: usize,
    mut position: [f64; 2],
    scale: f64,
    age: f64,
) -> bool {
    let (label, small, large, travel) = match result {
        HitResult::Great => (0, 4, 4, 52.0),
        HitResult::Ok => (1, 4, 0, 31.2),
        HitResult::Meh => (2, 3, 0, 15.6),
        HitResult::Miss => (3, 0, 0, 0.0),
        _ => return false,
    };
    let duration = 800.0;
    if !(0.0..duration).contains(&age) {
        return false;
    }
    let missed = !result.is_hit();
    let parent_alpha = 1.0 - age / duration;
    let expansion = if missed {
        1.6 - 0.6 * (age / 100.0).clamp(0.0, 1.0).powi(2)
    } else {
        1.0 + 0.2 * out_quint(age / 1800.0)
    };
    if small + large > 0 {
        let alpha = parent_alpha * (1.0 - out_quint(age / 1000.0));
        let mut seed = event_id as u64;
        for index in 0..small + large {
            let angle = random(&mut seed) * std::f64::consts::TAU;
            let distance = travel
                * (0.5 + random(&mut seed) * 0.5)
                * (0.3 + 0.7 * out_quint(age / 600.0))
                * scale;
            target.additive_transformed(
                &visuals.rings[label][usize::from(index >= small)],
                [
                    position[0] + angle.cos() * distance,
                    position[1] + angle.sin() * distance,
                ],
                1.0,
                0.0,
                alpha,
            );
        }
    }
    let alpha = if missed {
        parent_alpha
    } else {
        parent_alpha * out_quint(age / 300.0)
    };
    let rotation = if missed {
        position[1] += 100.0 * scale * (age / duration).powi(5);
        40.0 * (age / duration).powi(5)
    } else {
        0.0
    };
    target.additive_transformed(&visuals.labels[label], position, expansion, rotation, alpha);
    true
}

fn out_quint(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(5)
}

fn random(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_add(0x9e3779b97f4a7c15);
    let mut value = *seed;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((value ^ (value >> 31)) >> 11) as f64 / (1_u64 << 53) as f64
}
