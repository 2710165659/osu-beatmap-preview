//! 平台无关的 WGPU 绘制适配层。
//!
//! 平台负责创建 `Device`、`Queue`、surface 和目标 view；本 crate 不持有窗口或 Canvas 句柄。

use std::sync::Arc;

use osu_beatmap_preview_core::{FrameScene, PreviewError, Result};

mod offscreen;
mod rasterizer;
mod runtime;

pub mod adapters;

pub use offscreen::OffscreenRenderer;
pub use osu_beatmap_preview_core::{DrawCommand, Img, Rgba, SceneRect, SceneSize};
pub use runtime::RendererError as RealtimeError;
pub use runtime::{
    CancellationToken, FrameRequest, OffscreenConfig, RenderedFrame, RendererError, RendererInfo,
    RgbaFrame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceConfig {
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
}

/// 将场景编码到宿主提供的 WGPU texture view。
///
/// `Device` 和 `Queue` 由平台创建并通过 `Arc` 共享，renderer 不持有窗口或 Canvas。
pub struct SurfaceRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    config: SurfaceConfig,
    rasterizer: rasterizer::SceneRasterizer,
    present_layout: wgpu::BindGroupLayout,
    present_sampler: wgpu::Sampler,
    present_pipeline: wgpu::RenderPipeline,
    present_bind_group: wgpu::BindGroup,
}
impl SurfaceRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        config: SurfaceConfig,
    ) -> Result<Self> {
        if config.width == 0 || config.height == 0 {
            return Err(PreviewError::render("surface dimensions must be positive"));
        }
        let rasterizer = rasterizer::SceneRasterizer::new(&device, config.width, config.height, 1);
        let (present_layout, present_sampler, present_pipeline, present_bind_group) =
            create_present_resources(&device, config.format, rasterizer.output_view());
        Ok(Self {
            device,
            queue,
            config,
            rasterizer,
            present_layout,
            present_sampler,
            present_pipeline,
            present_bind_group,
        })
    }
    pub fn config(&self) -> SurfaceConfig {
        self.config
    }
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Err(PreviewError::render("surface dimensions must be positive"));
        }
        self.config.width = width;
        self.config.height = height;
        self.rasterizer = rasterizer::SceneRasterizer::new(&self.device, width, height, 1);
        let (layout, sampler, pipeline, bind_group) = create_present_resources(
            &self.device,
            self.config.format,
            self.rasterizer.output_view(),
        );
        self.present_layout = layout;
        self.present_sampler = sampler;
        self.present_pipeline = pipeline;
        self.present_bind_group = bind_group;
        Ok(())
    }
    pub fn render_to_view(&mut self, scene: &FrameScene, view: &wgpu::TextureView) -> Result<()> {
        if scene.width() > self.config.width || scene.height() > self.config.height {
            return Err(PreviewError::render("surface is smaller than scene"));
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("osu preview surface encoder"),
            });
        self.rasterizer
            .encode(&self.device, &self.queue, &mut encoder, scene)?;
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("osu preview present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.present_pipeline);
            pass.set_bind_group(0, &self.present_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}

fn create_present_resources(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    source: &wgpu::TextureView,
) -> (
    wgpu::BindGroupLayout,
    wgpu::Sampler,
    wgpu::RenderPipeline,
    wgpu::BindGroup,
) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("osu preview present layout"),
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
        label: Some("osu preview present sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let shader = device.create_shader_module(wgpu::include_wgsl!("present.wgsl"));
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("osu preview present pipeline layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("osu preview present pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("present_vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("present_fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("osu preview present bind group"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    (layout, sampler, pipeline, bind_group)
}
