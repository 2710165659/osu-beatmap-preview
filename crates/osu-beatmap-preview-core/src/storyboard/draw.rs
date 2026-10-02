//! 故事板的坐标映射与 CPU 光栅绘制。
//!
//! 坐标映射对齐 osu!（`DrawableStoryboard`）：640×480 虚拟空间按**高度等比**
//! 缩放并居中（`scale = 画布高 / 480`），层遮罩裁剪到
//! `480 × (宽屏 ? 16/9 : 4/3)` 的居中矩形。因此 4:3 故事板在 16:9 画布上
//! 居中留边，宽屏故事板铺满宽度。
//!
//! 精灵绘制是一次逆映射光栅化：旋转、均匀/矢量缩放、翻转、颜色调制、
//! 淡入淡出与加色混合在同一次逐像素采样内完成，避免逐帧多次重采样分配。

use super::{ElementState, Origin, SpriteDraw, Storyboard, Textures};
use crate::render::canvas::Img;
use crate::render::scene::FrameSceneBuilder;
use std::sync::Arc;

/// 640×480 虚拟坐标 → 目标画布的映射与层遮罩（目标画布坐标，像素）。
#[derive(Clone, Copy, Debug)]
pub struct StoryboardViewport {
    /// 虚拟单位 → 画布像素的等比缩放（画布高 / 480）。
    pub scale: f32,
    /// 虚拟空间中心 (320,240) 对应的画布位置。
    pub center: [f32; 2],
    /// 层遮罩矩形（画布像素，x0, y0, x1, y1）。
    pub clip: [f32; 4],
}

impl StoryboardViewport {
    /// 为给定画布尺寸构建视口；`widescreen` 决定层遮罩的水平范围。
    pub fn new(canvas_width: f32, canvas_height: f32, widescreen: bool) -> Self {
        let scale = if canvas_height > 0.0 {
            canvas_height / super::VIRTUAL_HEIGHT
        } else {
            1.0
        };
        let center = [canvas_width / 2.0, canvas_height / 2.0];
        // osu! 的层遮罩：非宽屏 x∈[0,640]，宽屏 x∈[-106.67, 746.67]，y∈[0,480]。
        let half_width =
            super::VIRTUAL_HEIGHT / 2.0 * if widescreen { 16.0 / 9.0 } else { 4.0 / 3.0 };
        let half_width_px = half_width * scale;
        let half_height_px = super::VIRTUAL_HEIGHT / 2.0 * scale;
        Self {
            scale,
            center,
            clip: [
                (center[0] - half_width_px).max(0.0),
                (center[1] - half_height_px).max(0.0),
                (center[0] + half_width_px).min(canvas_width),
                (center[1] + half_height_px).min(canvas_height),
            ],
        }
    }

    /// 虚拟坐标 → 画布坐标。
    pub fn map_point(&self, point: [f32; 2]) -> [f32; 2] {
        [
            (point[0] - super::VIRTUAL_WIDTH / 2.0) * self.scale + self.center[0],
            (point[1] - super::VIRTUAL_HEIGHT / 2.0) * self.scale + self.center[1],
        ]
    }
}

/// 精灵的最终绘制几何（CPU 光栅与场景命令共用的中间表示）。
#[derive(Clone, Copy, Debug)]
pub struct SpriteTransform {
    /// 锚点位置（画布像素）。
    pub position: [f32; 2],
    /// 缩放后的目标尺寸（画布像素）；负值表示翻转。
    pub size: [f32; 2],
    /// 按翻转调整后的归一化原点（0 / 0.5 / 1）。
    pub origin: [f32; 2],
    /// 顺时针旋转弧度。
    pub rotation: f32,
    /// 颜色调制（RGB 0..1）。
    pub colour: [f32; 3],
    /// 不透明度（0..1）。
    pub alpha: f32,
    /// 加色混合（osu! 的 `P,,A`）。
    pub additive: bool,
}

