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
    /// 填充矩形；半透明颜色按 src-over 混合。
    fn fill_rect_size(&mut self, x: i64, y: i64, w: i64, h: i64, color: [u8; 4]);
    /// 填充圆盘；供偏移条的中心标记和反馈效果共用。
    fn blend_circle(&mut self, cx: f64, cy: f64, radius: f64, color: [u8; 4]);
    /// 混合绘制单段直线，用于判定条的刻度与平均偏移标记。
    fn blend_line(&mut self, x: f64, y: f64, tx: f64, ty: f64, width: f64, color: [u8; 4]);
    fn additive(&mut self, image: &Img, x: i64, y: i64, alpha: f64);
    fn additive_transformed(
        &mut self,
        image: &Arc<Img>,
        center: [f64; 2],
        scale: f64,
        rotation: f64,
        alpha: f64,
    ) {
        if alpha <= 0.0 {
            return;
        }
        if scale == 1.0 && rotation == 0.0 {
            self.additive(
                image,
                (center[0] - image.w as f64 / 2.0).round() as i64,
                (center[1] - image.h as f64 / 2.0).round() as i64,
                alpha,
            );
            return;
        }
        let mut sprite = image.resize_bilinear(
            (image.w as f64 * scale).round().max(1.0) as u32,
            (image.h as f64 * scale).round().max(1.0) as u32,
        );
        // CPU 图像旋转方向与游戏坐标相反；GPU 则直接使用顺时针弧度。
        if rotation != 0.0 {
            sprite = sprite.rotate_expand(-rotation);
        }
        self.additive(
            &sprite,
            (center[0] - sprite.w as f64 / 2.0).round() as i64,
            (center[1] - sprite.h as f64 / 2.0).round() as i64,
            alpha,
        );
    }
}
impl OverlayCanvas for Img {
    fn alpha_composite(&mut self, image: &Img, x: i64, y: i64) {
        Img::alpha_composite(self, image, x, y);
    }
    fn fill_rect_size(&mut self, x: i64, y: i64, w: i64, h: i64, color: [u8; 4]) {
        Img::fill_rect_size(self, x, y, w, h, color);
    }
    fn blend_circle(&mut self, cx: f64, cy: f64, radius: f64, color: [u8; 4]) {
        Img::fill_circle_aa(self, cx, cy, radius, color);
    }
    fn blend_line(&mut self, x: f64, y: f64, tx: f64, ty: f64, width: f64, color: [u8; 4]) {
        Img::blend_line(self, x, y, tx, ty, width, color);
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
    fn blend_circle(&mut self, cx: f64, cy: f64, radius: f64, color: [u8; 4]) {
        self.circle([cx as f32, cy as f32], radius as f32, color);
    }
    fn blend_line(&mut self, x: f64, y: f64, tx: f64, ty: f64, width: f64, color: [u8; 4]) {
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

    fn additive_transformed(
        &mut self,
        image: &Arc<Img>,
        center: [f64; 2],
        scale: f64,
        rotation: f64,
        alpha: f64,
    ) {
        if alpha <= 0.0 {
            return;
        }
        // 图像身份在整个会话中保持稳定；逐帧只提交变换，不生成或上传动画位图。
        self.transformed_sprite(
            Arc::clone(image),
            SpriteSpec {
                position: [center[0] as f32, center[1] as f32],
                origin: [0.5; 2],
                size: [
                    (image.w as f64 * scale) as f32,
                    (image.h as f64 * scale) as f32,
                ],
                rotation: rotation.to_radians() as f32,
                color: [255, 255, 255, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8],
                additive: true,
            },
        );
    }
}
