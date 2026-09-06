//! 与具体后端无关的有序逐帧场景描述。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::render::canvas::{Img, Rgba};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SceneSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ResourceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SceneRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum DrawCommand {
    PushClip(SceneRect),
    PopClip,
    Sprite {
        resource: ResourceId,
        destination: SceneRect,
        alpha: f32,
    },
    Rectangle {
        rect: SceneRect,
        color: Rgba,
    },
    Circle {
        center: [f32; 2],
        radius: f32,
        color: Rgba,
    },
    Ring {
        center: [f32; 2],
        radius: f32,
        thickness: f32,
        color: Rgba,
    },
    Line {
        from: [f32; 2],
        to: [f32; 2],
        thickness: f32,
        color: Rgba,
    },
    Glyph {
        resource: ResourceId,
        destination: SceneRect,
        color: Rgba,
    },
    SliderMesh {
        vertices: Arc<[[f32; 2]]>,
        thickness: f32,
        border: Rgba,
        body: Rgba,
    },
}

/// 字段保持私有，调用方只能把场景交给渲染后端或读取稳定元数据。
#[derive(Debug, Clone)]
pub struct FrameScene {
    pub(crate) size: SceneSize,
    pub(crate) absolute_time_ms: i64,
    pub(crate) commands: Arc<[DrawCommand]>,
    pub(crate) resources: Arc<BTreeMap<ResourceId, Arc<Img>>>,
}

pub(crate) struct FrameSceneBuilder {
    size: SceneSize,
    absolute_time_ms: i64,
    commands: Vec<DrawCommand>,
    resources: BTreeMap<ResourceId, Arc<Img>>,
    next_resource: u64,
}

impl FrameScene {
    pub fn width(&self) -> u32 {
        self.size.width
    }

    pub fn height(&self) -> u32 {
        self.size.height
    }

    pub fn absolute_time_ms(&self) -> i64 {
        self.absolute_time_ms
    }

    #[cfg(test)]
    pub(crate) fn from_image(image: Img, absolute_time_ms: i64) -> Self {
        let size = SceneSize {
            width: image.w,
            height: image.h,
        };
        let resource = ResourceId(0);
        let mut resources = BTreeMap::new();
        resources.insert(resource, Arc::new(image));
        Self {
            size,
            absolute_time_ms,
            commands: Arc::new([DrawCommand::Sprite {
                resource,
                destination: SceneRect {
                    x: 0.0,
                    y: 0.0,
                    width: size.width as f32,
                    height: size.height as f32,
                },
                alpha: 1.0,
            }]),
            resources: Arc::new(resources),
        }
    }
}

#[allow(dead_code)]
impl FrameSceneBuilder {
    pub(crate) fn new(width: u32, height: u32, absolute_time_ms: i64) -> Self {
        Self {
            size: SceneSize { width, height },
            absolute_time_ms,
            commands: Vec::new(),
            resources: BTreeMap::new(),
            next_resource: 0,
        }
    }

    pub(crate) fn rectangle(&mut self, rect: SceneRect, color: Rgba) {
        self.commands.push(DrawCommand::Rectangle { rect, color });
    }

    pub(crate) fn push_clip(&mut self, rect: SceneRect) {
        self.commands.push(DrawCommand::PushClip(rect));
    }

    pub(crate) fn pop_clip(&mut self) {
        self.commands.push(DrawCommand::PopClip);
    }

    pub(crate) fn circle(&mut self, center: [f32; 2], radius: f32, color: Rgba) {
        self.commands.push(DrawCommand::Circle {
            center,
            radius,
            color,
        });
    }

    pub(crate) fn ring(&mut self, center: [f32; 2], radius: f32, thickness: f32, color: Rgba) {
        self.commands.push(DrawCommand::Ring {
            center,
            radius,
            thickness,
            color,
        });
    }

    pub(crate) fn line(&mut self, from: [f32; 2], to: [f32; 2], thickness: f32, color: Rgba) {
        self.commands.push(DrawCommand::Line {
            from,
            to,
            thickness,
            color,
        });
    }

    pub(crate) fn sprite(&mut self, image: Arc<Img>, destination: SceneRect, alpha: f32) {
        let resource = self.insert_resource(image);
        self.commands.push(DrawCommand::Sprite {
            resource,
            destination,
            alpha,
        });
    }

    pub(crate) fn glyph(&mut self, image: Arc<Img>, destination: SceneRect, color: Rgba) {
        let resource = self.insert_resource(image);
        self.commands.push(DrawCommand::Glyph {
            resource,
            destination,
            color,
        });
    }