/// 推导精灵的绘制几何。
///
/// 与 osu! 一致：`DrawScale = (FlipH ? -1 : 1) × Scale × VectorScale`，且翻转
/// （或负矢量缩放）时把原点锚在对边（`AdjustOrigin`），保证翻转只镜像内容、
/// 不产生位移。纹理尺寸为 0 或缩放为 0 时返回 `None`（不可见）。
pub fn sprite_geometry(
    state: &ElementState,
    origin: Origin,
    texture_size: [f32; 2],
    view: &StoryboardViewport,
) -> Option<SpriteTransform> {
    let (width, height) = (texture_size[0], texture_size[1]);
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let mut anchor = origin.anchor();
    if state.flip_h != (state.vector_scale[0] < 0.0) {
        anchor[0] = 1.0 - anchor[0];
    }
    if state.flip_v != (state.vector_scale[1] < 0.0) {
        anchor[1] = 1.0 - anchor[1];
    }
    let flip_x = if state.flip_h { -1.0 } else { 1.0 };
    let flip_y = if state.flip_v { -1.0 } else { 1.0 };
    let size = [
        width * state.scale * state.vector_scale[0] * flip_x * view.scale,
        height * state.scale * state.vector_scale[1] * flip_y * view.scale,
    ];
    if size[0] == 0.0 || size[1] == 0.0 || !size[0].is_finite() || !size[1].is_finite() {
        return None;
    }
    Some(SpriteTransform {
        position: view.map_point([state.x, state.y]),
        size,
        origin: anchor,
        rotation: state.rotation,
        colour: state.colour,
        alpha: state.alpha,
        additive: state.additive,
    })
}

