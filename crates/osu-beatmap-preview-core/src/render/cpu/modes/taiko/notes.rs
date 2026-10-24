//! 太鼓行背景与 note 的程序化绘制（classic-2013 风格，无图片资源）。

use super::constants::{
    DRUM_PANEL_BACKGROUND_COLOR, DRUM_PANEL_CENTER_LINE_COLOR, DRUM_PANEL_FACE_COLOR,
    DRUM_PANEL_OUTLINE_COLOR,
};
use crate::domain::parser::round_half_even;
use crate::render::canvas::Img;
use std::collections::HashMap;
use std::sync::Arc;

// ─── 工具 ───

#[inline]
fn pyround(v: f64) -> i64 {
    round_half_even(v)
}

// ─── 缓存 ───

/// 渲染缓存：note 圆盘与滚奏尾端按（颜色, 尺寸）缓存，避免重复光栅化。
#[derive(Default)]
pub struct RenderCache {
    swells: HashMap<(i64, [u8; 4]), Img>,
    animation_swells: HashMap<(i64, [u8; 4], i64, bool), Img>,
    discs: HashMap<([u8; 3], i64, i64, bool), Arc<Img>>,
    tails: HashMap<([u8; 3], i64), Arc<Img>>,
    drum_roll_ticks: HashMap<(i64, u8), Arc<Img>>,
}

// ─── 行背景（程序化，替代原 taiko-bar-left/right 图片） ───

/// 左侧鼓面板宽度与行高的比例（原图 362×400 的纵横比）。
/// 绘制左侧鼓面板：红色竖条 + 米色鼓面圆，模拟 classic 皮肤的 taiko-bar-left。
pub fn draw_drum_panel(
    image: &mut Img,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    accent_color: [u8; 4],
    detail_scale: f64,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    // 底色：深色面板
    image.fill_rect_size(x, y, w, h, DRUM_PANEL_BACKGROUND_COLOR);
    // 左右红色饰条（约占面板宽度的 12%）
    let stripe = ((w as f64 * 0.12) as i64).max(2);
    image.fill_rect_size(x, y, stripe, h, accent_color);
    image.fill_rect_size(x + w - stripe, y, stripe, h, accent_color);
    // 中央鼓面：米色圆 + 深色描边
    let cx = x as f64 + w as f64 / 2.0;
    let cy = y as f64 + h as f64 / 2.0;
    let r = (h.min(w) as f64) * 0.36;
    image.fill_circle_aa(
        cx,
        cy,
        r + (1.5 * detail_scale).max(0.5),
        DRUM_PANEL_OUTLINE_COLOR,
    );
    image.fill_circle_aa(cx, cy, r, DRUM_PANEL_FACE_COLOR);
    // 鼓面中线（左右手分界）
    let center_line_width = crate::render::geometry::scale_stroke_px(2.0, detail_scale);
    image.fill_rect_size(
        pyround(cx) - center_line_width / 2,
        pyround(cy - r),
        center_line_width,
        pyround(2.0 * r).max(1),
        DRUM_PANEL_CENTER_LINE_COLOR,
    );
}

/// 绘制 note 滚动轨道背景：半透明深灰长条，上下各 1px 高光边，
/// 模拟 classic 皮肤的 taiko-bar-right。
#[allow(clippy::too_many_arguments)]
pub fn draw_track_background(
    image: &mut Img,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    background_color: [u8; 4],
    edge_color: [u8; 4],
    edge_width: i64,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    image.fill_rect_size(x, y, w, h, background_color);
    let edge_width = edge_width.clamp(1, h);
    image.fill_rect_size(x, y, w, edge_width, edge_color);
    image.fill_rect_size(x, y + h - edge_width, w, edge_width, edge_color);
}

