//! 太鼓行背景与 note 的程序化绘制（classic-2013 风格，无图片资源）。

use crate::parser::round_half_even;
use crate::render::canvas::Img;
use std::collections::HashMap;

// ─── 工具 ───

#[inline]
fn pyround(v: f64) -> i64 {
    round_half_even(v)
}

// ─── 缓存 ───

/// 渲染缓存：note 圆盘与滚奏尾端按（颜色, 尺寸）缓存，避免重复光栅化。
#[derive(Default)]
pub(crate) struct RenderCache {
    discs: HashMap<([u8; 3], i64, bool), Img>,
    tails: HashMap<([u8; 3], i64), Img>,
    drum_roll_ticks: HashMap<(i64, u8), Img>,
}

// ─── 行背景（程序化，替代原 taiko-bar-left/right 图片） ───

/// 左侧鼓面板宽度与行高的比例（原图 362×400 的纵横比）。
/// 绘制左侧鼓面板：红色竖条 + 米色鼓面圆，模拟 classic 皮肤的 taiko-bar-left。
pub(crate) fn draw_drum_panel(image: &mut Img, x: i64, y: i64, w: i64, h: i64) {
    if w <= 0 || h <= 0 {
        return;
    }
    // 底色：深色面板
    image.fill_rect(x, y, x + w - 1, y + h - 1, [30, 30, 30, 255]);
    // 左右红色饰条（约占面板宽度的 12%）
    let stripe = ((w as f64 * 0.12) as i64).max(2);
    image.fill_rect(
        x,
        y,
        x + stripe - 1,
        y + h - 1,
        crate::config::current().layout.taiko.png.TRACK_ACCENT_COLOR,
    );
    image.fill_rect(
        x + w - stripe,
        y,
        x + w - 1,
        y + h - 1,
        crate::config::current().layout.taiko.png.TRACK_ACCENT_COLOR,
    );
    // 中央鼓面：米色圆 + 深色描边
    let cx = x as f64 + w as f64 / 2.0;
    let cy = y as f64 + h as f64 / 2.0;
    let r = (h.min(w) as f64) * 0.36;
    image.fill_circle_aa(cx, cy, r + 1.5, [20, 20, 20, 255]);
    image.fill_circle_aa(cx, cy, r, [248, 238, 220, 255]);
    // 鼓面中线（左右手分界）
    image.fill_rect(
        pyround(cx) - 1,
        pyround(cy - r),
        pyround(cx),
        pyround(cy + r),
        [180, 165, 135, 255],
    );
}

/// 绘制 note 滚动轨道背景：半透明深灰长条，上下各 1px 高光边，
/// 模拟 classic 皮肤的 taiko-bar-right。
pub(crate) fn draw_track_background(image: &mut Img, x: i64, y: i64, w: i64, h: i64) {
    if w <= 0 || h <= 0 {
        return;
    }
    image.fill_rect(
        x,
        y,
        x + w - 1,
        y + h - 1,
        crate::config::current()
            .layout
            .taiko
            .png
            .TRACK_BACKGROUND_COLOR,
    );
    image.fill_rect(
        x,
        y,
        x + w - 1,
        y,
        crate::config::current().layout.taiko.png.TRACK_EDGE_COLOR,
    );
    image.fill_rect(
        x,
        y + h - 1,
        x + w - 1,
        y + h - 1,
        crate::config::current().layout.taiko.png.TRACK_EDGE_COLOR,
    );
}

