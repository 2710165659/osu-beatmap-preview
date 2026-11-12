//! 浏览器 GPU 后端的创建与资源封装；独立于谱面、时钟和音频会话。

use std::sync::Arc;

use osu_beatmap_preview_renderer::{SurfaceConfig, SurfaceRenderer};
use wasm_bindgen::prelude::*;

#[derive(Debug)]
struct BrowserDisplay;

impl wgpu::rwh::HasDisplayHandle for BrowserDisplay {
    fn display_handle(&self) -> Result<wgpu::rwh::DisplayHandle<'_>, wgpu::rwh::HandleError> {
        Ok(wgpu::rwh::DisplayHandle::web())
    }
}

#[wasm_bindgen]
pub struct WebRenderer {
    pub(crate) surface: wgpu::Surface<'static>,
    pub(crate) config: wgpu::SurfaceConfiguration,
    pub(crate) renderer: SurfaceRenderer,
    backend: String,
}

#[wasm_bindgen]
impl WebRenderer {
    /// 在新 Canvas 上准备渲染器；创建失败不会影响正在播放的谱面会话。
    #[wasm_bindgen(js_name = create)]
    pub async fn create(
        canvas: web_sys::HtmlCanvasElement,
        width: u32,
        height: u32,
        backend: &str,
    ) -> Result<WebRenderer, JsValue> {
        let backends = match backend {
            "webgpu" => wgpu::Backends::BROWSER_WEBGPU,
            "webgl2" => wgpu::Backends::GL,
            _ => return Err(JsValue::from_str("渲染后端必须为 webgpu 或 webgl2")),
        };
        if width == 0 || height == 0 {
            return Err(JsValue::from_str("Canvas 尺寸必须为正数"));
        }
        canvas.set_width(width);
        canvas.set_height(height);
        // wgpu-core 的 WebGL surface 同样要求 display；Web 句柄不借用原生资源。
        let mut descriptor =
            wgpu::InstanceDescriptor::new_with_display_handle(Box::new(BrowserDisplay));
        descriptor.backends = backends;
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(crate::js_error)?;
        let mut options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        };
        let adapter = match instance.request_adapter(&options).await {
            Ok(adapter) => adapter,
            Err(hardware_error) if backend == "webgpu" => {
                options.force_fallback_adapter = true;
                instance.request_adapter(&options).await.map_err(|fallback_error| {
                    JsValue::from_str(&format!(
                        "没有可用的 WebGPU 适配器（硬件：{hardware_error}；软件回退：{fallback_error}）"
                    ))
                })?
            }
            Err(error) => return Err(crate::js_error(error)),
        };
        let required_limits = if backend == "webgl2" {
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits())
        } else {
            wgpu::Limits::default()
        };
        if width > required_limits.max_texture_dimension_2d
            || height > required_limits.max_texture_dimension_2d
        {
            return Err(JsValue::from_str("输出尺寸超出当前 GPU 后端的纹理尺寸限制"));
        }
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("osu beatmap preview browser device"),
                required_limits,
                ..Default::default()
            })
            .await
            .map_err(crate::js_error)?;
        let mut config = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| JsValue::from_str("Canvas 不支持所选 GPU 后端"))?;
        // 场景纹理已保存 sRGB 编码后的颜色；wgpu-core 优先返回 sRGB surface，
        // 直接呈现会再编码一次。与 WebGPU 一样使用普通 UNORM，保留原始颜色值。
        let format = config.format.remove_srgb_suffix();
        if !surface.get_capabilities(&adapter).formats.contains(&format) {
            return Err(JsValue::from_str("Canvas 不支持保留场景颜色的输出格式"));
        }
        config.format = format;
        let device = Arc::new(device);
        // 管线验证失败必须成为可捕获错误，宿主才能继续尝试另一个后端。
        let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
        surface.configure(&device, &config);
        let renderer = SurfaceRenderer::new(
            Arc::clone(&device),
            Arc::new(queue),
            SurfaceConfig {
                width,
                height,
                format: config.format,
            },
        )
        .map_err(crate::js_error)?;
        if let Some(error) = errors.pop().await {
            return Err(crate::js_error(error));
        }
        Ok(Self {
            surface,
            config,
            renderer,
            backend: backend.to_owned(),
        })
    }

    pub fn backend(&self) -> String {
        self.backend.clone()
    }
}
