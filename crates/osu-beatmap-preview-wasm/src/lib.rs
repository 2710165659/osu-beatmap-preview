//! WebAssembly 适配层。网络请求和 Canvas 生命周期由 JavaScript 宿主负责。

#[cfg(target_arch = "wasm32")]
use osu_beatmap_preview_core::{
    parse_beatmap_bytes, BeatmapInfo, RealtimeOptions, RealtimeSession, ResourceBundle,
};
#[cfg(target_arch = "wasm32")]
use osu_beatmap_preview_renderer::{SurfaceConfig, SurfaceRenderer};
#[cfg(target_arch = "wasm32")]
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// 直接渲染到浏览器 WebGPU Canvas 的会话。
///
/// 浏览器仅把 `.osu` 字节、Canvas 和类型化选项交给 WASM；每一帧都在本地 GPU
/// 中完成，不会返回 RGBA 缓冲或发起任何帧网络请求。
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub struct WebGpuSession {
    inner: RealtimeSession,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    renderer: SurfaceRenderer,
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl WebGpuSession {
    #[wasm_bindgen(js_name = create)]
    pub async fn create(
        bytes: Vec<u8>,
        canvas: web_sys::HtmlCanvasElement,
        options: Option<JsValue>,
    ) -> Result<WebGpuSession, JsValue> {
        let session = create_session(&bytes, options.unwrap_or(JsValue::UNDEFINED))?;
        let (width, height) = session.options().render.dimensions();
        canvas.set_width(width);
        canvas.set_height(height);

        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::BROWSER_WEBGPU;
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(js_error)?;
        // 先要硬件适配器。部分移动设备的 GPU 在浏览器黑名单里，只有 CPU 回退适配器，
        // 而 force_fallback_adapter 为 false 时浏览器会直接返回 null；因此失败后允许
        // 一次回退请求，让这类设备至少能以软件光栅化运行，而不是整页无法渲染。
        let mut adapter_options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        };
        let adapter = match instance.request_adapter(&adapter_options).await {
            Ok(adapter) => adapter,
            Err(hardware_error) => {
                adapter_options.force_fallback_adapter = true;
                instance
                    .request_adapter(&adapter_options)
                    .await
                    .map_err(|fallback_error| {
                        JsValue::from_str(&format!(
                            "没有可用的 WebGPU 适配器（硬件：{hardware_error}；回退：{fallback_error}）。\
                             请确认浏览器支持 WebGPU（Android Chrome 121+、iOS Safari 26+），\
                             并打开 /gpu-check.html 查看具体原因。"
                        ))
                    })?
            }
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("osu beatmap preview webgpu device"),
                ..Default::default()
            })
            .await
            .map_err(js_error)?;
        let surface_config = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| JsValue::from_str("浏览器 WebGPU Canvas 不支持所选适配器"))?;
        let device = Arc::new(device);
        let queue = Arc::new(queue);
        surface.configure(&device, &surface_config);
        let renderer = SurfaceRenderer::new(
            Arc::clone(&device),
            queue,
            SurfaceConfig {
                width,
                height,
                format: surface_config.format,
            },
        )
        .map_err(js_error)?;
        Ok(Self {
            inner: session,
            surface,
            surface_config,
            renderer,
        })
    }

    pub fn render_number(&mut self, absolute_time_ms: f64) -> Result<(), JsValue> {
        if !absolute_time_ms.is_finite() {
            return Err(JsValue::from_str("渲染时间必须是有限数字"));
        }
        let scene = self
            .inner
            .scene_at_absolute(absolute_time_ms.round() as i64)
            .map_err(js_error)?;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(())
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface
                    .configure(self.renderer.device(), &self.surface_config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(JsValue::from_str("WebGPU surface 获取当前帧时发生验证错误"));
            }
        };
        let view = frame.texture.create_view(&Default::default());
        self.renderer
            .render_to_view(&scene, &view)
            .map_err(js_error)?;
        frame.present();
        Ok(())
    }

    pub fn duration_ms_number(&self) -> f64 {
        self.inner.timeline().duration_ms as f64
    }

    pub fn absolute_start_ms_number(&self) -> f64 {
        self.inner.timeline().absolute_start_ms as f64
    }

    pub fn beatmap_speed_number(&self) -> f64 {
        self.inner.timeline().beatmap_speed
    }

    pub fn width(&self) -> u32 {
        self.surface_config.width
    }

    pub fn height(&self) -> u32 {
        self.surface_config.height
    }

    pub fn mode(&self) -> String {
        format!("{:?}", self.inner.mode()).to_lowercase()
    }

    /// 热切换当前会话的 mod，数组中的每项对应一个独立 token。
    pub fn set_mods(&mut self, mods: JsValue) -> Result<(), JsValue> {
        let values = serde_wasm_bindgen::from_value::<Vec<String>>(mods)
            .map_err(|error| JsValue::from_str(&format!("mod 参数无效：{error}")))?;
        self.inner.set_mods(values).map_err(js_error)
    }

    pub fn set_background_rgba(
        &mut self,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<(), JsValue> {
        self.inner
            .set_background(osu_beatmap_preview_core::ImageData {
                width,
                height,
                rgba,
            })
            .map_err(js_error)
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), JsValue> {
        if width == 0 || height == 0 {
            return Err(JsValue::from_str("Canvas 尺寸必须为正数"));
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface
            .configure(self.renderer.device(), &self.surface_config);
        self.renderer.resize(width, height).map_err(js_error)?;
        // 合成场景的尺寸由 core 的 RealtimeOptions 决定；只调整 surface 会让
        // 画面仍按旧尺寸渲染并贴在左上角，因此需要同步更新 core。
        self.inner.set_render_size(width, height).map_err(js_error)
    }
}

/// 解析 `.osu` 字节并返回谱面内部信息（全量字段）。
///
/// WASM 侧没有网络能力，`bid` 的下载由宿主完成（浏览器取 `/resource/beatmap`、
/// Node 后端取本地缓存），这里只按传入的谱面字节解析，返回的对象字段与
/// [`BeatmapInfo`] 一一对应。
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(js_name = beatmapInfo)]
pub fn beatmap_info(bytes: Vec<u8>) -> Result<JsValue, JsValue> {
    let beatmap = parse_beatmap_bytes(&bytes).map_err(js_error)?;
    let info = BeatmapInfo::from_beatmap(&beatmap);
    // 默认序列化会把 map 变成 JS Map、把 None 变成 undefined；这里统一成
    // 普通对象与 null，宿主可以直接用 `info.title` 取值。
    let serializer = serde_wasm_bindgen::Serializer::new()
        .serialize_maps_as_objects(true)
        .serialize_missing_as_null(true);
    serde::Serialize::serialize(&info, &serializer).map_err(js_error)
}

#[cfg(target_arch = "wasm32")]
fn create_session(bytes: &[u8], options: JsValue) -> Result<RealtimeSession, JsValue> {
    let beatmap = parse_beatmap_bytes(bytes).map_err(js_error)?;
    let mut realtime = RealtimeOptions::default();
    if !options.is_undefined() && !options.is_null() {
        if let Ok(value) = js_sys::Reflect::get(&options, &JsValue::from_str("convert")) {
            realtime.convert = value.as_string();
        }
        if let Ok(value) = js_sys::Reflect::get(&options, &JsValue::from_str("mods")) {
            if let Ok(values) = serde_wasm_bindgen::from_value::<Vec<String>>(value) {
                realtime.mods = values;
            }
        }
        if let Ok(value) = js_sys::Reflect::get(&options, &JsValue::from_str("width")) {
            realtime.render.width = value.as_f64().unwrap_or(0.0) as u32;
        }
        if let Ok(value) = js_sys::Reflect::get(&options, &JsValue::from_str("height")) {
            realtime.render.height = value.as_f64().unwrap_or(0.0) as u32;
        }
    }
    RealtimeSession::from_bundle(ResourceBundle::new(beatmap), realtime).map_err(js_error)
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
