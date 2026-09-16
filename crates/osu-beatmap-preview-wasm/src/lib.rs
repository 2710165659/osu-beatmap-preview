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

    // ── 打击音（hit sound） ──────────────────────────────────────────────
    //
    // 音频的整条时间轴都在 WASM 内：宿主只负责「解码样本 → 送进来」和
    // 「按音频时钟取混音结果 → 送出去」。这样画面与声音共用同一份位置计算，
    // 倍速、seek、暂停都只有一处实现。

    /// 打开打击音并指定音量百分比（0～100）。
    ///
    /// `sampleRate` 必须等于宿主音频设备的采样率，混音结果可以直接使用。
    /// 打开后样本库为空，需要按 [`WebGpuSession::hitsound_required_names`] 逐个
    /// 调用 [`WebGpuSession::set_hitsound_sample`]。
    #[wasm_bindgen(js_name = enableHitsound)]
    pub fn enable_hitsound(&mut self, volume_percent: i32, sample_rate: u32) -> Result<(), JsValue> {
        if sample_rate == 0 {
            return Err(JsValue::from_str("音频采样率必须为正数"));
        }
        self.inner
            .enable_hitsound(volume_percent.clamp(0, 100), sample_rate);
        Ok(())
    }

    /// 关闭打击音；关闭后所有混音接口返回静音。
    #[wasm_bindgen(js_name = disableHitsound)]
    pub fn disable_hitsound(&mut self) {
        self.inner.disable_hitsound();
    }

    /// 清空已加载的样本并重建时间轴（保留音量与采样率）。
    ///
    /// 切 Mod 或转谱后需要的样本集合会变，宿主重新调用
    /// [`WebGpuSession::set_hitsound_sample`] 即可，不必重建会话。
    #[wasm_bindgen(js_name = resetHitsoundSamples)]
    pub fn reset_hitsound_samples(&mut self) {
        self.inner.reset_hitsound_samples();
    }

    #[wasm_bindgen(js_name = hitsoundEnabled)]
    pub fn hitsound_enabled(&self) -> bool {
        self.inner.hitsound_enabled()
    }

    /// 当前谱面需要宿主提供 PCM 的样本名（按优先级排列，含裸名回退）。
    ///
    /// 宿主只需下载/解码它认得出来的名字；缺失的样本在混音时按静音处理。
    #[wasm_bindgen(js_name = hitsoundRequiredNames)]
    pub fn hitsound_required_names(&self) -> Vec<String> {
        self.inner.hitsound_required_names()
    }

    /// 当前采样率下是否已有可用样本；没有样本时宿主不必启动音频输出。
    #[wasm_bindgen(js_name = hitsoundHasSamples)]
    pub fn hitsound_has_samples(&self) -> bool {
        self.inner.hitsound_has_samples()
    }

    #[wasm_bindgen(js_name = hitsoundSampleRate)]
    pub fn hitsound_sample_rate(&self) -> u32 {
        self.inner.hitsound_sample_rate()
    }

    /// 放入一段已解码的样本 PCM。
    ///
    /// 只放进样本库；全部放完后必须调用一次
    /// [`WebGpuSession::rebuild_hitsound_timeline`]（否则事件时间轴还是空的）。
    /// 一次批量加载只需重建一次，避免逐个样本遍历整张谱面。
    #[wasm_bindgen(js_name = setHitsoundSample)]
    pub fn set_hitsound_sample(
        &mut self,
        name: &str,
        channels: u32,
        sample_rate: u32,
        loop_length: u32,
        samples: Vec<f32>,
    ) -> Result<(), JsValue> {
        if name.is_empty() {
            return Err(JsValue::from_str("样本名不能为空"));
        }
        if sample_rate == 0 {
            return Err(JsValue::from_str("样本采样率必须为正数"));
        }
        if !matches!(channels, 1 | 2) {
            return Err(JsValue::from_str("样本声道数只能是 1 或 2"));
        }
        // 循环长度以采样帧为单位；交错立体声的数组长度是帧数的两倍。
        let frame_count = if channels == 1 {
            samples.len()
        } else {
            samples.len() / 2
        };
        let loop_length = loop_length.min(frame_count as u32) as usize;
        let data = if channels == 1 {
            osu_beatmap_preview_core::Channels::Mono(samples)
        } else {
            osu_beatmap_preview_core::Channels::Stereo(samples)
        };
        self.inner
            .set_hitsound_sample(name, data, sample_rate, loop_length);
        Ok(())
    }

    /// 用已放入的样本重建打击音事件时间轴（样本全部放完后调用一次）。
    #[wasm_bindgen(js_name = rebuildHitsoundTimeline)]
    pub fn rebuild_hitsound_timeline(&mut self) {
        self.inner.rebuild_hitsound_timeline();
    }

    /// 更新打击音音量百分比（0～100）。
    #[wasm_bindgen(js_name = setHitsoundVolume)]
    pub fn set_hitsound_volume(&mut self, volume_percent: i32) {
        self.inner.set_hitsound_volume(volume_percent.clamp(0, 100));
    }

    /// 把混音位置对齐到谱面时间（毫秒），不清空正在播放的声音。
    ///
    /// 用于宿主音频时钟与 WASM 位置对齐；真正的 seek 请用
    /// [`WebGpuSession::seekHitsound`]。
    #[wasm_bindgen(js_name = positionHitsound)]
    pub fn position_hitsound(&mut self, chart_time_ms: f64) {
        self.inner.position_hitsound(chart_time_ms);
    }

    /// 跳到指定谱面时间并丢弃正在播放的声音。
    #[wasm_bindgen(js_name = seekHitsound)]
    pub fn seek_hitsound(&mut self, chart_time_ms: f64) {
        self.inner.seek_hitsound(chart_time_ms);
    }

    #[wasm_bindgen(js_name = hitsoundPositionMs)]
    pub fn hitsound_position_ms(&self) -> f64 {
        self.inner.hitsound_position_ms()
    }

    /// 从当前混音位置渲染 `frames` 个立体声采样帧到内部缓冲。
    ///
    /// 返回随后可读取的帧数（未启用打击音时为 0）。
    #[wasm_bindgen(js_name = renderHitsound)]
    pub fn render_hitsound(&mut self, frames: u32) -> u32 {
        self.inner.render_hitsound(frames as usize) as u32
    }

    /// 取回最近一次 [`WebGpuSession::render_hitsound`] 的混音结果。
    ///
    /// 返回交错立体声 f32 的副本（长度为帧数 × 2），长度就是上一次渲染请求的帧数。
    /// 用返回值而不是裸指针：wasm-bindgen 会为 `Vec<f32>` 生成能感知内存增长的
    /// 拷贝，宿主拿到的是普通 `Float32Array`，不需要自己维护 `WebAssembly.Memory` 视图。
    #[wasm_bindgen(js_name = takeHitsoundBuffer)]
    pub fn take_hitsound_buffer(&mut self) -> Vec<f32> {
        self.inner.take_hitsound_buffer()
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

/// 按传入的 `.osu` 字节返回打击音需要的样本名（按优先级排列，含裸名回退）。
///
/// 宿主据此决定要解码哪些音效；名字取不到资源时直接忽略即可，混音阶段按静音处理。
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(js_name = hitsoundNames)]
pub fn hitsound_names(bytes: Vec<u8>) -> Result<Vec<String>, JsValue> {
    let beatmap = parse_beatmap_bytes(&bytes).map_err(js_error)?;
    Ok(osu_beatmap_preview_core::hitsound_referenced_names(&beatmap))
}