/// Classic-2013 风格音符：实心抗锯齿圆盘、浅色圆环边框、1px 深色外缘，
/// 无中心符号。`swell_marker` 使用独立的完整空心转盘。
pub fn build_note_disc(color: [u8; 3], diameter: i64, edge_width: i64, swell_marker: bool) -> Img {
    if swell_marker {
        return build_swell_disc(diameter);
    }
    let d = diameter.max(1);
    let mut img = Img::new(d as u32, d as u32, [0, 0, 0, 0]);
    let c = d as f64 / 2.0;
    let r = c;
    let ring = (d as f64 * crate::render::cpu::modes::taiko::constants::NOTE_RING_THICKNESS_RATIO)
        .max(1.0);
    let fill: [u8; 4] = [color[0], color[1], color[2], 255];
    img.fill_circle_aa(
        c,
        c,
        r,
        crate::render::cpu::modes::taiko::constants::NOTE_EDGE_COLOR,
    );
    let edge_width = edge_width.max(1) as f64;
    img.fill_circle_aa(
        c,
        c,
        r - edge_width,
        crate::render::cpu::modes::taiko::constants::NOTE_RING_COLOR,
    );
    img.fill_circle_aa(c, c, r - edge_width - ring, fill);
    img
}

/// 默认转盘图标为空心圆，动画圆环另按直径与颜色缓存。
fn build_swell_disc(diameter: i64) -> Img {
    use super::constants::{NOTE_RING_COLOR, NOTE_RING_THICKNESS_RATIO};
    let diameter = diameter.max(1);
    let mut image = Img::new(diameter as u32, diameter as u32, [0, 0, 0, 0]);
    let center = diameter as f64 / 2.0;
    image.stroke_circle_aa(
        center,
        center,
        center,
        diameter as f64 * NOTE_RING_THICKNESS_RATIO,
        NOTE_RING_COLOR,
    );
    image
}

pub fn cached_swell_disc(cache: &mut RenderCache, diameter: i64) -> &Img {
    cached_colored_swell_disc(cache, diameter, super::constants::NOTE_RING_COLOR)
}

/// 圆环颜色参与缓存键，静态图标与动画转盘分别复用各自的配色。
pub fn cached_colored_swell_disc(cache: &mut RenderCache, diameter: i64, color: [u8; 4]) -> &Img {
    cache
        .swells
        .entry((diameter.max(1), color))
        .or_insert_with(|| {
            let diameter = diameter.max(1);
            let mut image = Img::new(diameter as u32, diameter as u32, [0, 0, 0, 0]);
            let center = diameter as f64 / 2.0;
            image.stroke_circle_aa(
                center,
                center,
                center,
                diameter as f64 * super::constants::NOTE_RING_THICKNESS_RATIO,
                color,
            );
            image
        })
}

/// 外圈和固定中心标志一次光栅化，线宽与标志开关参与缓存，避免逐帧重绘。
pub fn cached_animation_swell_disc(
    cache: &mut RenderCache,
    diameter: i64,
    color: [u8; 4],
    thickness_ratio: f64,
    marker: bool,
) -> &Img {
    let diameter = diameter.max(1);
    // 非法倍率回退默认值，线宽不超过半径，避免圆环退化为实心圆。
    let ratio = if thickness_ratio.is_finite() {
        thickness_ratio.clamp(0.01, 0.2)
    } else {
        0.08
    };
    let thickness = pyround(diameter as f64 * ratio).max(1);
    cache
        .animation_swells
        .entry((diameter, color, thickness, marker))
        .or_insert_with(|| {
            let mut image = Img::new(diameter as u32, diameter as u32, [0, 0, 0, 0]);
            let center = diameter as f64 / 2.0;
            image.stroke_circle_aa(center, center, center, thickness as f64, color);
            if marker {
                // 沿用原转盘标志的 0.55 半径比例，小环包围数字，保持圆心透明。
                let radius = (center - thickness as f64) * 0.55;
                image.stroke_circle_aa(center, center, radius, thickness as f64, color);
            }
            image
        })
}

pub fn cached_note_disc(
    cache: &mut RenderCache,
    color: [u8; 3],
    diameter: i64,
    edge_width: i64,
    swell_marker: bool,
) -> &Img {
    cached_note_disc_arc(cache, color, diameter, edge_width, swell_marker)
}