/// Classic-2013 风格音符：实心抗锯齿圆盘、浅色圆环边框、1px 深色外缘，
/// 无中心符号。`swell_marker` 会增加内环（替代 spinner-warning 精灵图）。
pub(crate) fn build_note_disc(color: [u8; 3], diameter: i64, swell_marker: bool) -> Img {
    let d = diameter.max(1);
    let mut img = Img::new(d as u32, d as u32, [0, 0, 0, 0]);
    let c = d as f64 / 2.0;
    let r = c;
    let ring = (d as f64 * crate::render::taiko::constants::NOTE_RING_THICKNESS_RATIO).max(1.0);
    let fill: [u8; 4] = [color[0], color[1], color[2], 255];
    img.fill_circle_aa(c, c, r, crate::render::taiko::constants::NOTE_EDGE_COLOR);
    img.fill_circle_aa(
        c,
        c,
        r - 1.0,
        crate::render::taiko::constants::NOTE_RING_COLOR,
    );
    img.fill_circle_aa(c, c, r - 1.0 - ring, fill);
    if swell_marker {
        let inner_r = (r - 1.0 - ring) * 0.55;
        img.fill_circle_aa(
            c,
            c,
            inner_r,
            crate::render::taiko::constants::NOTE_RING_COLOR,
        );
        img.fill_circle_aa(c, c, inner_r - ring.max(1.0), fill);
    }
    img
}

pub(crate) fn cached_note_disc(
    cache: &mut RenderCache,
    color: [u8; 3],
    diameter: i64,
    swell_marker: bool,
) -> &Img {
    cache
        .discs
        .entry((color, diameter, swell_marker))
        .or_insert_with(|| build_note_disc(color, diameter, swell_marker))
}

pub(crate) fn build_roll_tail_sprite(color: [u8; 3], height: i64) -> Img {
    let scale: i64 = 4;
    let scaled_height = height * scale;
    let scaled_width = (((height as f64) / 2.0).ceil() as i64 * scale).max(1);
    let radius = scaled_height as f64 / 2.0;
    let border_width = pyround(height as f64 * 0.05).max(1) * scale;

    let mut tail = Img::new(
        scaled_width.max(1) as u32,
        scaled_height.max(1) as u32,
        [0, 0, 0, 0],
    );
    tail.fill_ellipse(-radius, 0.0, radius, scaled_height as f64, [0, 0, 0, 255]);
    tail.fill_ellipse(
        -radius + border_width as f64,
        border_width as f64,
        radius - border_width as f64,
        (scaled_height - border_width) as f64,
        [color[0], color[1], color[2], 255],
    );
    tail.resize(((scaled_width / scale).max(1)) as u32, height.max(1) as u32)
}

pub(crate) fn cached_roll_tail(cache: &mut RenderCache, color: [u8; 3], height: i64) -> &Img {
    cache
        .tails
        .entry((color, height))
        .or_insert_with(|| build_roll_tail_sprite(color, height))
}

/// 构造白色实心菱形连打点，并通过高分辨率光栅化保留小尺寸边缘。
pub(crate) fn build_drum_roll_tick_sprite(diameter: i64, alpha: f64) -> Img {
    let diameter = diameter.max(1);
    let scale = 4i64;
    let scaled_diameter = diameter * scale;
    let center = scaled_diameter as f64 / 2.0;
    let radius = center;
    let alpha = alpha.clamp(0.0, 1.0);
    let color = crate::render::taiko::constants::DRUM_ROLL_TICK_COLOR;
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

pub(crate) fn cached_drum_roll_tick(cache: &mut RenderCache, diameter: i64, alpha: f64) -> &Img {
    let alpha_byte = pyround(alpha.clamp(0.0, 1.0) * 255.0).clamp(0, 255) as u8;
    cache
        .drum_roll_ticks
        .entry((diameter.max(1), alpha_byte))
        .or_insert_with(|| build_drum_roll_tick_sprite(diameter, alpha_byte as f64 / 255.0))
}

pub(crate) fn draw_note_disc(
    image: &mut Img,
    cache: &mut RenderCache,
    color: [u8; 3],
    diameter: i64,
    center_x: i64,
    center_y: i64,
    swell_marker: bool,
) {
    let pos_x = pyround(center_x as f64 - diameter as f64 / 2.0);
    let pos_y = pyround(center_y as f64 - diameter as f64 / 2.0);
    let disc = cached_note_disc(cache, color, diameter, swell_marker);
    image.alpha_composite(disc, pos_x, pos_y);
}

pub(crate) fn paste_clipped(
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
}
