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
        let color = crate::infrastructure::config::current().skin.HYPER_DASH;
        image.stroke_circle_aa(cx, cy, radius * 1.6, radius * 0.6, rgba(color));
    }
    image.fill_circle_aa(cx, cy, radius, rgba(color));
    image.stroke_circle_aa(cx, cy, radius, radius * 0.2, WHITE);
}

fn draw_classic_banana(image: &mut Img, cx: f64, cy: f64, diameter: f64, color: [u8; 3]) {
    let radius = diameter / 2.0;
    image.stroke_circle_aa(cx, cy, radius, radius * 0.2, rgba(color));
}

pub(crate) fn draw_catch_object(
    image: &mut Img,
    object: &RenderObject,
    cx: f64,
    cy: f64,
    diameter: f64,
) {
    match object.object_type {
        ObjType::Fruit | ObjType::Droplet | ObjType::TinyDroplet => {
            draw_classic_fruit(image, cx, cy, diameter, object.color, object.hyper_dash)
        }
        ObjType::Banana => draw_classic_banana(image, cx, cy, diameter, object.color),
    }
}

/// 应用难度、物件种类及游戏区域缩放后的物件直径。
pub(crate) fn object_diameter(object_scale: f64, playfield_scale: f64, scale_factor: f64) -> f64 {
    super::constants::OBJECT_RADIUS * 2.0 * object_scale * scale_factor * playfield_scale
}

#[cfg(test)]
mod tests {
    use super::*;

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
