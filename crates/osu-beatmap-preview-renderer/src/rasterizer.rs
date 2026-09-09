//! 将后端无关的场景命令直接光栅化到 WGPU RGBA8 离屏目标。

use std::collections::{HashMap, HashSet};
use std::f32::consts::TAU;
use std::ops::Range;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use osu_beatmap_preview_core::render::scene::ResourceId;
use osu_beatmap_preview_core::{DrawCommand, FrameScene, SceneRect};
use osu_beatmap_preview_core::{Img, Rgba};
use osu_beatmap_preview_core::{PreviewError, Result};

const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

impl GpuVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ResourceKey {
    id: ResourceId,
    identity: usize,
}

impl ResourceKey {
    fn new(id: ResourceId, image: &Arc<Img>) -> Self {
        Self {
            id,
            identity: Arc::as_ptr(image) as usize,
        }
    }
}

struct CachedTexture {
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchKind {
    Solid,
    Sprite(ResourceKey),
    Glyph(ResourceKey),
    SliderBorderCoverage,
    SliderBorderColor,
    SliderBodyCoverage,
    SliderBodyColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StencilMode {
    Keep,
    Coverage,
    Color,
}

struct DrawBatch {
    kind: BatchKind,
    scissor: [u32; 4],
    vertices: Vec<GpuVertex>,
}

struct UploadedBatch {
    kind: BatchKind,
    scissor: [u32; 4],
    vertices: Range<u32>,
}

pub(crate) struct SceneRasterizer {
    width: u32,
    height: u32,
    // 目标纹理必须与 view 同生命周期；字段本身不需要被外部读取。
    _output: wgpu::Texture,
    output_view: wgpu::TextureView,
    _composite: wgpu::Texture,
    composite_view: wgpu::TextureView,
    _msaa: Option<wgpu::Texture>,
    msaa_view: Option<wgpu::TextureView>,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    solid_pipeline: wgpu::RenderPipeline,
    sprite_pipeline: wgpu::RenderPipeline,
    glyph_pipeline: wgpu::RenderPipeline,
    slider_coverage_pipeline: wgpu::RenderPipeline,
    slider_color_pipeline: wgpu::RenderPipeline,
    resolve_pipeline: wgpu::RenderPipeline,
    resolve_bind_group: wgpu::BindGroup,
    _depth_stencil: wgpu::Texture,
    depth_stencil_view: wgpu::TextureView,
    textures: HashMap<ResourceKey, CachedTexture>,
}

impl SceneRasterizer {
    pub(crate) fn new(device: &wgpu::Device, width: u32, height: u32, sample_count: u32) -> Self {
        let output = create_target(
            device,
            "osu-beatmap-preview straight RGBA8 target",
            width,
            height,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let output_view = output.create_view(&Default::default());
        let composite = create_target(
            device,
            "osu-beatmap-preview premultiplied target",
            width,
            height,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let composite_view = composite.create_view(&Default::default());
        let msaa = (sample_count > 1).then(|| {
            create_target(
                device,
                "osu-beatmap-preview MSAA target",
                width,
                height,
                sample_count,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        });
        let msaa_view = msaa
            .as_ref()
            .map(|texture| texture.create_view(&Default::default()));
        let depth_stencil = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("osu-beatmap-preview slider stencil"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Stencil8,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_stencil_view = depth_stencil.create_view(&Default::default());
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("osu-beatmap-preview scene texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("osu-beatmap-preview scene sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("scene.wgsl"));
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("osu-beatmap-preview scene pipeline layout"),
            bind_group_layouts: &[Some(&texture_layout)],
            immediate_size: 0,
        });
        let solid_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("osu-beatmap-preview solid pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let solid_pipeline = create_scene_pipeline(
            device,
            &shader,
            &solid_layout,
            "solid_fragment",
            sample_count,
            StencilMode::Keep,
            "osu-beatmap-preview solid pipeline",
        );
        let sprite_pipeline = create_scene_pipeline(
            device,
            &shader,
            &scene_layout,
            "sprite_fragment",
            sample_count,
            StencilMode::Keep,
            "osu-beatmap-preview sprite pipeline",
        );
        let glyph_pipeline = create_scene_pipeline(
            device,
            &shader,
            &scene_layout,
            "glyph_fragment",
            sample_count,
            StencilMode::Keep,
            "osu-beatmap-preview glyph pipeline",
        );
        let slider_coverage_pipeline = create_scene_pipeline(
            device,
            &shader,
            &solid_layout,
            "solid_fragment",
            sample_count,
            StencilMode::Coverage,
            "osu-beatmap-preview slider coverage pipeline",
        );
        let slider_color_pipeline = create_scene_pipeline(
            device,
            &shader,
            &solid_layout,
            "solid_fragment",
            sample_count,
            StencilMode::Color,
            "osu-beatmap-preview slider color pipeline",
        );
        let resolve_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("osu-beatmap-preview alpha resolve layout"),
            bind_group_layouts: &[Some(&texture_layout)],
            immediate_size: 0,
        });
        let resolve_pipeline = create_resolve_pipeline(device, &shader, &resolve_layout);
        let resolve_bind_group = create_texture_bind_group(
            device,
            &texture_layout,
            &composite_view,
            &sampler,
            "osu-beatmap-preview alpha resolve bind group",
        );
        Self {
            width,
            height,
            _output: output,
            output_view,
            _composite: composite,
            composite_view,
            _msaa: msaa,
            msaa_view,
            texture_layout,
            sampler,
            solid_pipeline,
            sprite_pipeline,
            glyph_pipeline,
            slider_coverage_pipeline,
            slider_color_pipeline,
            resolve_pipeline,
            resolve_bind_group,
            _depth_stencil: depth_stencil,
            depth_stencil_view,
            textures: HashMap::new(),
        }
    }

    pub(crate) fn output_view(&self) -> &wgpu::TextureView {
        &self.output_view
    }

    pub(crate) fn output(&self) -> &wgpu::Texture {
        &self._output
    }

    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &FrameScene,
    ) -> Result<()> {
        self.prepare_textures(device, queue, scene)?;
        let batches = build_batches(scene, self.width, self.height)?;
        let (vertices, batches) = upload_batches(batches);
        let vertex_buffer = (!vertices.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("osu-beatmap-preview scene vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        {
            let (view, resolve_target, store) = if let Some(view) = &self.msaa_view {
                (view, Some(&self.composite_view), wgpu::StoreOp::Discard)
            } else {
                (&self.composite_view, None, wgpu::StoreOp::Store)
            };
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store,
                },
            })];
            let depth_stencil = Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_stencil_view,
                depth_ops: None,
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0),
                    store: wgpu::StoreOp::Store,
                }),
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("osu-beatmap-preview scene pass"),
                color_attachments: &attachments,
                depth_stencil_attachment: depth_stencil,
                ..Default::default()
            });
            if let Some(vertex_buffer) = &vertex_buffer {
                pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                for batch in batches {
                    pass.set_scissor_rect(
                        batch.scissor[0],
                        batch.scissor[1],
                        batch.scissor[2],
                        batch.scissor[3],
                    );
                    pass.set_stencil_reference(stencil_reference(batch.kind) as u32);
                    match batch.kind {
                        BatchKind::Solid => pass.set_pipeline(&self.solid_pipeline),
                        BatchKind::Sprite(key) => {
                            pass.set_pipeline(&self.sprite_pipeline);
                            pass.set_bind_group(
                                0,
                                &self
                                    .textures
                                    .get(&key)
                                    .expect("场景纹理已在编码前上传")
                                    .bind_group,
                                &[],
                            );
                        }
                        BatchKind::Glyph(key) => {
                            pass.set_pipeline(&self.glyph_pipeline);
                            pass.set_bind_group(
                                0,
                                &self
                                    .textures
                                    .get(&key)
                                    .expect("场景字形已在编码前上传")
                                    .bind_group,
                                &[],
                            );
                        }
                        BatchKind::SliderBorderCoverage | BatchKind::SliderBodyCoverage => {
                            pass.set_pipeline(&self.slider_coverage_pipeline);
                        }
                        BatchKind::SliderBorderColor | BatchKind::SliderBodyColor => {
                            pass.set_pipeline(&self.slider_color_pipeline);
                        }
                    }
                    pass.draw(batch.vertices, 0..1);
                }
            }
        }
        {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &self.output_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("osu-beatmap-preview straight alpha resolve pass"),
                color_attachments: &attachments,
                ..Default::default()
            });
            pass.set_pipeline(&self.resolve_pipeline);
            pass.set_bind_group(0, &self.resolve_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        Ok(())
    }

    fn prepare_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &FrameScene,
    ) -> Result<()> {
        let live = scene
            .resources
            .iter()
            .map(|(&id, image)| ResourceKey::new(id, image))
            .collect::<HashSet<_>>();
        self.textures.retain(|key, _| live.contains(key));
        for (&id, image) in scene.resources.iter() {
            if image.w == 0
                || image.h == 0
                || image.data.len() != image.w as usize * image.h as usize * 4
            {
                return Err(PreviewError::render(format!(
                    "scene resource {} has invalid dimensions or RGBA length",
                    id.0
                )));
            }
            let key = ResourceKey::new(id, image);
            if self.textures.contains_key(&key) {
                continue;
            }
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("osu-beatmap-preview scene resource"),
                size: wgpu::Extent3d {
                    width: image.w,
                    height: image.h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TARGET_FORMAT,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &image.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(image.w * 4),
                    rows_per_image: Some(image.h),
                },
                wgpu::Extent3d {
                    width: image.w,
                    height: image.h,
                    depth_or_array_layers: 1,
                },
            );
            let view = texture.create_view(&Default::default());
            let bind_group = create_texture_bind_group(
                device,
                &self.texture_layout,
                &view,
                &self.sampler,
                "osu-beatmap-preview scene resource bind group",
            );
            self.textures.insert(
                key,
                CachedTexture {
                    _texture: texture,
                    bind_group,
                },
            );
        }
        Ok(())
    }
}

