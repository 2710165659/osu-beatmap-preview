//! 场景的 CPU 参考光栅器，复用现有 `Img` 绘图原语。

use crate::domain::errors::{PreviewError, Result};
use crate::render::canvas::Img;
use crate::render::scene::{DrawCommand, FrameScene, SceneRect};

pub(crate) trait FrameBackend {
    fn render_frame(&mut self, scene: &FrameScene) -> Result<Img>;
}

#[derive(Default)]
pub(crate) struct CpuRasterizer;

impl FrameBackend for CpuRasterizer {
    fn render_frame(&mut self, scene: &FrameScene) -> Result<Img> {
        let mut target = Img::new(scene.size.width, scene.size.height, [0, 0, 0, 0]);
        let mut clips = vec![SceneRect {
            x: 0.0,
            y: 0.0,
            width: scene.size.width as f32,
            height: scene.size.height as f32,
        }];
        for command in scene.commands.iter() {
            match command {
                DrawCommand::PushClip(rect) => {
                    let parent = clips.last().copied().expect("裁剪栈始终保留画布边界");
                    clips.push(intersection(parent, *rect));
                }
                DrawCommand::PopClip => {
                    if clips.len() == 1 {
                        return Err(PreviewError::render("frame scene clip stack underflow"));
                    }
                    clips.pop();
                }
                DrawCommand::Sprite {
                    resource,
                    destination,
                    alpha,
                } => {
                    let source = scene.resources.get(resource).ok_or_else(|| {
                        PreviewError::render(format!(
                            "frame scene resource {} is missing",
                            resource.0
                        ))
                    })?;
                    let width = destination.width.round().max(1.0) as u32;
                    let height = destination.height.round().max(1.0) as u32;
                    let image = if source.w == width && source.h == height {
                        source.as_ref().clone()
                    } else {
                        source.resize(width, height)
                    };
                    let image = if *alpha < 1.0 {
                        image.scale_alpha((*alpha as f64).clamp(0.0, 1.0))
                    } else {
                        image
                    };
                    composite_clipped(
                        &mut target,
                        &image,
                        destination.x.round() as i64,
                        destination.y.round() as i64,
                        *clips.last().expect("裁剪栈始终非空"),
                    );
                }
                DrawCommand::Rectangle { rect, color } => {
                    let rect = intersection(*clips.last().expect("裁剪栈始终非空"), *rect);
                    target.fill_rect_size(
                        rect.x.round() as i64,
                        rect.y.round() as i64,
                        rect.width.round() as i64,
                        rect.height.round() as i64,
                        *color,
                    );
                }
                DrawCommand::Circle {
                    center,
                    radius,
                    color,
                } => target.fill_circle_aa(
                    center[0] as f64,
                    center[1] as f64,
                    *radius as f64,
                    *color,
                ),
                DrawCommand::Ring {
                    center,
                    radius,
                    thickness,
                    color,
                } => target.stroke_circle_aa(
                    center[0] as f64,
                    center[1] as f64,
                    *radius as f64,
                    *thickness as f64,
                    *color,
                ),
                DrawCommand::Line {
                    from,
                    to,
                    thickness,
                    color,
                } => target.draw_line(
                    from[0] as f64,
                    from[1] as f64,
                    to[0] as f64,
                    to[1] as f64,
                    *thickness as f64,
                    *color,
                ),
                DrawCommand::Glyph {
                    resource,
                    destination,
                    color,
                } => {
                    let source = scene.resources.get(resource).ok_or_else(|| {
                        PreviewError::render(format!("frame scene glyph {} is missing", resource.0))
                    })?;
                    let mut glyph = source.as_ref().clone();
                    for pixel in glyph.data.chunks_exact_mut(4) {
                        pixel[0] = color[0];
                        pixel[1] = color[1];
                        pixel[2] = color[2];
                        pixel[3] = ((pixel[3] as u16 * color[3] as u16) / 255) as u8;
                    }
                    composite_clipped(
                        &mut target,
                        &glyph,
                        destination.x.round() as i64,
                        destination.y.round() as i64,
                        *clips.last().expect("裁剪栈始终非空"),
                    );
                }
                DrawCommand::SliderMesh {
                    vertices,
                    thickness,
                    border,
                    body,
                } => {
                    for points in vertices.windows(2) {
                        target.draw_line(
                            points[0][0] as f64,
                            points[0][1] as f64,
                            points[1][0] as f64,
                            points[1][1] as f64,
                            *thickness as f64,
                            *border,
                        );
                        target.draw_line(
                            points[0][0] as f64,
                            points[0][1] as f64,
                            points[1][0] as f64,
                            points[1][1] as f64,
                            (*thickness * 0.72) as f64,
                            *body,
                        );
                    }
                }
            }
        }
        if clips.len() != 1 {
            return Err(PreviewError::render("frame scene clip stack is unbalanced"));
        }
        Ok(target)
    }
}

fn intersection(left: SceneRect, right: SceneRect) -> SceneRect {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.width).min(right.x + right.width);
    let bottom_edge = (left.y + left.height).min(right.y + right.height);
    SceneRect {
        x,
        y,
        width: (right_edge - x).max(0.0),
        height: (bottom_edge - y).max(0.0),
    }
}

fn composite_clipped(target: &mut Img, source: &Img, x: i64, y: i64, clip: SceneRect) {
    let clip_x = clip.x.round().max(0.0) as i64;
    let clip_y = clip.y.round().max(0.0) as i64;
    let clip_right = (clip.x + clip.width).round().min(target.w as f32) as i64;
    let clip_bottom = (clip.y + clip.height).round().min(target.h as f32) as i64;
    let left = x.max(clip_x);
    let top = y.max(clip_y);
    let right = (x + source.w as i64).min(clip_right);
    let bottom = (y + source.h as i64).min(clip_bottom);
    if left >= right || top >= bottom {
        return;
    }
    let cropped = source.crop(
        (left - x) as u32,
        (top - y) as u32,
        (right - x) as u32,
        (bottom - y) as u32,
    );
    target.alpha_composite(&cropped, left, top);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::scene::FrameScene;

    #[test]
    fn 图像场景经过参考后端保持像素不变() {
        let source = Img::new(3, 2, [10, 20, 30, 128]);
        let scene = FrameScene::from_image(source.clone(), 0);
        let rendered = CpuRasterizer.render_frame(&scene).unwrap();
        assert_eq!(rendered.data, source.data);
    }
}
