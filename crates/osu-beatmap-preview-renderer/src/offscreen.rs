//! WGPU RGBA8 离屏目标与紧凑行 readback。

use std::collections::HashSet;
use std::collections::VecDeque;
use std::sync::mpsc;

use crate::{
    CancellationToken, FrameRequest, OffscreenConfig, RenderedFrame, RendererError, RendererInfo,
    RgbaFrame,
};
use osu_beatmap_preview_core::FrameScene;

use super::rasterizer::SceneRasterizer;

pub struct OffscreenRenderer {
    config: OffscreenConfig,
    info: RendererInfo,
    device: wgpu::Device,
    queue: wgpu::Queue,
    rasterizer: SceneRasterizer,
    readbacks: Vec<wgpu::Buffer>,
    next_readback: usize,
    padded_bytes_per_row: u32,
}

struct PendingReadback {
    buffer_index: usize,
    submission: wgpu::SubmissionIndex,
    receiver: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
}

impl std::fmt::Debug for OffscreenRenderer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OffscreenRenderer")
            .field("config", &self.config)
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl OffscreenRenderer {
    pub async fn new(config: OffscreenConfig) -> Result<Self, RendererError> {
        validate_config(config)?;
        let backends = wgpu::Backends::from_env().unwrap_or_else(wgpu::Backends::all);
        if backends.is_empty() {
            return Err(RendererError::GpuUnavailable(
                "WGPU_BACKEND did not contain a supported backend name".to_string(),
            ));
        }
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = backends;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = select_adapter(&instance, backends).await?;
        let adapter_info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("osu-beatmap-preview offscreen device"),
                ..Default::default()
            })
            .await
            .map_err(|error| RendererError::Device(error.to_string()))?;
        let format_features = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba8Unorm);
        let msaa_samples = [config.msaa_samples, 8, 4, 2, 1]
            .into_iter()
            .filter(|sample| *sample <= config.msaa_samples)
            .find(|sample| format_features.flags.sample_count_supported(*sample))
            .unwrap_or(1);
        let rasterizer = SceneRasterizer::new(&device, config.width, config.height, msaa_samples);
        let row_bytes = config.width * 4;
        let padded_bytes_per_row = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer_size = padded_bytes_per_row as u64 * config.height as u64;
        let readbacks = (0..config.max_in_flight)
            .map(|index| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(match index {
                        0 => "osu-beatmap-preview readback 0",
                        1 => "osu-beatmap-preview readback 1",
                        _ => "osu-beatmap-preview readback",
                    }),
                    size: buffer_size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            })
            .collect();
        Ok(Self {
            config,
            info: RendererInfo {
                adapter_name: adapter_info.name,
                backend: format!("{:?}", adapter_info.backend),
                msaa_samples,
            },
            device,
            queue,
            rasterizer,
            readbacks,
            next_readback: 0,
            padded_bytes_per_row,
        })
    }

    pub fn info(&self) -> &RendererInfo {
        &self.info
    }

    #[allow(dead_code)]
    pub fn config(&self) -> OffscreenConfig {
        self.config
    }

    #[allow(dead_code)]
    pub async fn render(&mut self, scene: &FrameScene) -> Result<RgbaFrame, RendererError> {
        if scene.width() > self.config.width || scene.height() > self.config.height {
            return Err(RendererError::CanvasTooSmall {
                width: self.config.width,
                height: self.config.height,
                required_width: scene.width(),
                required_height: scene.height(),
            });
        }
        let buffer_index = self.next_readback;
        self.next_readback = (self.next_readback + 1) % self.readbacks.len();
        let pending = self.submit_scene(scene, buffer_index)?;
        self.finish_readback(pending)
    }

    fn submit_scene(
        &mut self,
        scene: &FrameScene,
        buffer_index: usize,
    ) -> Result<PendingReadback, RendererError> {
        let buffer = &self.readbacks[buffer_index];
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("osu-beatmap-preview readback encoder"),
            });
        self.rasterizer
            .encode(&self.device, &self.queue, &mut encoder, scene)
            .map_err(|error| RendererError::Scene(error.to_string()))?;
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: self.rasterizer.output(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.config.height),
                },
            },
            wgpu::Extent3d {
                width: self.config.width,
                height: self.config.height,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (sender, receiver) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        Ok(PendingReadback {
            buffer_index,
            submission,
            receiver,
        })
    }

    fn finish_readback(&self, pending: PendingReadback) -> Result<RgbaFrame, RendererError> {
        let buffer = &self.readbacks[pending.buffer_index];
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(pending.submission),
                timeout: None,
            })
            .map_err(|error| RendererError::Device(error.to_string()))?;
        pending
            .receiver
            .recv()
            .map_err(|_| RendererError::Device("readback callback was dropped".to_string()))?
            .map_err(|error| RendererError::Device(error.to_string()))?;
        let mapped = buffer.slice(..).get_mapped_range();
        let row_bytes = self.config.width as usize * 4;
        let mut compact = Vec::with_capacity(row_bytes * self.config.height as usize);
        for row in mapped.chunks_exact(self.padded_bytes_per_row as usize) {
            compact.extend_from_slice(&row[..row_bytes]);
        }
        drop(mapped);
        buffer.unmap();
        RgbaFrame::new(self.config.width, self.config.height, compact)
    }

    fn abort_pending(&self, pending: &VecDeque<(FrameRequest, PendingReadback)>) {
        // 取消映射不会提交新工作；设备稍后仍可回收已提交命令占用的资源。
        for (_, readback) in pending {
            self.readbacks[readback.buffer_index].unmap();
        }
        let _ = self.device.poll(wgpu::PollType::Poll);
    }

    /// CLI 与公开会话接口共用同一套有界 readback 调度；CLI 在提交前还会把
    /// playfield、背景和 HUD 合成为最终场景，因此场景构造保留为内部回调。
    pub async fn render_generated_stream(
        &mut self,
        requests: impl IntoIterator<Item = FrameRequest>,
        cancellation: &CancellationToken,
        mut scene_at: impl FnMut(FrameRequest) -> Result<FrameScene, RendererError>,
        mut callback: impl FnMut(RenderedFrame) -> Result<(), RendererError>,
    ) -> Result<(), RendererError> {
        if cancellation.is_cancelled() {
            return Err(RendererError::Cancelled);
        }
        let requests = normalize_requests(requests)?;
        let mut pending = VecDeque::with_capacity(self.config.max_in_flight);
        for request in requests {
            if cancellation.is_cancelled() {
                self.abort_pending(&pending);
                return Err(RendererError::Cancelled);
            }
            if pending.len() == self.config.max_in_flight {
                let (completed, readback) =
                    pending.pop_front().expect("达到在途上限时队列必定非空");
                let rgba = match self.finish_readback(readback) {
                    Ok(frame) => frame,
                    Err(error) => {
                        self.abort_pending(&pending);
                        return Err(error);
                    }
                };
                if cancellation.is_cancelled() {
                    self.abort_pending(&pending);
                    return Err(RendererError::Cancelled);
                }
                if let Err(error) = callback(RenderedFrame {
                    index: completed.index,
                    absolute_time_ms: completed.absolute_time_ms,
                    rgba,
                }) {
                    self.abort_pending(&pending);
                    return Err(error);
                }
            }
            let scene = match scene_at(request) {
                Ok(scene) => scene,
                Err(error) => {
                    self.abort_pending(&pending);
                    return Err(error);
                }
            };
            let buffer_index = self.next_readback;
            self.next_readback = (self.next_readback + 1) % self.readbacks.len();
            let readback = match self.submit_scene(&scene, buffer_index) {
                Ok(readback) => readback,
                Err(error) => {
                    self.abort_pending(&pending);
                    return Err(error);
                }
            };
            pending.push_back((request, readback));
        }
        while let Some((request, readback)) = pending.pop_front() {
            let rgba = match self.finish_readback(readback) {
                Ok(frame) => frame,
                Err(error) => {
                    self.abort_pending(&pending);
                    return Err(error);
                }
            };
            if cancellation.is_cancelled() {
                self.abort_pending(&pending);
                return Err(RendererError::Cancelled);
            }
            if let Err(error) = callback(RenderedFrame {
                index: request.index,
                absolute_time_ms: request.absolute_time_ms,
                rgba,
            }) {
                self.abort_pending(&pending);
                return Err(error);
            }
        }
        Ok(())
    }
}