pub fn cached_note_disc_arc(
    cache: &mut RenderCache,
    color: [u8; 3],
    diameter: i64,
    edge_width: i64,
    swell_marker: bool,
) -> &Arc<Img> {
    cache
        .discs
        .entry((color, diameter, edge_width, swell_marker))
        .or_insert_with(|| Arc::new(build_note_disc(color, diameter, edge_width, swell_marker)))
}

pub fn build_roll_tail_sprite(color: [u8; 3], height: i64) -> Img {
    let scale: i64 = 4;
    let scaled_height = height * scale;
    let scaled_width = (((height as f64) / 2.0).ceil() as i64 * scale).max(1);
    let radius = scaled_height as f64 / 2.0;

    let mut tail = Img::new(
        scaled_width.max(1) as u32,
        scaled_height.max(1) as u32,
        [0, 0, 0, 0],
    );
    // 尾部外缘与主体必须同色；黑色底层会在滑条和转盘末端形成一圈黑边。
    tail.fill_ellipse(
        -radius,
        0.0,
        radius,
        scaled_height as f64,
        [color[0], color[1], color[2], 255],
    );
    tail.resize(((scaled_width / scale).max(1)) as u32, height.max(1) as u32)
}

pub fn cached_roll_tail(cache: &mut RenderCache, color: [u8; 3], height: i64) -> &Img {
    cached_roll_tail_arc(cache, color, height)
}

pub fn cached_roll_tail_arc(cache: &mut RenderCache, color: [u8; 3], height: i64) -> &Arc<Img> {
    cache
        .tails
        .entry((color, height))
        .or_insert_with(|| Arc::new(build_roll_tail_sprite(color, height)))
}

/// 构造白色实心菱形连打点，并通过高分辨率光栅化保留小尺寸边缘。
pub fn build_drum_roll_tick_sprite(diameter: i64, alpha: f64) -> Img {
    let diameter = diameter.max(1);
    let scale = 4i64;
    let scaled_diameter = diameter * scale;
    let center = scaled_diameter as f64 / 2.0;
    let radius = center;
    let alpha = alpha.clamp(0.0, 1.0);
    let color = crate::render::cpu::modes::taiko::constants::DRUM_ROLL_TICK_COLOR;
    let output_alpha = pyround(color[3] as f64 * alpha).clamp(0, 255) as u8;
    let mut sprite = Img::new(scaled_diameter as u32, scaled_diameter as u32, [0, 0, 0, 0]);

    for y in 0..scaled_diameter {
        for x in 0..scaled_diameter {
            let dx = x as f64 + 0.5 - center;
            let dy = y as f64 + 0.5 - center;
            if dx.abs() + dy.abs() > radius {
                continue;
            }
            sprite.put(
                x as u32,
                y as u32,
                [color[0], color[1], color[2], output_alpha],
            );
        }
    }

    sprite.resize(diameter as u32, diameter as u32)
}

pub fn cached_drum_roll_tick(cache: &mut RenderCache, diameter: i64, alpha: f64) -> &Img {
    cached_drum_roll_tick_arc(cache, diameter, alpha)
}

pub fn cached_drum_roll_tick_arc(cache: &mut RenderCache, diameter: i64, alpha: f64) -> &Arc<Img> {
    let alpha_byte = pyround(alpha.clamp(0.0, 1.0) * 255.0).clamp(0, 255) as u8;
    cache
        .drum_roll_ticks
        .entry((diameter.max(1), alpha_byte))
        .or_insert_with(|| {
            Arc::new(build_drum_roll_tick_sprite(
                diameter,
                alpha_byte as f64 / 255.0,
            ))
        })
}