fn create_target(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    sample_count: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage,
        view_formats: &[],
    })
}

fn create_scene_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    fragment_entry: &str,
    sample_count: u32,
    stencil_mode: StencilMode,
    label: &str,
) -> wgpu::RenderPipeline {
    let (compare, pass_op, write_mask, color_write_mask) = match stencil_mode {
        StencilMode::Keep => (
            wgpu::CompareFunction::Always,
            wgpu::StencilOperation::Keep,
            0,
            wgpu::ColorWrites::ALL,
        ),
        StencilMode::Coverage => (
            wgpu::CompareFunction::NotEqual,
            wgpu::StencilOperation::Replace,
            0xff,
            wgpu::ColorWrites::empty(),
        ),
        StencilMode::Color => (
            wgpu::CompareFunction::Equal,
            wgpu::StencilOperation::IncrementClamp,
            0xff,
            wgpu::ColorWrites::ALL,
        ),
    };
    let stencil_face = wgpu::StencilFaceState {
        compare,
        fail_op: wgpu::StencilOperation::Keep,
        depth_fail_op: wgpu::StencilOperation::Keep,
        pass_op,
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("scene_vertex"),
            compilation_options: Default::default(),
            buffers: &[GpuVertex::layout()],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Stencil8,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState {
                front: stencil_face,
                back: stencil_face,
                read_mask: 0xff,
                write_mask,
            },
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: sample_count,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: TARGET_FORMAT,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: color_write_mask,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_resolve_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("osu-beatmap-preview straight alpha resolve pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("resolve_vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("resolve_fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: TARGET_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    label: &str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn build_batches(
    scene: &FrameScene,
    target_width: u32,
    target_height: u32,
) -> Result<Vec<DrawBatch>> {
    let offset = [
        target_width.saturating_sub(scene.width()) as f32 / 2.0,
        target_height.saturating_sub(scene.height()) as f32 / 2.0,
    ];
    let mut clips = vec![SceneRect {
        x: 0.0,
        y: 0.0,
        width: scene.width() as f32,
        height: scene.height() as f32,
    }];
    let mut batches = Vec::new();
    for command in scene.commands.iter() {
        match command {
            DrawCommand::PushClip(rect) => {
                let parent = *clips.last().expect("裁剪栈始终保留场景边界");
                clips.push(intersection(parent, *rect));
            }
            DrawCommand::PopClip => {
                if clips.len() == 1 {
                    return Err(PreviewError::render(
                        "frame scene clip stack underflow".to_string(),
                    ));
                }
                clips.pop();
            }
            DrawCommand::Sprite {
                resource,
                destination,
                alpha,
            } => {
                let image = scene.resources.get(resource).ok_or_else(|| {
                    PreviewError::render(format!("frame scene resource {} is missing", resource.0))
                })?;
                push_textured_rect(
                    &mut batches,
                    BatchKind::Sprite(ResourceKey::new(*resource, image)),
                    scissor(
                        *clips.last().expect("裁剪栈始终非空"),
                        offset,
                        target_width,
                        target_height,
                    ),
                    *destination,
                    [1.0, 1.0, 1.0, alpha.clamp(0.0, 1.0)],
                    offset,
                    target_width,
                    target_height,
                );
            }
            DrawCommand::Glyph {
                resource,
                destination,
                color,
            } => {
                let image = scene.resources.get(resource).ok_or_else(|| {
                    PreviewError::render(format!("frame scene glyph {} is missing", resource.0))
                })?;
                push_textured_rect(
                    &mut batches,
                    BatchKind::Glyph(ResourceKey::new(*resource, image)),
                    scissor(
                        *clips.last().expect("裁剪栈始终非空"),
                        offset,
                        target_width,
                        target_height,
                    ),
                    *destination,
                    color_f32(*color),
                    offset,
                    target_width,
                    target_height,
                );
            }
            DrawCommand::Rectangle { rect, color } => push_rectangle(
                &mut batches,
                current_scissor(&clips, offset, target_width, target_height),
                *rect,
                *color,
                offset,
                target_width,
                target_height,
            ),
            DrawCommand::Circle {
                center,
                radius,
                color,
            } => push_circle(
                &mut batches,
                current_scissor(&clips, offset, target_width, target_height),
                *center,
                *radius,
                *color,
                offset,
                target_width,
                target_height,
            ),
            DrawCommand::Ring {
                center,
                radius,
                thickness,
                color,
            } => push_ring(
                &mut batches,
                current_scissor(&clips, offset, target_width, target_height),
                *center,
                *radius,
                *thickness,
                *color,
                offset,
                target_width,
                target_height,
            ),
            DrawCommand::Line {
                from,
                to,
                thickness,
                color,
            } => push_line(
                &mut batches,
                current_scissor(&clips, offset, target_width, target_height),
                *from,
                *to,
                *thickness,
                *color,
                offset,
                target_width,
                target_height,
                false,
            ),
            DrawCommand::SliderMesh {
                vertices,
                thickness,
                border,
                body,
            } => {
                let clip = current_scissor(&clips, offset, target_width, target_height);
                push_slider_layer(
                    &mut batches,
                    clip,
                    vertices,
                    *thickness,
                    *border,
                    offset,
                    target_width,
                    target_height,
                    BatchKind::SliderBorderCoverage,
                    BatchKind::SliderBorderColor,
                );
                push_slider_layer(
                    &mut batches,
                    clip,
                    vertices,
                    *thickness * 0.72,
                    *body,
                    offset,
                    target_width,
                    target_height,
                    BatchKind::SliderBodyCoverage,
                    BatchKind::SliderBodyColor,
                );
            }
        }
    }
    if clips.len() != 1 {
        return Err(PreviewError::render(
            "frame scene clip stack is unbalanced".to_string(),
        ));
    }
    Ok(batches)
}

fn current_scissor(clips: &[SceneRect], offset: [f32; 2], width: u32, height: u32) -> [u32; 4] {
    scissor(
        *clips.last().expect("裁剪栈始终非空"),
        offset,
        width,
        height,
    )
}

// 该函数只负责把一个纹理矩形转换为 GPU 批次；参数分别对应绘制状态、几何体和目标尺寸，
// 保持拆开可避免在每个调用点构造短生命周期的临时配置对象。
#[allow(clippy::too_many_arguments)]
fn push_textured_rect(
    batches: &mut Vec<DrawBatch>,
    kind: BatchKind,
    scissor: [u32; 4],
    rect: SceneRect,
    color: [f32; 4],
    offset: [f32; 2],
    width: u32,
    height: u32,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || color[3] <= 0.0 {
        return;
    }
    let positions = rect_positions(rect, offset, width, height);
    push_batch(
        batches,
        kind,
        scissor,
        quad(
            positions,
            [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            color,
        ),
    );
}

fn push_rectangle(
    batches: &mut Vec<DrawBatch>,
    scissor: [u32; 4],
    rect: SceneRect,
    color: Rgba,
    offset: [f32; 2],
    width: u32,
    height: u32,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || color[3] == 0 {
        return;
    }
    push_batch(
        batches,
        BatchKind::Solid,
        scissor,
        quad(
            rect_positions(rect, offset, width, height),
            [[0.0; 2]; 4],
            color_f32(color),
        ),
    );
}

#[allow(clippy::too_many_arguments)]
fn push_circle(
    batches: &mut Vec<DrawBatch>,
    scissor: [u32; 4],
    center: [f32; 2],
    radius: f32,
    color: Rgba,
    offset: [f32; 2],
    width: u32,
    height: u32,
) {
    if radius <= 0.0 || color[3] == 0 {
        return;
    }
    let segments = circle_segments(radius);
    let mut vertices = Vec::with_capacity(segments * 3);
    let center_vertex = vertex(center, [0.0; 2], color_f32(color), offset, width, height);
    for index in 0..segments {
        let first = radial_point(center, radius, index, segments);
        let second = radial_point(center, radius, index + 1, segments);
        vertices.extend_from_slice(&[
            center_vertex,
            vertex(first, [0.0; 2], color_f32(color), offset, width, height),
            vertex(second, [0.0; 2], color_f32(color), offset, width, height),
        ]);
    }
    push_batch(batches, BatchKind::Solid, scissor, vertices);
}

#[allow(clippy::too_many_arguments)]
fn push_ring(
    batches: &mut Vec<DrawBatch>,
    scissor: [u32; 4],
    center: [f32; 2],
    outer_radius: f32,
    thickness: f32,
    color: Rgba,
    offset: [f32; 2],
    width: u32,
    height: u32,
) {
    if outer_radius <= 0.0 || thickness <= 0.0 || color[3] == 0 {
        return;
    }
    let inner_radius = (outer_radius - thickness).max(0.0);
    if inner_radius == 0.0 {
        push_circle(
            batches,
            scissor,
            center,
            outer_radius,
            color,
            offset,
            width,
            height,
        );
        return;
    }
    let segments = circle_segments(outer_radius);
    let color = color_f32(color);
    let mut vertices = Vec::with_capacity(segments * 6);
    for index in 0..segments {
        let outer_a = vertex(
            radial_point(center, outer_radius, index, segments),
            [0.0; 2],
            color,
            offset,
            width,
            height,
        );
        let outer_b = vertex(
            radial_point(center, outer_radius, index + 1, segments),
            [0.0; 2],
            color,
            offset,
            width,
            height,
        );
        let inner_a = vertex(
            radial_point(center, inner_radius, index, segments),
            [0.0; 2],
            color,
            offset,
            width,
            height,
        );
        let inner_b = vertex(
            radial_point(center, inner_radius, index + 1, segments),
            [0.0; 2],
            color,
            offset,
            width,
            height,
        );
        vertices.extend_from_slice(&[outer_a, outer_b, inner_b, outer_a, inner_b, inner_a]);
    }
    push_batch(batches, BatchKind::Solid, scissor, vertices);
}

#[allow(clippy::too_many_arguments)]
fn push_line(
    batches: &mut Vec<DrawBatch>,
    scissor: [u32; 4],
    from: [f32; 2],
    to: [f32; 2],
    thickness: f32,
    color: Rgba,
    offset: [f32; 2],
    width: u32,
    height: u32,
    round_caps: bool,
) {
    if thickness <= 0.0 || color[3] == 0 {
        return;
    }
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let length = dx.hypot(dy);
    if length <= f32::EPSILON {
        if round_caps {
            push_circle(
                batches,
                scissor,
                from,
                thickness / 2.0,
                color,
                offset,
                width,
                height,
            );
        }
        return;
    }
    let scale = thickness / (2.0 * length);
    let perpendicular = [-dy * scale, dx * scale];
    let points = [
        [from[0] + perpendicular[0], from[1] + perpendicular[1]],
        [to[0] + perpendicular[0], to[1] + perpendicular[1]],
        [to[0] - perpendicular[0], to[1] - perpendicular[1]],
        [from[0] - perpendicular[0], from[1] - perpendicular[1]],
    ];
    let positions = points.map(|point| position(point, offset, width, height));
    push_batch(
        batches,
        BatchKind::Solid,
        scissor,
        quad(positions, [[0.0; 2]; 4], color_f32(color)),
    );
    if round_caps {
        push_circle(
            batches,
            scissor,
            from,
            thickness / 2.0,
            color,
            offset,
            width,
            height,
        );
        push_circle(
            batches,
            scissor,
            to,
            thickness / 2.0,
            color,
            offset,
            width,
            height,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_slider_layer(
    batches: &mut Vec<DrawBatch>,
    scissor: [u32; 4],
    points: &[[f32; 2]],
    thickness: f32,
    color: Rgba,
    offset: [f32; 2],
    width: u32,
    height: u32,
    coverage_kind: BatchKind,
    color_kind: BatchKind,
) {
    if points.is_empty() || thickness <= 0.0 || color[3] == 0 {
        return;
    }
    let mut path = Vec::with_capacity(points.len());
    for &point in points {
        if path
            .last()
            .is_none_or(|last: &[f32; 2]| (point[0] - last[0]).hypot(point[1] - last[1]) > 1e-4)
        {
            path.push(point);
        }
    }
    let mut vertices = Vec::new();
    let rgba = color_f32(color);
    for segment in path.windows(2) {
        let from = segment[0];
        let to = segment[1];
        let dx = to[0] - from[0];
        let dy = to[1] - from[1];
        let length = dx.hypot(dy);
        if length <= f32::EPSILON {
            continue;
        }
        let scale = thickness / (2.0 * length);
        let normal = [-dy * scale, dx * scale];
        let quad_points = [
            [from[0] + normal[0], from[1] + normal[1]],
            [to[0] + normal[0], to[1] + normal[1]],
            [to[0] - normal[0], to[1] - normal[1]],
            [from[0] - normal[0], from[1] - normal[1]],
        ];
        let positions =
            quad_points.map(|point| vertex(point, [0.0; 2], rgba, offset, width, height));
        vertices.extend_from_slice(&[
            positions[0],
            positions[1],
            positions[2],
            positions[0],
            positions[2],
            positions[3],
        ]);
    }
    // 圆形连接头和端帽保留 CPU 的 round join 外观；stencil coverage pass 会将重叠区域去重。
    let segments = circle_segments(thickness / 2.0);
    for &center in &path {
        let center_vertex = vertex(center, [0.0; 2], rgba, offset, width, height);
        for index in 0..segments {
            vertices.extend_from_slice(&[
                center_vertex,
                vertex(
                    radial_point(center, thickness / 2.0, index, segments),
                    [0.0; 2],
                    rgba,
                    offset,
                    width,
                    height,
                ),
                vertex(
                    radial_point(center, thickness / 2.0, index + 1, segments),
                    [0.0; 2],
                    rgba,
                    offset,
                    width,
                    height,
                ),
            ]);
        }
    }
    if vertices.is_empty() {
        return;
    }
    push_batch(batches, coverage_kind, scissor, vertices.clone());
    push_batch(batches, color_kind, scissor, vertices);
}

fn push_batch(
    batches: &mut Vec<DrawBatch>,
    kind: BatchKind,
    scissor: [u32; 4],
    vertices: Vec<GpuVertex>,
) {
    if vertices.is_empty() || scissor[2] == 0 || scissor[3] == 0 {
        return;
    }
    if let Some(last) = batches.last_mut() {
        if last.kind == kind && last.scissor == scissor {
            last.vertices.extend(vertices);
            return;
        }
    }
    batches.push(DrawBatch {
        kind,
        scissor,
        vertices,
    });
}

fn upload_batches(batches: Vec<DrawBatch>) -> (Vec<GpuVertex>, Vec<UploadedBatch>) {
    let count = batches.iter().map(|batch| batch.vertices.len()).sum();
    let mut vertices = Vec::with_capacity(count);
    let mut uploaded = Vec::with_capacity(batches.len());
    for batch in batches {
        let start = vertices.len() as u32;
        vertices.extend(batch.vertices);
        uploaded.push(UploadedBatch {
            kind: batch.kind,
            scissor: batch.scissor,
            vertices: start..vertices.len() as u32,
        });
    }
    (vertices, uploaded)
}

fn stencil_reference(kind: BatchKind) -> u8 {
    match kind {
        BatchKind::SliderBorderCoverage | BatchKind::SliderBorderColor => 1,
        BatchKind::SliderBodyCoverage | BatchKind::SliderBodyColor => 3,
        _ => 0,
    }
}

fn rect_positions(rect: SceneRect, offset: [f32; 2], width: u32, height: u32) -> [[f32; 2]; 4] {
    [
        position([rect.x, rect.y], offset, width, height),
        position([rect.x + rect.width, rect.y], offset, width, height),
        position(
            [rect.x + rect.width, rect.y + rect.height],
            offset,
            width,
            height,
        ),
        position([rect.x, rect.y + rect.height], offset, width, height),
    ]
}

fn quad(positions: [[f32; 2]; 4], uv: [[f32; 2]; 4], color: [f32; 4]) -> Vec<GpuVertex> {
    [0, 1, 2, 0, 2, 3]
        .map(|index| GpuVertex {
            position: positions[index],
            uv: uv[index],
            color,
        })
        .to_vec()
}

fn vertex(
    point: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    offset: [f32; 2],
    width: u32,
    height: u32,
) -> GpuVertex {
    GpuVertex {
        position: position(point, offset, width, height),
        uv,
        color,
    }
}

fn position(point: [f32; 2], offset: [f32; 2], width: u32, height: u32) -> [f32; 2] {
    [
        (point[0] + offset[0]) * 2.0 / width as f32 - 1.0,
        1.0 - (point[1] + offset[1]) * 2.0 / height as f32,
    ]
}

fn color_f32(color: Rgba) -> [f32; 4] {
    color.map(|channel| channel as f32 / 255.0)
}

fn radial_point(center: [f32; 2], radius: f32, index: usize, segments: usize) -> [f32; 2] {
    let angle = index as f32 * TAU / segments as f32;
    [
        center[0] + angle.cos() * radius,
        center[1] + angle.sin() * radius,
    ]
}

fn circle_segments(radius: f32) -> usize {
    (radius * 2.0).ceil().clamp(24.0, 192.0) as usize
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

fn scissor(rect: SceneRect, offset: [f32; 2], width: u32, height: u32) -> [u32; 4] {
    let left = (rect.x + offset[0]).round().clamp(0.0, width as f32) as u32;
    let top = (rect.y + offset[1]).round().clamp(0.0, height as f32) as u32;
    let right = (rect.x + rect.width + offset[0])
        .round()
        .clamp(left as f32, width as f32) as u32;
    let bottom = (rect.y + rect.height + offset[1])
        .round()
        .clamp(top as f32, height as f32) as u32;
    [left, top, right - left, bottom - top]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use osu_beatmap_preview_core::render::scene::{ResourceId, SceneSize};

    fn scene(commands: Vec<DrawCommand>) -> FrameScene {
        FrameScene {
            size: SceneSize {
                width: 100,
                height: 50,
            },
            absolute_time_ms: 0,
            commands: commands.into(),
            resources: Arc::new(BTreeMap::new()),
        }
    }

    #[test]
    fn 相邻同类命令会合并且场景在目标中居中() {
        let scene = scene(vec![
            DrawCommand::Rectangle {
                rect: SceneRect {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                color: [255, 0, 0, 255],
            },
            DrawCommand::Rectangle {
                rect: SceneRect {
                    x: 10.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                color: [0, 255, 0, 255],
            },
        ]);
        let batches = build_batches(&scene, 200, 100).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].vertices.len(), 12);
        assert_eq!(batches[0].scissor, [50, 25, 100, 50]);
        assert_eq!(batches[0].vertices[0].position, [-0.5, 0.5]);
    }

    #[test]
    fn 裁剪栈错误会在提交gpu前被拒绝() {
        let underflow = scene(vec![DrawCommand::PopClip]);
        assert!(build_batches(&underflow, 100, 50).is_err());
        let unbalanced = scene(vec![DrawCommand::PushClip(SceneRect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        })]);
        assert!(build_batches(&unbalanced, 100, 50).is_err());
    }

    #[test]
    fn 滑条网格生成两层圆角覆盖并集() {
        let points: Arc<[[f32; 2]]> = Arc::from([[10.0, 10.0], [40.0, 10.0], [40.0, 30.0]]);
        let scene = scene(vec![DrawCommand::SliderMesh {
            vertices: points,
            thickness: 12.0,
            border: [0, 0, 0, 255],
            body: [255, 0, 0, 255],
        }]);
        let batches = build_batches(&scene, 100, 50).unwrap();
        assert_eq!(batches.len(), 4);
        assert_eq!(batches[0].kind, BatchKind::SliderBorderCoverage);
        assert_eq!(batches[1].kind, BatchKind::SliderBorderColor);
        assert_eq!(batches[2].kind, BatchKind::SliderBodyCoverage);
        assert_eq!(batches[3].kind, BatchKind::SliderBodyColor);
        assert_eq!(batches[0].vertices.len(), batches[1].vertices.len());
        assert_eq!(batches[2].vertices.len(), batches[3].vertices.len());
        assert_eq!(stencil_reference(batches[0].kind), 1);
        assert_eq!(stencil_reference(batches[2].kind), 3);
    }

    #[test]
    fn 资源键同时包含稳定编号和资源实例() {
        let first = Arc::new(Img::new(1, 1, [0, 0, 0, 0]));
        let second = Arc::new(Img::new(1, 1, [0, 0, 0, 0]));
        assert_ne!(
            ResourceKey::new(ResourceId(1), &first),
            ResourceKey::new(ResourceId(1), &second)
        );
        assert_eq!(
            ResourceKey::new(ResourceId(1), &first),
            ResourceKey::new(ResourceId(1), &first)
        );
    }
}