/// 取回某个打击音样本的 ogg 字节。
///
/// **资源随 wasm 一起分发**（由 core 的 build.rs 内嵌），宿主不需要再向站点请求
/// 音效文件，也就不存在「静态副本没同步导致全部 404」的问题。返回空数组表示没有
/// 对应资源（裸名回退、或本套皮肤不提供的音效），宿主跳过即可。
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(js_name = hitsoundAsset)]
pub fn hitsound_asset(name: &str) -> Vec<u8> {
    osu_beatmap_preview_core::hitsound::asset_bytes(name)
        .map(<[u8]>::to_vec)
        .unwrap_or_default()
}

/// 内嵌打击音资源数量，便于宿主自检。
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(js_name = hitsoundAssetCount)]
pub fn hitsound_asset_count() -> u32 {
    osu_beatmap_preview_core::hitsound::asset_count() as u32
}

/// 返回某个模式在 `assets/shared_config.yml` 里的打击音默认设置。
///
/// CLI 直接读同一份配置，网页端通过这里取值，避免两边各写一份默认值而走偏。
/// 返回 `{ enabled, volume, beatmapEnabled }`：`beatmapEnabled` 对应
/// `ENABLE_BEATMAP_HITSOUND`，表示是否使用谱面自带的自定义打击音。
/// `mode` 接受 `standard` / `taiko` / `catch` / `mania`（也接受 `std` 与 `ctb`）。
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(js_name = hitsoundDefaults)]
pub fn hitsound_defaults(mode: &str) -> Result<JsValue, JsValue> {
    // WASM 里没有外置配置文件，用内嵌的共享配置默认值——它由
    // `assets/shared_config.yml` 在构建时生成，CLI 也读同一份来源。
    let config = osu_beatmap_preview_core::config::CoreConfig::default();
    // 各模式的 style 类型不同，因此这里统一取出（是否启用、音量、是否用谱面自带音效）。
    let (enabled, volume, beatmap_enabled) = match mode.trim().to_ascii_lowercase().as_str() {
        "standard" | "std" => (
            config.render.standard.mp4.style.ENABLE_HITSOUND,
            config.render.standard.mp4.style.HITSOUND_VOLUME,
            config.render.standard.mp4.style.ENABLE_BEATMAP_HITSOUND,
        ),
        "taiko" => (
            config.render.taiko.mp4.style.ENABLE_HITSOUND,
            config.render.taiko.mp4.style.HITSOUND_VOLUME,
            config.render.taiko.mp4.style.ENABLE_BEATMAP_HITSOUND,
        ),
        "catch" | "ctb" => (
            config.render.catch.mp4.style.ENABLE_HITSOUND,
            config.render.catch.mp4.style.HITSOUND_VOLUME,
            config.render.catch.mp4.style.ENABLE_BEATMAP_HITSOUND,
        ),
        "mania" => (
            config.render.mania.mp4.style.ENABLE_HITSOUND,
            config.render.mania.mp4.style.HITSOUND_VOLUME,
            config.render.mania.mp4.style.ENABLE_BEATMAP_HITSOUND,
        ),
        _ => {
            return Err(JsValue::from_str(&format!(
                "未知模式 '{mode}'，可选 standard/taiko/catch/mania"
            )))
        }
    };
    let object = js_sys::Object::new();
    js_sys::Reflect::set(
        &object,
        &JsValue::from_str("enabled"),
        &JsValue::from_bool(enabled),
    )?;
    js_sys::Reflect::set(
        &object,
        &JsValue::from_str("volume"),
        &JsValue::from_f64(volume as f64),
    )?;
    js_sys::Reflect::set(
        &object,
        &JsValue::from_str("beatmapEnabled"),
        &JsValue::from_bool(beatmap_enabled),
    )?;
    Ok(object.into())
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