    pub(crate) fn slider_mesh(
        &mut self,
        vertices: Arc<[[f32; 2]]>,
        thickness: f32,
        border: Rgba,
        body: Rgba,
    ) {
        self.commands.push(DrawCommand::SliderMesh {
            vertices,
            thickness,
            border,
            body,
        });
    }

    pub(crate) fn append(&mut self, scene: &FrameScene, offset: [f32; 2]) {
        self.append_scaled(scene, offset, 1.0);
    }

    pub(crate) fn append_scaled(&mut self, scene: &FrameScene, offset: [f32; 2], scale: f32) {
        assert!(scale.is_finite() && scale > 0.0, "场景缩放必须为正有限数");
        let remapped = scene
            .resources
            .iter()
            .map(|(&source, image)| (source, self.insert_resource(Arc::clone(image))))
            .collect::<BTreeMap<_, _>>();
        for command in scene.commands.iter() {
            self.commands
                .push(transform_command(command, &remapped, offset, scale));
        }
    }

    pub(crate) fn finish(self) -> FrameScene {
        FrameScene {
            size: self.size,
            absolute_time_ms: self.absolute_time_ms,
            commands: self.commands.into(),
            resources: Arc::new(self.resources),
        }
    }

    fn insert_resource(&mut self, image: Arc<Img>) -> ResourceId {
        let resource = ResourceId(self.next_resource);
        self.next_resource = self
            .next_resource
            .checked_add(1)
            .expect("单帧资源编号不会耗尽 u64");
        self.resources.insert(resource, image);
        resource
    }
}

fn transform_command(
    command: &DrawCommand,
    resources: &BTreeMap<ResourceId, ResourceId>,
    offset: [f32; 2],
    scale: f32,
) -> DrawCommand {
    let rect = |rect: SceneRect| SceneRect {
        x: rect.x * scale + offset[0],
        y: rect.y * scale + offset[1],
        width: rect.width * scale,
        height: rect.height * scale,
    };
    let point = |point: [f32; 2]| [point[0] * scale + offset[0], point[1] * scale + offset[1]];
    match command {
        DrawCommand::PushClip(value) => DrawCommand::PushClip(rect(*value)),
        DrawCommand::PopClip => DrawCommand::PopClip,
        DrawCommand::Sprite {
            resource,
            destination,
            alpha,
        } => DrawCommand::Sprite {
            resource: resources[resource],
            destination: rect(*destination),
            alpha: *alpha,
        },
        DrawCommand::Rectangle { rect: value, color } => DrawCommand::Rectangle {
            rect: rect(*value),
            color: *color,
        },
        DrawCommand::Circle {
            center,
            radius,
            color,
        } => DrawCommand::Circle {
            center: point(*center),
            radius: *radius * scale,
            color: *color,
        },
        DrawCommand::Ring {
            center,
            radius,
            thickness,
            color,
        } => DrawCommand::Ring {
            center: point(*center),
            radius: *radius * scale,
            thickness: *thickness * scale,
            color: *color,
        },
        DrawCommand::Line {
            from,
            to,
            thickness,
            color,
        } => DrawCommand::Line {
            from: point(*from),
            to: point(*to),
            thickness: *thickness * scale,
            color: *color,
        },
        DrawCommand::Glyph {
            resource,
            destination,
            color,
        } => DrawCommand::Glyph {
            resource: resources[resource],
            destination: rect(*destination),
            color: *color,
        },
        DrawCommand::SliderMesh {
            vertices,
            thickness,
            border,
            body,
        } => DrawCommand::SliderMesh {
            vertices: vertices
                .iter()
                .map(|&vertex| point(vertex))
                .collect::<Vec<_>>()
                .into(),
            thickness: *thickness * scale,
            border: *border,
            body: *body,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 图像场景使用稳定资源编号和有序命令() {
        let scene = FrameScene::from_image(Img::new(2, 3, [1, 2, 3, 4]), -50);
        assert_eq!((scene.width(), scene.height()), (2, 3));
        assert_eq!(scene.absolute_time_ms(), -50);
        assert_eq!(scene.resources.len(), 1);
        assert_eq!(scene.commands.len(), 1);
    }

    #[test]
    fn 场景合并会平移命令并重新编号资源() {
        let source = FrameScene::from_image(Img::new(2, 3, [1, 2, 3, 4]), 10);
        let mut builder = FrameSceneBuilder::new(10, 10, 10);
        builder.rectangle(
            SceneRect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            [0, 0, 0, 255],
        );
        builder.append(&source, [4.0, 5.0]);
        let scene = builder.finish();
        assert_eq!(scene.resources.len(), 1);
        let DrawCommand::Sprite { destination, .. } = &scene.commands[1] else {
            panic!("第二条命令必须是平移后的精灵");
        };
        assert_eq!((destination.x, destination.y), (4.0, 5.0));
    }
}
