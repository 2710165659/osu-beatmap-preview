//! 回放叠加层共享绘制契约：CPU 合成像素，WebGPU 发出小精灵与几何命令。
use crate::render::{
    canvas::Img,
    scene::{FrameSceneBuilder, SceneRect, SpriteSpec},
};
use std::sync::Arc;

pub(super) trait OverlayCanvas {
    fn alpha_composite(&mut self, image: &Img, x: i64, y: i64);
    fn shared_sprite(&mut self, image: &Arc<Img>, x: i64, y: i64) {
        self.alpha_composite(image, x, y);
    }
    fn fill_rect_size(&mut self, x: i64, y: i64, w: i64, h: i64, color: [u8; 4]);
    fn draw_line(&mut self, x: f64, y: f64, tx: f64, ty: f64, width: f64, color: [u8; 4]);
    fn additive(&mut self, image: &Img, x: i64, y: i64, alpha: f64);
}
impl OverlayCanvas for Img {
    fn alpha_composite(&mut self, image: &Img, x: i64, y: i64) {
        Img::alpha_composite(self, image, x, y);
    }
    fn fill_rect_size(&mut self, x: i64, y: i64, w: i64, h: i64, color: [u8; 4]) {
        Img::fill_rect_size(self, x, y, w, h, color);
    }
    fn draw_line(&mut self, x: f64, y: f64, tx: f64, ty: f64, width: f64, color: [u8; 4]) {
        Img::draw_line(self, x, y, tx, ty, width, color);
    }
    fn additive(&mut self, image: &Img, x: i64, y: i64, alpha: f64) {
        super::blend::composite(self, image, x, y, alpha);
    }
}
impl OverlayCanvas for FrameSceneBuilder {
    fn shared_sprite(&mut self, image: &Arc<Img>, x: i64, y: i64) {
        self.sprite(
            Arc::clone(image),
            SceneRect {
                x: x as f32,
                y: y as f32,
                width: image.w as f32,
                height: image.h as f32,
            },
            1.0,
        );
    }
    fn alpha_composite(&mut self, image: &Img, x: i64, y: i64) {
        self.sprite(
            Arc::new(image.clone()),
            SceneRect {
                x: x as f32,
                y: y as f32,
                width: image.w as f32,
                height: image.h as f32,
            },
            1.0,
        );
    }
    fn fill_rect_size(&mut self, x: i64, y: i64, w: i64, h: i64, color: [u8; 4]) {
        self.rectangle(
            SceneRect {
                x: x as f32,
                y: y as f32,
                width: w as f32,
                height: h as f32,
            },
            color,
        );
    }
    fn draw_line(&mut self, x: f64, y: f64, tx: f64, ty: f64, width: f64, color: [u8; 4]) {
        self.line(
            [x as f32, y as f32],
            [tx as f32, ty as f32],
            width as f32,
            color,
        );
    }
    fn additive(&mut self, image: &Img, x: i64, y: i64, alpha: f64) {
        self.transformed_sprite(
            Arc::new(image.clone()),
            SpriteSpec {
                position: [x as f32, y as f32],
                origin: [0.0; 2],
                size: [image.w as f32, image.h as f32],
                rotation: 0.0,
                color: [255, 255, 255, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8],
                additive: true,
            },
        );
    }
}
