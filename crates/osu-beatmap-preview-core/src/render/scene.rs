//! 与具体后端无关的有序逐帧场景描述。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::render::canvas::{Img, Rgba};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// 带组合变换的贴图绘制参数：锚点 + 尺寸 + 旋转 + 颜色 + 混合模式。
///
/// 与 [`DrawCommand::TransformedSprite`] 的几何字段一一对应，打包成结构体，
/// 避免 [`FrameSceneBuilder::transformed_sprite`] 一长串参数传错位。
#[derive(Debug, Clone, Copy)]
pub struct SpriteSpec {
    /// 锚点位置（画布坐标）。
    pub position: [f32; 2],
    /// 原点在图像中的归一化偏移（0 / 0.5 / 1，已按翻转调整）。
    pub origin: [f32; 2],
    /// 缩放后的目标尺寸；负值表示翻转。
    pub size: [f32; 2],
    /// 顺时针旋转弧度。
    pub rotation: f32,
    /// 颜色调制（RGB）与不透明度（A）。
    pub color: Rgba,
    /// 加色混合（osu! 的 `P,,A`）。
    pub additive: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum DrawCommand {
    PushClip(SceneRect),
    PopClip,
    Sprite {
        resource: ResourceId,
        destination: SceneRect,
        alpha: f32,
    },
    /// 变换精灵：支持旋转、缩放/翻转、颜色调制与混合模式的贴图绘制。
    ///
    /// 与 [`DrawCommand::Sprite`] 的轴对齐矩形不同，本命令按「锚点 + 尺寸 +
    /// 旋转」描述几何，供故事板这类带组合变换的贴图使用；CPU 与 WGPU 两条
    /// 光栅路径按同一套几何语义实现。
    TransformedSprite {
        resource: ResourceId,
        /// 锚点位置（画布坐标）。
        position: [f32; 2],
        /// 原点在图像中的归一化偏移（0 / 0.5 / 1，已按翻转调整）。
        origin: [f32; 2],
        /// 缩放后的目标尺寸；负值表示翻转。
        size: [f32; 2],
        /// 顺时针旋转弧度。
        rotation: f32,
        /// 颜色调制（RGB）与不透明度（A）。
        color: Rgba,
        /// 加色混合（osu! 的 `P,,A`）。
        additive: bool,
    },
    /// 外部纹理精灵：纹理内容由渲染后端从外部源（如浏览器视频帧）直接拷入，
    /// 场景里只带槽位号——逐帧视频像素不进 CPU 内存。
    ExternalSprite {
        slot: u32,
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

/// 场景结构在各 crate 之间共享，调用方应优先把它交给渲染后端，避免自行改写命令顺序。
#[derive(Debug, Clone)]
pub struct FrameScene {
    pub size: SceneSize,
    pub absolute_time_ms: i64,
    pub commands: Arc<[DrawCommand]>,
    pub resources: Arc<BTreeMap<ResourceId, Arc<Img>>>,
}

pub struct FrameSceneBuilder {
    size: SceneSize,
    absolute_time_ms: i64,
    commands: Vec<DrawCommand>,
    resources: BTreeMap<ResourceId, Arc<Img>>,
    next_resource: u64,
}

impl FrameScene {
    pub fn new(size: SceneSize, absolute_time_ms: i64, commands: Vec<DrawCommand>) -> Self {
        Self {
            size,
            absolute_time_ms,
            commands: commands.into(),
            resources: Arc::new(BTreeMap::new()),
        }
    }

    pub fn clear(size: SceneSize, absolute_time_ms: i64, color: Rgba) -> Self {
        Self::new(
            size,
            absolute_time_ms,
            vec![DrawCommand::Rectangle {
                rect: SceneRect {
                    x: 0.0,
                    y: 0.0,
                    width: size.width as f32,
                    height: size.height as f32,
                },
                color,
            }],
        )
    }

    pub fn width(&self) -> u32 {
        self.size.width
    }

    pub fn height(&self) -> u32 {
        self.size.height
    }

    pub fn absolute_time_ms(&self) -> i64 {
        self.absolute_time_ms
    }

    /// 使用单张图像构造场景，适合宿主测试或简单的图像管线。
    pub fn from_image(image: Img, absolute_time_ms: i64) -> Self {
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
    pub fn new(width: u32, height: u32, absolute_time_ms: i64) -> Self {
        Self {
            size: SceneSize { width, height },
            absolute_time_ms,
            commands: Vec::new(),
            resources: BTreeMap::new(),
            next_resource: 0,
        }
    }

    /// 画布宽度；绘制阶段需要据此把越界矩形夹回场景内。
    pub fn width(&self) -> u32 {
        self.size.width
    }

    /// 画布高度；与 [`Self::width`] 一起用于裁剪越界绘制。
    pub fn height(&self) -> u32 {
        self.size.height
    }

    pub fn rectangle(&mut self, rect: SceneRect, color: Rgba) {
        self.commands.push(DrawCommand::Rectangle { rect, color });
    }

    pub fn push_clip(&mut self, rect: SceneRect) {
        self.commands.push(DrawCommand::PushClip(rect));
    }

    pub fn pop_clip(&mut self) {
        self.commands.push(DrawCommand::PopClip);
    }

    pub fn circle(&mut self, center: [f32; 2], radius: f32, color: Rgba) {
        self.commands.push(DrawCommand::Circle {
            center,
            radius,
            color,
        });
    }

    pub fn ring(&mut self, center: [f32; 2], radius: f32, thickness: f32, color: Rgba) {
        self.commands.push(DrawCommand::Ring {
            center,
            radius,
            thickness,
            color,
        });
    }

    pub fn line(&mut self, from: [f32; 2], to: [f32; 2], thickness: f32, color: Rgba) {
        self.commands.push(DrawCommand::Line {
            from,
            to,
            thickness,
            color,
        });
    }

    pub fn sprite(&mut self, image: Arc<Img>, destination: SceneRect, alpha: f32) {
        let resource = self.insert_resource(image);
        self.commands.push(DrawCommand::Sprite {
            resource,
            destination,
            alpha,
        });
    }

    /// 绘制带组合变换的贴图（旋转、缩放/翻转、颜色调制、加色混合）。
    pub fn transformed_sprite(&mut self, image: Arc<Img>, spec: SpriteSpec) {
        let resource = self.insert_resource(image);
        self.commands.push(DrawCommand::TransformedSprite {
            resource,
            position: spec.position,
            origin: spec.origin,
            size: spec.size,
            rotation: spec.rotation,
            color: spec.color,
            additive: spec.additive,
        });
    }

    /// 绘制外部纹理槽位（如浏览器视频帧）；纹理由渲染后端按槽位号填充。
    pub fn external_sprite(&mut self, slot: u32, destination: SceneRect, alpha: f32) {
        self.commands.push(DrawCommand::ExternalSprite {
            slot,
            destination,
            alpha,
        });
    }

    pub fn glyph(&mut self, image: Arc<Img>, destination: SceneRect, color: Rgba) {
        let resource = self.insert_resource(image);
        self.commands.push(DrawCommand::Glyph {
            resource,
            destination,
            color,
        });
    }

    pub fn slider_mesh(
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

    pub fn append(&mut self, scene: &FrameScene, offset: [f32; 2]) {
        self.append_scaled(scene, offset, 1.0);
    }

    pub fn append_scaled(&mut self, scene: &FrameScene, offset: [f32; 2], scale: f32) {
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

    pub fn finish(self) -> FrameScene {
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
        // 均匀缩放不改变旋转角与原点比例；尺寸按同样倍率缩放（负值=翻转保留符号）。
        DrawCommand::TransformedSprite {
            resource,
            position,
            origin,
            size,
            rotation,
            color,
            additive,
        } => DrawCommand::TransformedSprite {
            resource: resources[resource],
            position: point(*position),
            origin: *origin,
            size: [size[0] * scale, size[1] * scale],
            rotation: *rotation,
            color: *color,
            additive: *additive,
        },
        DrawCommand::ExternalSprite {
            slot,
            destination,
            alpha,
        } => DrawCommand::ExternalSprite {
            slot: *slot,
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

    /// 图像场景使用稳定资源编号和有序命令。
    #[test]
    fn image_scene_uses_stable_ids_and_ordered_commands() {
        let scene = FrameScene::from_image(Img::new(2, 3, [1, 2, 3, 4]), -50);
        assert_eq!((scene.width(), scene.height()), (2, 3));
        assert_eq!(scene.absolute_time_ms(), -50);
        assert_eq!(scene.resources.len(), 1);
        assert_eq!(scene.commands.len(), 1);
    }

    /// 场景合并会平移命令并重新编号资源。
    #[test]
    fn scene_merge_translates_commands_and_renumbers_resources() {
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