fn normalize_requests(
    requests: impl IntoIterator<Item = FrameRequest>,
) -> Result<Vec<FrameRequest>, RendererError> {
    let mut requests = requests.into_iter().collect::<Vec<_>>();
    let mut indexes = HashSet::with_capacity(requests.len());
    for request in &requests {
        if !indexes.insert(request.index) {
            return Err(RendererError::DuplicateFrameIndex(request.index));
        }
    }
    requests.sort_by_key(|request| request.index);
    Ok(requests)
}

async fn select_adapter(
    instance: &wgpu::Instance,
    backends: wgpu::Backends,
) -> Result<wgpu::Adapter, RendererError> {
    if let Ok(wanted) = std::env::var("WGPU_ADAPTER_NAME") {
        let wanted_lower = wanted.to_lowercase();
        return instance
            .enumerate_adapters(backends)
            .await
            .into_iter()
            .find(|adapter| {
                adapter
                    .get_info()
                    .name
                    .to_lowercase()
                    .contains(&wanted_lower)
            })
            .ok_or_else(|| {
                RendererError::GpuUnavailable(format!(
                    "WGPU_ADAPTER_NAME '{wanted}' did not match an available adapter"
                ))
            });
    }
    instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .await
        .map_err(|error| RendererError::GpuUnavailable(error.to_string()))
}