#[allow(clippy::too_many_arguments)]
pub fn draw_note_disc(
    image: &mut Img,
    cache: &mut RenderCache,
    color: [u8; 3],
    diameter: i64,
    center_x: i64,
    center_y: i64,
    edge_width: i64,
    swell_marker: bool,
) {
    let pos_x = pyround(center_x as f64 - diameter as f64 / 2.0);
    let pos_y = pyround(center_y as f64 - diameter as f64 / 2.0);
    let disc = cached_note_disc(cache, color, diameter, edge_width, swell_marker);
    image.alpha_composite(disc, pos_x, pos_y);
}

pub fn paste_clipped(
    image: &mut Img,
    sprite: &Img,
    x: i64,
    y: i64,
    clip_left: i64,
    clip_right: i64,
) {
    let sprite_left = x;
    let sprite_right = x + sprite.w as i64;
    let visible_left = sprite_left.max(clip_left);
    let visible_right = sprite_right.min(clip_right);
    if visible_right <= visible_left {
        return;
    }
    if visible_left == sprite_left && visible_right == sprite_right {
        image.alpha_composite(sprite, x, y);
        return;
    }
    let crop_left = (visible_left - sprite_left) as u32;
    let crop_right = crop_left + (visible_right - visible_left) as u32;
    let cropped = sprite.crop(crop_left, 0, crop_right.min(sprite.w), sprite.h);
    image.alpha_composite(&cropped, visible_left, y);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roll_tail_uses_body_color_at_the_outer_edge() {
        let color = [232, 198, 61];
        let sprite = build_roll_tail_sprite(color, 20);
        let edge = sprite.get(8, 10);
        assert_eq!(&edge[..3], &color);
        assert!(edge[3] > 0);
    }

    #[test]
    fn drum_roll_tick_is_a_solid_white_diamond() {
        let sprite = build_drum_roll_tick_sprite(16, 1.0);
        assert_eq!(sprite.get(8, 8), [255, 255, 255, 255]);
        assert!(sprite.get(8, 1)[3] > 0);
        assert!(sprite.get(0, 0)[3] < sprite.get(8, 1)[3]);
    }

    #[test]
    fn drum_roll_tick_cache_reuses_frames_and_separates_animation_states() {
        let mut cache = RenderCache::default();
        let first = cached_drum_roll_tick(&mut cache, 8, 1.0) as *const Img;
        let repeated = cached_drum_roll_tick(&mut cache, 8, 1.0) as *const Img;
        assert_eq!(first, repeated);
        let _ = cached_drum_roll_tick(&mut cache, 10, 0.5);
        assert_eq!(cache.drum_roll_ticks.len(), 2);
    }

    #[test]
    fn swell_marker_is_hollow_and_has_no_colored_fill() {
        let sprite = build_note_disc(super::super::constants::SWELL_COLOR, 48, 1, true);
        assert_eq!(sprite.get(24, 24)[3], 0);
        assert!(sprite
            .data
            .chunks_exact(4)
            .all(|pixel| pixel[3] == 0
                || pixel[..3] == super::super::constants::NOTE_RING_COLOR[..3]));
        assert!(sprite.get(24, 1)[3] > 0);
        assert_eq!(sprite.get(0, 0)[3], 0);
    }

    #[test]
    fn swell_ring_is_complete_at_every_angle() {
        let sprite = build_note_disc(super::super::constants::SWELL_COLOR, 96, 1, true);
        for step in 0..360 {
            let angle = (step as f64).to_radians();
            let x = (48.0 + 46.0 * angle.cos()).floor() as u32;
            let y = (48.0 + 46.0 * angle.sin()).floor() as u32;
            assert!(sprite.get(x, y)[3] > 0, "转盘边框不能有缺口：{step} 度");
        }
    }

    #[test]
    fn swell_cache_reuses_diameters() {
        let mut cache = RenderCache::default();
        let first = cached_swell_disc(&mut cache, 48) as *const Img;
        let repeated = cached_swell_disc(&mut cache, 48) as *const Img;
        assert_eq!(first, repeated);
        let _ = cached_swell_disc(&mut cache, 56);
        assert_eq!(cache.swells.len(), 2);
    }
}