/// 把一个精灵按最终几何绘制到画布（含旋转、翻转、颜色调制与加色混合）。
///
/// `clip` 是层遮罩（画布像素，x0, y0, x1, y1）；绘制顺序由调用方保证。
pub fn draw_transformed_sprite(
    canvas: &mut Img,
    texture: &Img,
    sprite: &SpriteTransform,
    clip: [f32; 4],
) {
    if canvas.w == 0 || canvas.h == 0 || texture.w == 0 || texture.h == 0 {
        return;
    }
    let (tw, th) = (texture.w as f32, texture.h as f32);
    let (sx, sy) = (sprite.size[0], sprite.size[1]);
    if sx == 0.0 || sy == 0.0 {
        return;
    }
    let anchor_px = [sprite.origin[0] * tw, sprite.origin[1] * th];
    // 每个纹理像素对应的画布像素：`size` 是整张贴图缩放后的宽高，
    // 逐像素步长要除回贴图尺寸，否则整张图会被放大约「贴图边长」倍。
    let (kx, ky) = (sx / tw, sy / th);
    let (sin, cos) = sprite.rotation.sin_cos();

    // 四角映射到画布坐标求包围盒，与层遮罩和画布边界求交后逐像素采样。
    let corner = |u: f32, v: f32| -> [f32; 2] {
        let local = [(u - anchor_px[0]) * kx, (v - anchor_px[1]) * ky];
        let rotated = [
            local[0] * cos - local[1] * sin,
            local[0] * sin + local[1] * cos,
        ];
        [
            sprite.position[0] + rotated[0],
            sprite.position[1] + rotated[1],
        ]
    };
    let corners = [
        corner(0.0, 0.0),
        corner(tw, 0.0),
        corner(tw, th),
        corner(0.0, th),
    ];
    let min_x = corners.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = corners.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
    let max_y = corners
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max);

    // 包围盒放宽 1 像素，覆盖跨过边界的像素中心；越界像素由 u/v 判定剔除。
    let x0 = (min_x.floor() as i64).max(clip[0] as i64).max(0);
    let x1 = ((max_x.ceil() as i64) + 1)
        .min(clip[2] as i64)
        .min(canvas.w as i64);
    let y0 = (min_y.floor() as i64).max(clip[1] as i64).max(0);
    let y1 = ((max_y.ceil() as i64) + 1)
        .min(clip[3] as i64)
        .min(canvas.h as i64);
    if x1 <= x0 || y1 <= y0 {
        return;
    }

    let colour = sprite.colour;
    let additive = sprite.additive;
    let alpha_factor = sprite.alpha;

    for py in y0..y1 {
        for px in x0..x1 {
            // 以像素中心 (px+0.5, py+0.5) 逆映射回纹理坐标。
            let dx = px as f32 + 0.5 - sprite.position[0];
            let dy = py as f32 + 0.5 - sprite.position[1];
            let lx = dx * cos + dy * sin;
            let ly = -dx * sin + dy * cos;
            let u = lx / kx + anchor_px[0];
            let v = ly / ky + anchor_px[1];
            if u < 0.0 || v < 0.0 || u > tw || v > th {
                continue;
            }
            let sampled = sample_bilinear_clamped(texture, u, v);
            let alpha = (sampled[3] / 255.0 * alpha_factor).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            let source = [
                (sampled[0] * colour[0]).round().clamp(0.0, 255.0),
                (sampled[1] * colour[1]).round().clamp(0.0, 255.0),
                (sampled[2] * colour[2]).round().clamp(0.0, 255.0),
            ];
            let index = (py as u32 * canvas.w + px as u32) as usize * 4;
            let dst = &mut canvas.data[index..index + 4];
            if additive {
                // osu! 的 Additive = (SrcAlpha, One)：dst += src × alpha。
                for channel in 0..3 {
                    dst[channel] = (dst[channel] as f32 + source[channel] * alpha)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
                dst[3] = (dst[3] as f32 + alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            } else {
                for channel in 0..3 {
                    let blended = source[channel] * alpha + dst[channel] as f32 * (1.0 - alpha);
                    dst[channel] = blended.round().clamp(0.0, 255.0) as u8;
                }
                dst[3] = ((alpha + dst[3] as f32 / 255.0 * (1.0 - alpha)) * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// 双线性采样（clamp-to-edge，与 osu! 的纹理 WrapMode 一致）。
///
/// 采样坐标是「像素空间」（0..w），纹理像素中心在半整数处。
fn sample_bilinear_clamped(texture: &Img, u: f32, v: f32) -> [f32; 4] {
    let x = (u - 0.5).clamp(0.0, texture.w.saturating_sub(1) as f32);
    let y = (v - 0.5).clamp(0.0, texture.h.saturating_sub(1) as f32);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(texture.w - 1);
    let y1 = (y0 + 1).min(texture.h - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let mut result = [0.0; 4];
    for channel in 0..4 {
        let top = texture.data[(y0 * texture.w + x0) as usize * 4 + channel] as f32 * (1.0 - fx)
            + texture.data[(y0 * texture.w + x1) as usize * 4 + channel] as f32 * fx;
        let bottom = texture.data[(y1 * texture.w + x0) as usize * 4 + channel] as f32 * (1.0 - fx)
            + texture.data[(y1 * texture.w + x1) as usize * 4 + channel] as f32 * fx;
        result[channel] = top * (1.0 - fy) + bottom * fy;
    }
    result
}

/// 把一个元素状态绘制到画布（内部按 [`sprite_geometry`] 推导几何）。
///
/// `brightness` 是用户暗度的亮度（1 − dim，如 `BACKGROUND_DIM=0.7` 时为 0.3），
/// 直接乘在精灵颜色上——与 lazer `UserDimContainer` 的
/// `FadeColour(Gray(1-DimLevel))` 等价（背景图也是同一亮度预暗化，逐像素结果一致）。
pub fn draw_sprite(
    canvas: &mut Img,
    texture: &Img,
    state: &ElementState,
    origin: Origin,
    view: &StoryboardViewport,
    brightness: f32,
) {
    if !state.x.is_finite() || !state.y.is_finite() {
        // osu! 对 NaN 坐标直接不绘制（IsPresent 检查）。
        return;
    }
    let Some(mut geometry) =
        sprite_geometry(state, origin, [texture.w as f32, texture.h as f32], view)
    else {
        return;
    };
    geometry.colour = [
        geometry.colour[0] * brightness,
        geometry.colour[1] * brightness,
        geometry.colour[2] * brightness,
    ];
    draw_transformed_sprite(canvas, texture, &geometry, view.clip);
}

/// 把时刻 `time_ms` 的故事板层画到画布上（`behind` 为物件下层、`front` 为 Overlay 层）。
///
/// 贴图按归一化路径查 `textures`；缺图的精灵跳过（与 osu! 贴图缺失时静默一致）。
pub fn draw_storyboard(
    canvas: &mut Img,
    storyboard: &Storyboard,
    textures: &Textures,
    time_ms: f64,
    behind: bool,
    view: &StoryboardViewport,
    brightness: f32,
) {
    let (behind_draws, front_draws) = storyboard.sprites_at(time_ms);
    let draws = if behind { &behind_draws } else { &front_draws };
    draw_sprites(canvas, draws, textures, view, brightness);
}

/// 按顺序绘制一组精灵（列表顺序即绘制顺序，后画者在上）。
pub fn draw_sprites(
    canvas: &mut Img,
    draws: &[SpriteDraw<'_>],
    textures: &Textures,
    view: &StoryboardViewport,
    brightness: f32,
) {
    for draw in draws {
        let Some(texture) = textures.get(&draw.texture_path()) else {
            continue;
        };
        draw_sprite(
            canvas,
            texture,
            &draw.state,
            draw.element.origin,
            view,
            brightness,
        );
    }
}

/// 把一组精灵转成场景命令（WGPU 实时路径），几何推导与 CPU 光栅完全一致。
///
/// `brightness` 语义同 [`draw_sprite`]：暗度预乘进精灵颜色。
pub fn append_scene_sprites(
    builder: &mut FrameSceneBuilder,
    draws: &[SpriteDraw<'_>],
    textures: &Textures,
    view: &StoryboardViewport,
    brightness: f32,
) {
    for draw in draws {
        if !draw.state.x.is_finite() || !draw.state.y.is_finite() {
            continue;
        }
        let Some(texture) = textures.get(&draw.texture_path()) else {
            continue;
        };
        let Some(geometry) = sprite_geometry(
            &draw.state,
            draw.element.origin,
            [texture.w as f32, texture.h as f32],
            view,
        ) else {
            continue;
        };
        let to_u8 = |value: f32| (value * 255.0).round().clamp(0.0, 255.0) as u8;
        let color = [
            to_u8(geometry.colour[0] * brightness),
            to_u8(geometry.colour[1] * brightness),
            to_u8(geometry.colour[2] * brightness),
            to_u8(geometry.alpha),
        ];
        builder.transformed_sprite(
            Arc::clone(texture),
            geometry.position,
            geometry.origin,
            geometry.size,
            geometry.rotation,
            color,
            geometry.additive,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storyboard::{AnimationLoop, Element, ElementCommands, ElementKind, Layer};

    /// 造一个测试用状态（测试辅助，仅测试构建使用）。
    #[cfg(test)]
    fn state_at(x: f32, y: f32, scale: f32) -> ElementState {
        ElementState {
            x,
            y,
            scale,
            vector_scale: [1.0, 1.0],
            rotation: 0.0,
            colour: [1.0, 1.0, 1.0],
            alpha: 1.0,
            additive: false,
            flip_h: false,
            flip_v: false,
            frame_index: 0,
        }
    }

    /// 视口按高度等比缩放并居中：4:3 故事板在 16:9 画布上水平居中留边。
    #[test]
    fn viewport_scales_by_height_and_centres() {
        let view = StoryboardViewport::new(960.0, 720.0, false);
        assert!((view.scale - 1.5).abs() < 1e-6);
        assert_eq!(view.map_point([320.0, 240.0]), [480.0, 360.0]);
        // 非宽屏遮罩 x∈[0,640] → 画布 x∈[0.0, 960.0]（640×1.5=960 正好铺满）。
        assert!((view.clip[0] - 0.0).abs() < 1e-6);
        assert!((view.clip[2] - 960.0).abs() < 1e-6);
    }

    /// 非宽屏故事板在 16:9 画布上被遮罩裁剪到 4:3 居中区域，宽屏放宽。
    #[test]
    fn widescreen_flag_widens_the_mask() {
        let view = StoryboardViewport::new(684.0, 384.0, false);
        let half = 320.0 * (384.0 / 480.0);
        assert!((view.clip[0] - (342.0 - half)).abs() < 1e-3);
        assert!((view.clip[2] - (342.0 + half)).abs() < 1e-3);
        let wide = StoryboardViewport::new(684.0, 384.0, true);
        assert!(wide.clip[2] - wide.clip[0] > view.clip[2] - view.clip[0]);
    }

    /// 中心原点的精灵几何：位置即中心，尺寸按 scale 换算。
    #[test]
    fn sprite_geometry_centres_on_declared_position() {
        let view = StoryboardViewport::new(640.0, 480.0, true);
        let geometry = sprite_geometry(
            &state_at(320.0, 240.0, 1.0),
            Origin::Centre,
            [64.0, 32.0],
            &view,
        )
        .expect("几何必须可推导");
        assert_eq!(geometry.position, [320.0, 240.0]);
        assert_eq!(geometry.size, [64.0, 32.0]);
        assert_eq!(geometry.origin, [0.5, 0.5]);
    }

    /// 翻转时原点锚换到对边（AdjustOrigin），保证内容镜像不位移。
    #[test]
    fn flip_swaps_origin_anchor() {
        let view = StoryboardViewport::new(640.0, 480.0, true);
        let mut state = state_at(320.0, 240.0, 1.0);
        state.flip_h = true;
        let geometry =
            sprite_geometry(&state, Origin::TopLeft, [64.0, 32.0], &view).expect("几何必须可推导");
        assert_eq!(geometry.origin, [1.0, 0.0]);
        assert_eq!(geometry.size, [-64.0, 32.0]);
    }

    /// 光栅绘制：不透明矩形画到中心，颜色调制生效。
    #[test]
    fn draw_sprite_composites_tinted_texture() {
        let view = StoryboardViewport::new(480.0, 480.0, true);
        let mut canvas = Img::new(480, 480, [0, 0, 0, 255]);
        let texture = Img::new(4, 4, [200, 100, 50, 255]);
        let mut state = state_at(320.0, 240.0, 1.0);
        state.colour = [0.5, 1.0, 1.0];
        draw_sprite(&mut canvas, &texture, &state, Origin::Centre, &view, 1.0);
        let pixel = canvas.get(240, 240);
        assert_eq!(pixel, [100, 100, 50, 255], "颜色调制按分量乘");
    }

    /// 暗度预乘进精灵颜色（lazer `UserDimContainer` 的 `FadeColour(Gray(1-Dim))` 语义）。
    #[test]
    fn brightness_dims_sprite_colour_like_user_dim() {
        let view = StoryboardViewport::new(480.0, 480.0, true);
        let mut canvas = Img::new(480, 480, [0, 0, 0, 255]);
        let texture = Img::new(4, 4, [200, 100, 50, 255]);
        draw_sprite(
            &mut canvas,
            &texture,
            &state_at(320.0, 240.0, 1.0),
            Origin::Centre,
            &view,
            0.3,
        );
        assert_eq!(
            canvas.get(240, 240),
            [60, 30, 15, 255],
            "亮度 0.3 乘在颜色上"
        );
    }

    /// 缩放语义：`size` 是整张贴图的缩放后宽高，覆盖范围与贴图像素一一对应
    ///（历史 bug：把整图尺寸当逐像素步长，整屏被放大约「贴图边长」倍）。
    #[test]
    fn sprite_extent_matches_scaled_texture_size() {
        let view = StoryboardViewport::new(640.0, 480.0, true);
        let mut canvas = Img::new(640, 480, [0, 0, 0, 255]);
        let texture = Img::new(4, 4, [255, 255, 255, 255]);
        draw_sprite(
            &mut canvas,
            &texture,
            &state_at(320.0, 240.0, 1.0),
            Origin::Centre,
            &view,
            1.0,
        );
        // 4×4 贴图在 scale=1 时精确覆盖 4×4 像素（318..321），范围外不能有内容。
        assert_ne!(canvas.get(318, 240), [0, 0, 0, 255]);
        assert_ne!(canvas.get(321, 240), [0, 0, 0, 255]);
        assert_eq!(canvas.get(317, 240), [0, 0, 0, 255], "范围外不得有内容");
        assert_eq!(canvas.get(322, 240), [0, 0, 0, 255], "范围外不得有内容");
    }

    /// 加色混合：dst += src × alpha，不覆盖底色。
    #[test]
    fn additive_blending_accumulates() {
        let view = StoryboardViewport::new(480.0, 480.0, true);
        let mut canvas = Img::new(480, 480, [10, 10, 10, 255]);
        let texture = Img::new(4, 4, [100, 100, 100, 255]);
        let mut state = state_at(320.0, 240.0, 1.0);
        state.additive = true;
        state.alpha = 0.5;
        draw_sprite(&mut canvas, &texture, &state, Origin::Centre, &view, 1.0);
        let pixel = canvas.get(240, 240);
        assert_eq!(pixel, [60, 60, 60, 255], "10 + 100×0.5 = 60");
    }

    /// 层遮罩裁剪：虚拟坐标 x<0 的内容在非宽屏视口下被裁掉，宽屏视口可见。
    #[test]
    fn layer_mask_clips_outside_virtual_bounds() {
        let texture = Img::new(10, 10, [255, 255, 255, 255]);
        // 800×480 画布：非宽屏遮罩虚拟 x∈[0,640]，宽屏放宽到 x≈[-100,700]。
        let view = StoryboardViewport::new(800.0, 480.0, false);
        let mut canvas = Img::new(800, 480, [0, 0, 0, 255]);
        draw_sprite(
            &mut canvas,
            &texture,
            &state_at(-20.0, 240.0, 1.0),
            Origin::Centre,
            &view,
            1.0,
        );
        assert_eq!(canvas.get(60, 240), [0, 0, 0, 255], "遮罩外不绘制");

        let wide = StoryboardViewport::new(800.0, 480.0, true);
        let mut canvas = Img::new(800, 480, [0, 0, 0, 255]);
        draw_sprite(
            &mut canvas,
            &texture,
            &state_at(-20.0, 240.0, 1.0),
            Origin::Centre,
            &wide,
            1.0,
        );
        assert_ne!(canvas.get(60, 240), [0, 0, 0, 255], "宽屏遮罩放宽后可见");
    }

    /// 旋转绘制：绕锚点顺时针旋转后内容仍覆盖中心区域。
    #[test]
    fn rotated_sprite_covers_centre() {
        let view = StoryboardViewport::new(480.0, 480.0, true);
        let mut canvas = Img::new(480, 480, [0, 0, 0, 255]);
        let texture = Img::new(8, 8, [255, 255, 255, 255]);
        let mut state = state_at(320.0, 240.0, 1.0);
        state.rotation = std::f32::consts::FRAC_PI_4;
        draw_sprite(&mut canvas, &texture, &state, Origin::Centre, &view, 1.0);
        assert_ne!(canvas.get(240, 240), [0, 0, 0, 255], "旋转后中心仍有内容");
    }

    /// 元素辅助函数保底可编译（绘制路径使用同一套元素类型）。
    #[test]
    fn element_helpers_stay_consistent() {
        let element = Element {
            kind: ElementKind::Animation {
                frame_count: 1,
                frame_delay_ms: 100.0,
                loop_type: AnimationLoop::LoopForever,
            },
            layer: Layer::Foreground,
            path: "a.png".to_string(),
            origin: Origin::Centre,
            x: 0.0,
            y: 0.0,
            commands: ElementCommands::default(),
        };
        assert_eq!(element.texture_path_at(0), "a0.png");
    }
}