fn validate_config(config: OffscreenConfig) -> Result<(), RendererError> {
    if config.width == 0
        || config.height == 0
        || config.target_fps == 0
        || config.max_in_flight == 0
    {
        return Err(RendererError::InvalidRequest(
            "offscreen dimensions, target_fps and max_in_flight must be positive".to_string(),
        ));
    }
    if !matches!(config.msaa_samples, 1 | 2 | 4 | 8 | 16) {
        return Err(RendererError::InvalidRequest(
            "msaa_samples must be one of 1, 2, 4, 8, 16".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use osu_beatmap_preview_core::Img;
    use osu_beatmap_preview_core::{DrawCommand, SceneRect, SceneSize};

    #[test]
    fn 离屏配置拒绝零尺寸和非法采样数() {
        assert!(validate_config(OffscreenConfig {
            width: 0,
            ..Default::default()
        })
        .is_err());
        assert!(validate_config(OffscreenConfig {
            msaa_samples: 3,
            ..Default::default()
        })
        .is_err());
    }

    #[test]
    fn 流请求在提交前拒绝重复编号并按编号排序() {
        let sorted = normalize_requests([
            FrameRequest {
                index: 9,
                absolute_time_ms: 90,
            },
            FrameRequest {
                index: 2,
                absolute_time_ms: 20,
            },
        ])
        .unwrap();
        assert_eq!(
            sorted
                .iter()
                .map(|request| request.index)
                .collect::<Vec<_>>(),
            vec![2, 9]
        );
        let duplicate = normalize_requests([
            FrameRequest {
                index: 2,
                absolute_time_ms: 20,
            },
            FrameRequest {
                index: 2,
                absolute_time_ms: 30,
            },
        ]);
        assert_eq!(duplicate, Err(RendererError::DuplicateFrameIndex(2)));
    }

    #[test]
    #[ignore = "需要本机可用的 WGPU 适配器"]
    fn wgpu回读会移除行对齐填充并保留rgba() {
        let config = OffscreenConfig {
            width: 65,
            height: 17,
            msaa_samples: 4,
            target_fps: 30,
            max_in_flight: 3,
        };
        let mut renderer = pollster::block_on(OffscreenRenderer::new(config)).unwrap();
        let scene = FrameScene::from_image(Img::new(65, 17, [12, 34, 56, 255]), 0);
        let frame = pollster::block_on(renderer.render(&scene)).unwrap();
        assert_eq!((frame.width(), frame.height()), (65, 17));
        assert_eq!(frame.as_bytes().len(), 65 * 17 * 4);
        assert_eq!(&frame.as_bytes()[..4], &[12, 34, 56, 255]);
        assert_eq!(
            &frame.as_bytes()[frame.as_bytes().len() - 4..],
            &[12, 34, 56, 255]
        );
    }

    #[test]
    #[ignore = "需要本机可用的 WGPU 适配器"]
    fn wgpu直接光栅化图元并遵守透明混合与裁剪() {
        let config = OffscreenConfig {
            width: 96,
            height: 64,
            msaa_samples: 4,
            target_fps: 30,
            max_in_flight: 3,
        };
        let commands = vec![
            DrawCommand::Rectangle {
                rect: SceneRect {
                    x: 4.0,
                    y: 4.0,
                    width: 30.0,
                    height: 20.0,
                },
                color: [0, 0, 255, 255],
            },
            DrawCommand::Rectangle {
                rect: SceneRect {
                    x: 14.0,
                    y: 4.0,
                    width: 20.0,
                    height: 20.0,
                },
                color: [255, 0, 0, 128],
            },
            DrawCommand::PushClip(SceneRect {
                x: 0.0,
                y: 0.0,
                width: 48.0,
                height: 64.0,
            }),
            DrawCommand::Rectangle {
                rect: SceneRect {
                    x: 40.0,
                    y: 30.0,
                    width: 20.0,
                    height: 10.0,
                },
                color: [255, 255, 0, 255],
            },
            DrawCommand::PopClip,
            DrawCommand::Circle {
                center: [65.0, 14.0],
                radius: 8.0,
                color: [255, 255, 255, 255],
            },
            DrawCommand::Line {
                from: [56.0, 31.0],
                to: [78.0, 31.0],
                thickness: 5.0,
                color: [0, 255, 0, 255],
            },
            DrawCommand::SliderMesh {
                vertices: Arc::from([[56.0, 50.0], [72.0, 44.0], [86.0, 52.0]]),
                thickness: 10.0,
                border: [255, 255, 255, 255],
                body: [255, 64, 128, 255],
            },
        ];
        let scene = FrameScene {
            size: SceneSize {
                width: 96,
                height: 64,
            },
            absolute_time_ms: 0,
            commands: commands.into(),
            resources: Arc::new(BTreeMap::new()),
        };
        let mut renderer = pollster::block_on(OffscreenRenderer::new(config)).unwrap();
        assert!(renderer.info().msaa_samples > 1);
        let frame = pollster::block_on(renderer.render(&scene)).unwrap();
        let pixel = |x: usize, y: usize| {
            let offset = (y * 96 + x) * 4;
            &frame.as_bytes()[offset..offset + 4]
        };
        assert_eq!(pixel(8, 8), &[0, 0, 255, 255]);
        let blended = pixel(20, 8);
        assert!((126..=129).contains(&blended[0]));
        assert_eq!(blended[1], 0);
        assert!((126..=129).contains(&blended[2]));
        assert_eq!(blended[3], 255);
        assert_eq!(pixel(45, 35), &[255, 255, 0, 255]);
        assert_eq!(pixel(52, 35), &[0, 0, 0, 0]);
        assert_eq!(pixel(65, 14), &[255, 255, 255, 255]);
        assert_eq!(pixel(65, 31), &[0, 255, 0, 255]);
        assert_eq!(pixel(72, 44), &[255, 64, 128, 255]);
    }

    #[test]
    #[ignore = "需要本机可用的 WGPU 适配器"]
    fn wgpu滑条圆角连接不会重复累加透明度或产生尖角() {
        let config = OffscreenConfig {
            width: 96,
            height: 64,
            msaa_samples: 4,
            target_fps: 30,
            max_in_flight: 3,
        };
        let scene = FrameScene {
            size: SceneSize {
                width: 96,
                height: 64,
            },
            absolute_time_ms: 0,
            commands: vec![DrawCommand::SliderMesh {
                vertices: Arc::from([[20.0, 20.0], [50.0, 20.0], [50.0, 50.0]]),
                thickness: 12.0,
                border: [255, 0, 0, 128],
                body: [0, 0, 0, 0],
            }]
            .into(),
            resources: Arc::new(BTreeMap::new()),
        };
        let mut renderer = pollster::block_on(OffscreenRenderer::new(config)).unwrap();
        let frame = pollster::block_on(renderer.render(&scene)).unwrap();
        let alpha = |x: usize, y: usize| frame.as_bytes()[(y * 96 + x) * 4 + 3];

        // 直线与折角中心都只应用一次滑条 alpha；远离路径的位置保持透明。
        assert!((126..=129).contains(&alpha(30, 20)));
        assert!((126..=129).contains(&alpha(50, 20)));
        assert_eq!(alpha(75, 5), 0);
    }
}
