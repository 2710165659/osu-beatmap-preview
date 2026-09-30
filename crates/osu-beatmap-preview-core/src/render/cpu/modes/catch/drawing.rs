//! PNG、GIF 与 MP4 共用的经典 osu!catch 物件绘制。
//!
//! 这里保持 nonebot-plugin-osubot 既有预览样式：水果与水滴使用带白色
//! 内边框的实心圆，hyperdash 使用彩色外环，香蕉使用空心圆环。

use crate::render::canvas::Img;

use super::objects::{ObjType, RenderObject};

const WHITE: [u8; 4] = [255, 255, 255, 255];

fn rgba(color: [u8; 3]) -> [u8; 4] {
    [color[0], color[1], color[2], 255]
}

fn draw_classic_fruit(
    image: &mut Img,
    cx: f64,
    cy: f64,
    diameter: f64,
    color: [u8; 3],
    hyper_dash: bool,
) {
    let radius = diameter / 2.0;
    if hyper_dash {
        let color = crate::config::current().skin.HYPER_DASH;
        image.stroke_circle_aa(cx, cy, radius * 1.6, radius * 0.6, rgba(color));
    }
    image.fill_circle_aa(cx, cy, radius, rgba(color));
    image.stroke_circle_aa(cx, cy, radius, radius * 0.2, WHITE);
}

fn draw_classic_banana(image: &mut Img, cx: f64, cy: f64, diameter: f64, color: [u8; 3]) {
    let radius = diameter / 2.0;
    image.stroke_circle_aa(cx, cy, radius, radius * 0.2, rgba(color));
}

pub fn draw_catch_object(image: &mut Img, object: &RenderObject, cx: f64, cy: f64, diameter: f64) {
    match object.object_type {
        ObjType::Fruit | ObjType::Droplet | ObjType::TinyDroplet => {
            draw_classic_fruit(image, cx, cy, diameter, object.color, object.hyper_dash)
        }
        ObjType::Banana => draw_classic_banana(image, cx, cy, diameter, object.color),
    }
}

/// 整个物件只应用一次透明度，避免白色边框与本体重叠时叠加 alpha。
pub(crate) fn draw_catch_object_with_alpha(
    image: &mut Img,
    object: &RenderObject,
    cx: f64,
    cy: f64,
    diameter: f64,
    alpha: u8,
) {
    if alpha == 0 {
        return;
    }
    if alpha == 255 {
        draw_catch_object(image, object, cx, cy, diameter);
        return;
    }
    // hyperdash 外环的外半径是本体的 1.6 倍，再留出抗锯齿边缘。
    let radius = diameter / 2.0 * if object.hyper_dash { 1.6 } else { 1.0 };
    let left = (cx - radius - 1.0).floor() as i64;
    let top = (cy - radius - 1.0).floor() as i64;
    let right = (cx + radius + 1.0).ceil() as i64;
    let bottom = (cy + radius + 1.0).ceil() as i64;
    let mut sprite = Img::new((right - left) as u32, (bottom - top) as u32, [0, 0, 0, 0]);
    draw_catch_object(
        &mut sprite,
        object,
        cx - left as f64,
        cy - top as f64,
        diameter,
    );
    image.alpha_composite_scaled(&sprite, left, top, f64::from(alpha) / 255.0);
}

/// 应用难度、物件种类及游戏区域缩放后的物件直径。
pub fn object_diameter(object_scale: f64, playfield_scale: f64, scale_factor: f64) -> f64 {
    super::constants::OBJECT_RADIUS * 2.0 * object_scale * scale_factor * playfield_scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hyperdash_outline_is_red_before_and_during_hidden_fade() {
        let mut object =
            super::super::objects::build_fruit_object(256.0, 1000, [30, 120, 220], None);
        object.hyper_dash = true;
        for alpha in [255, 128] {
            let mut image = Img::new(96, 96, [0, 0, 0, 0]);
            draw_catch_object_with_alpha(&mut image, &object, 48.0, 48.0, 32.0, alpha);
            assert_eq!(image.get(72, 48), [255, 0, 0, alpha]);
            assert_eq!(image.get(48, 48), [30, 120, 220, alpha]);
        }
    }

    #[test]
    fn custom_hyperdash_color_is_preserved_during_hidden_fade() {
        let mut config = crate::config::CoreConfig::default();
        config.skin.HYPER_DASH = [255, 82, 139];
        crate::config::with_config(std::sync::Arc::new(config), || {
            let mut object =
                super::super::objects::build_fruit_object(256.0, 1000, [30, 120, 220], None);
            object.hyper_dash = true;
            let mut image = Img::new(96, 96, [0, 0, 0, 0]);
            draw_catch_object_with_alpha(&mut image, &object, 48.0, 48.0, 32.0, 128);
            assert_eq!(image.get(72, 48), [255, 82, 139, 128]);
            assert_eq!(image.get(48, 48), [30, 120, 220, 128]);
        });
    }

    #[test]
    fn fading_applies_one_alpha_to_the_fruit_border_and_hyperdash_ring() {
        let mut object =
            super::super::objects::build_fruit_object(256.0, 1000, [30, 120, 220], None);
        object.hyper_dash = true;
        let mut image = Img::new(96, 96, [0, 0, 0, 0]);
        draw_catch_object_with_alpha(&mut image, &object, 48.0, 48.0, 32.0, 128);
        assert_eq!(image.get(48, 48), [30, 120, 220, 128]);
        assert_eq!(image.get(62, 48), [255, 255, 255, 128]);
        assert_eq!(image.get(72, 48)[3], 128);

        let mut absent = Img::new(96, 96, [0, 0, 0, 0]);
        draw_catch_object_with_alpha(&mut absent, &object, 48.0, 48.0, 32.0, 0);
        assert!(absent.alpha_bbox().is_none());

        let mut opaque = Img::new(96, 96, [0, 0, 0, 0]);
        let mut normal = Img::new(96, 96, [0, 0, 0, 0]);
        draw_catch_object_with_alpha(&mut opaque, &object, 48.0, 48.0, 32.0, 255);
        draw_catch_object(&mut normal, &object, 48.0, 48.0, 32.0);
        assert!(opaque.data == normal.data);
    }

    #[test]
    fn classic_fruit_is_circular_with_a_white_border() {
        let mut image = Img::new(96, 96, [0, 0, 0, 0]);
        draw_classic_fruit(&mut image, 48.0, 48.0, 64.0, [30, 120, 220], false);

        assert_eq!(image.get(48, 48), [30, 120, 220, 255]);
        assert_eq!(image.get(48, 19), image.get(19, 48));
        assert_eq!(image.get(48, 19)[0..3], [255, 255, 255]);
        assert_eq!(image.get(48, 10)[3], 0);
    }

    #[test]
    fn classic_banana_keeps_its_center_transparent() {
        let mut image = Img::new(96, 96, [0, 0, 0, 0]);
        draw_classic_banana(&mut image, 48.0, 48.0, 64.0, [255, 255, 255]);

        assert_eq!(image.get(48, 48)[3], 0);
        assert!(image.get(48, 19)[3] > 0);
    }
}
