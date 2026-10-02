//! WebAssembly 适配层：**单文件输入**（`.osu` / `.osz`）→ 实时预览。
//!
//! 数据流只有一条：宿主把一份文件的字节（`.osu` 或 `.osz` 整包）连同 Canvas 交给
//! [`WebGpuSession::create`]，其余全部在 WASM 内完成——`.osz` 解包（音乐、背景、
//! 自带音效）、音频解码、音乐与音效统一混音、时钟推进。宿主只做四件事：
//! 取字节、给 Canvas、把混音结果送进音频设备、把音频线程的消费位置转发回来。
//!
//! 时间权威是会话时钟（[`osu_beatmap_preview_core::PreviewClock`]）：`play` / `pause` /
//! `seek` / `setRate` / `clockMs` 全部在 WASM 里，宿主不再计算时间，也没有
//! 「静音 WAV + `<audio>` 元素」这类假时钟。

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

pub mod archive;
pub mod decode;
pub mod video;

#[cfg(target_arch = "wasm32")]
use osu_beatmap_preview_core::{
    hitsound, parse_beatmap_bytes, BeatmapInfo, ImageData, RealtimeOptions, RealtimeSession,
    ResourceBundle, StoryboardBundle,
};
#[cfg(target_arch = "wasm32")]
use osu_beatmap_preview_renderer::{SurfaceConfig, SurfaceRenderer};

/// 墙钟读数（毫秒）：时钟推进与音频位置锚定共用。
#[cfg(target_arch = "wasm32")]
fn wall_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now())
        .unwrap_or_else(js_sys::Date::now)
}

/// 连续拿不到 surface 超过这个时长就报错，不再静默黑屏。
///
/// 取帧偶尔超时（GPU 忙、窗口在切换）是正常的，一秒左右还拿不到才算真的坏了。
#[cfg(target_arch = "wasm32")]
const SURFACE_FAILURE_LIMIT_MS: f64 = 1000.0;

/// 背景视频帧的外部纹理槽位（renderer 的 `copy_external_frame` 按槽位号填纹理，
/// 帧像素不进 CPU 内存）。
#[cfg(target_arch = "wasm32")]
const VIDEO_TEXTURE_SLOT: u32 = 0;

/// 直接渲染到浏览器 WebGPU Canvas 的会话。
///
/// 创建时一次性收下整份文件；音乐与打击音在 WASM 内统一混音，画面与声音共用同一条
/// 时间轴（会话时钟），倍速、seek、暂停都只有一处位置计算。
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub struct WebGpuSession {
    inner: RealtimeSession,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    renderer: SurfaceRenderer,
    /// `.osz` 里的自带音效条目（条目名 + 字节）；切 Mod 后按新候选名重新匹配装载。
    custom_samples: Vec<(String, Vec<u8>)>,
    /// 是否采用谱面自带音效（`ENABLE_BEATMAP_HITSOUND`）。
    use_beatmap_samples: bool,
    /// 背景视频解码器（`.osz` 里的 mp4）；缺失或不受支持时为 `None`。
    video: Option<video::BackgroundVideo>,
    /// 背景视频开关（默认关闭）：开启后逐帧解码并叠在静态背景上。
    video_enabled: bool,
    /// 背景视频层当前是否已有画面（决定要不要发一次 clear）。
    video_showing: bool,
    /// 连续拿不到 surface 的起始墙钟时间；拿得到时清空。
    ///
    /// 只用来把「画布一直画不出来」变成宿主能看到的错误：以前这种情况是每帧
    /// `Ok(())` 静默跳过，画面永远停在黑色（或旧帧）而音频照常在放，
    /// 用户只看到黑屏且日志里没有任何线索。
    surface_failure_since: Option<f64>,
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl WebGpuSession {
    /// 用一份文件创建会话：`.osu` 字节（只有谱面与内嵌音效）或 `.osz` 整包字节
    /// （音乐、背景、自带音效全部在 WASM 内解出）。
    ///
    /// `options`：`{ bid?, difficulty?, convert?, mods?, width, height, sampleRate?,
    /// hitsoundEnabled?, hitsoundVolume?, musicVolume?, beatmapHitsound?, storyboard? }`。
    /// 难度选择优先 `difficulty`（压缩包条目名），其次 `bid`（`BeatmapID`），
    /// 都没有时取第一个顶层 `.osu`。`sampleRate` 必须等于宿主音频设备的实际采样率。
    /// `storyboard` 默认关闭，开启后绘制 `.osb` / `[Events]` 的故事板层。
    #[wasm_bindgen(js_name = create)]
    pub async fn create(
        bytes: Vec<u8>,
        canvas: web_sys::HtmlCanvasElement,
        options: Option<JsValue>,
    ) -> Result<WebGpuSession, JsValue> {
        let options = options.unwrap_or(JsValue::UNDEFINED);
        let content = archive::read_input(&bytes, &read_selector(&options), true)
            .map_err(|error| JsValue::from_str(&error))?;
        let beatmap = parse_beatmap_bytes(&content.beatmap).map_err(js_error)?;

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
            if let Ok(value) = js_sys::Reflect::get(&options, &JsValue::from_str("sampleRate")) {
                if let Some(rate) = value.as_f64() {
                    realtime.audio.sample_rate = rate.max(1.0) as u32;
                }
            }
            if let Ok(value) = js_sys::Reflect::get(&options, &JsValue::from_str("hitsoundEnabled"))
            {
                realtime.audio.hitsound_enabled = value.as_bool().unwrap_or(true);
            }
            if let Ok(value) = js_sys::Reflect::get(&options, &JsValue::from_str("hitsoundVolume"))
            {
                realtime.audio.hitsound_volume = value.as_f64().unwrap_or(100.0) as i32;
            }
            if let Ok(value) = js_sys::Reflect::get(&options, &JsValue::from_str("musicVolume")) {
                realtime.audio.music_volume = value.as_f64().unwrap_or(50.0) as i32;
            }
        }
        let use_beatmap_samples = option_bool(&options, "beatmapHitsound").unwrap_or(true);

        // 背景视频的时间偏移来自 `Video` 事件；视频本体由会话按需解码。
        let video_start_ms = beatmap
            .video
            .as_ref()
            .map(|video| video.start_ms)
            .unwrap_or(0);

        // 背景在 WASM 内解码后直接进合成；解不出来退化成纯色背景。
        let mut bundle = ResourceBundle::new(beatmap);
        if let Some(background) = content.background.as_deref() {
            bundle.background = decode::decode_background(background);
        }
        // 故事板（默认关闭，`storyboard` 选项开启）：`.osu` 的 `[Events]` 与包内
        // `.osb` 一起解析，贴图按引用路径从同一份 `.osz` 取出并解码。osu! 的
        // ReplacesBackground（背景层有同名精灵时接管背景）由会话在合成时按
        // 开关联动——这里不丢背景资源，关闭故事板后背景照常显示。
        bundle.storyboard =
            build_storyboard(&content, &bytes).map_err(|error| JsValue::from_str(&error))?;
        realtime.storyboard_enabled = option_bool(&options, "storyboard").unwrap_or(false);
        // 背景暗化系数同步给会话：故事板精灵的亮度（1 − dim）由此而来，
        // 与 `decode_background` 的预暗化同一语义。
        realtime.video_style.background_dim = decode::BACKGROUND_DIM;
        let mut session = RealtimeSession::from_bundle(bundle, realtime).map_err(js_error)?;

        // 音乐解码失败是致命错误（没有声音的整包预览没有意义）；
        // 单独的 `.osu` 没有音乐，时钟照常自驱。
        if let Some(audio) = content.audio.as_deref() {
            let music = decode::decode_music(audio, extension_of(&content.audio_name))
                .map_err(|error| JsValue::from_str(&error))?;
            session.set_music(Some(music));
        }
        load_samples(&mut session, &content.samples, use_beatmap_samples);

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
        let device = std::sync::Arc::new(device);
        let queue = std::sync::Arc::new(queue);
        surface.configure(&device, &surface_config);
        let renderer = SurfaceRenderer::new(
            std::sync::Arc::clone(&device),
            queue,
            SurfaceConfig {
                width,
                height,
                format: surface_config.format,
            },
        )
        .map_err(js_error)?;
        // 背景视频在 WASM 内驱动浏览器硬解（`video.rs`）；容器/编码不受支持时
        // 静默回退背景图，与 osu! 的降级一致。抓帧分辨率跟随后端画布并封顶 720p。
        let mut video = content
            .video
            .map(|bytes| {
                video::BackgroundVideo::open(bytes, video_start_ms, 1.0 - decode::BACKGROUND_DIM)
            })
            .and_then(Result::ok);
        if let Some(video) = video.as_mut() {
            video.set_capture_limit(width, height);
        }
        Ok(Self {
            inner: session,
            surface,
            surface_config,
            renderer,
            custom_samples: content.samples,
            use_beatmap_samples,
            video,
            video_enabled: false,
            video_showing: false,
            surface_failure_since: None,
        })
    }

    // ── 时钟（WASM 内部维护，宿主不再计算时间） ─────────────────────────

    /// 开始播放：时钟从当前时刻继续推进。
    pub fn play(&mut self) {
        self.inner.play(wall_ms());
    }

    /// 暂停：把当前时刻固化。
    pub fn pause(&mut self) {
        self.inner.pause(wall_ms());
    }

    /// 跳到指定谱面绝对时间（毫秒，0 = 音频文件 0 点，可为负）。
    pub fn seek(&mut self, chart_ms: f64) -> Result<(), JsValue> {
        if !chart_ms.is_finite() {
            return Err(JsValue::from_str("跳转时间必须是有限数字"));
        }
        self.inner.seek(wall_ms(), chart_ms);
        Ok(())
    }

    /// 设置用户倍速（0.5 / 1 / 2……，不含 DT/HT；总倍速由会话乘上谱面变速）。
    #[wasm_bindgen(js_name = setRate)]
    pub fn set_rate(&mut self, user_rate: f64) -> Result<(), JsValue> {
        if !user_rate.is_finite() || user_rate <= 0.0 {
            return Err(JsValue::from_str("倍速必须是正的有限数字"));
        }
        self.inner.set_rate(wall_ms(), user_rate);
        Ok(())
    }

    /// 当前谱面绝对时间（毫秒）；画面按它渲染。
    #[wasm_bindgen(js_name = clockMs)]
    pub fn clock_ms(&self) -> f64 {
        self.inner.clock_ms(wall_ms())
    }

    /// 当前总倍速（用户倍速 × 谱面变速），即音频线程的消费速率。
    #[wasm_bindgen(js_name = rate)]
    pub fn total_rate(&self) -> f64 {
        self.inner.rate()
    }

    pub fn playing(&self) -> bool {
        self.inner.playing()
    }

    // ── 输出 ───────────────────────────────────────────────────────────

    /// 按内部时钟渲染一帧到 Canvas。
    ///
    /// 返回 `false` 表示这一帧没画：画布被浏览器标记为不可见（窗口被遮挡/最小化）
    /// 或 GPU 暂时取不到帧。宿主据此可以说明「为什么画面是黑的」，而不是让用户
    /// 面对一块没有线索的黑画布。
    #[wasm_bindgen(js_name = renderFrame)]
    pub fn render_frame(&mut self) -> Result<bool, JsValue> {
        let chart_ms = self.inner.clock_ms(wall_ms()).round() as i64;
        self.update_background_video(chart_ms);
        let scene = self.inner.scene_at_absolute(chart_ms).map_err(js_error)?;
        let Some(frame) = self.acquire_surface()? else {
            return Ok(false);
        };
        let view = frame.texture.create_view(&Default::default());
        self.renderer
            .render_to_view(&scene, &view)
            .map_err(js_error)?;
        frame.present();
        Ok(true)
    }

    /// 取一帧可绘制的 surface 纹理；`None` 表示这一帧跳过。
    ///
    /// `Outdated` / `Lost` 表示 surface 配置与画布已经不一致（换分辨率、窗口迁移、
    /// 设备回收），这里重新配置后**立刻重试一次**：以前直接返回会白白黑掉一帧，
    /// 而状态一直不变时就变成永久黑屏。
    ///
    /// `Occluded`（页面被遮挡）跳过是正常的；其余状态连续失败超过
    /// [`SURFACE_FAILURE_LIMIT_MS`] 就报错，让宿主停下来提示用户，
    /// 而不是留下一块黑画布继续放声音。
    fn acquire_surface(&mut self) -> Result<Option<wgpu::SurfaceTexture>, JsValue> {
        let mut last_reason = "surface 不可用";
        for attempt in 0..2 {
            match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    self.surface_failure_since = None;
                    return Ok(Some(frame));
                }
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    last_reason = "surface 过期或丢失";
                    self.surface
                        .configure(self.renderer.device(), &self.surface_config);
                    if attempt == 1 {
                        break;
                    }
                }
                wgpu::CurrentSurfaceTexture::Timeout => {
                    last_reason = "GPU 取帧超时";
                    if attempt == 1 {
                        break;
                    }
                }
                // 页面被遮挡时没有可显示的目标（窗口最小化、切到别的标签页）：
                // 跳过这一帧是预期行为，时钟与音频继续推进，恢复可见后自动续上。
                wgpu::CurrentSurfaceTexture::Occluded => {
                    self.surface_failure_since = None;
                    return Ok(None);
                }
                wgpu::CurrentSurfaceTexture::Validation => {
                    return Err(JsValue::from_str("WebGPU surface 获取当前帧时发生验证错误"));
                }
            }
        }
        let now = wall_ms();
        let Some(since) = self.surface_failure_since else {
            self.surface_failure_since = Some(now);
            return Ok(None);
        };
        if now - since >= SURFACE_FAILURE_LIMIT_MS {
            self.surface_failure_since = None;
            return Err(JsValue::from_str(&format!(
                "WebGPU 画布已连续 {:.1} 秒无法出帧（{last_reason}），画面已停止；\
                 请刷新页面重新加载预览",
                (now - since) / 1000.0
            )));
        }
        Ok(None)
    }

    /// 补一段「音乐 + 打击音」统一混音，返回交错立体声 f32（帧数 × 2，可能为空）。
    ///
    /// 补多少由预读窗口决定；宿主把返回数据接在环形缓冲写入前沿之后即可。
    /// 返回后请先检查 [`WebGpuSession::audio_epoch`]：纪元变化表示输出流已整体重置，
    /// 必须把环形读写指针一起归零并通知音频线程从头重读。
    #[wasm_bindgen(js_name = pullAudio)]
    pub fn pull_audio(&mut self, max_frames: u32) -> Vec<f32> {
        self.inner.pull_audio(wall_ms(), max_frames.max(1) as usize)
    }

    /// 转发音频线程回报的消费位置（自上次重置起已消费的采样帧）。
    ///
    /// 这是画面时钟锚定的依据：有音频输出时画面贴着「此刻听到的位置」走，音画不分家。
    #[wasm_bindgen(js_name = onAudioClock)]
    pub fn on_audio_clock(&mut self, consumed_frames: f64) {
        if consumed_frames.is_finite() && consumed_frames >= 0.0 {
            self.inner.on_audio_clock(consumed_frames as u64, wall_ms());
        }
    }

    /// 混音输出流的重置纪元（宿主据此归零环形读写指针）。
    #[wasm_bindgen(js_name = audioEpoch)]
    pub fn audio_epoch(&self) -> f64 {
        self.inner.audio_epoch() as f64
    }

    // ── 调节 ───────────────────────────────────────────────────────────

    /// 更新音乐音量百分比（0～100）。
    #[wasm_bindgen(js_name = setMusicVolume)]
    pub fn set_music_volume(&mut self, volume_percent: i32) {
        self.inner.set_music_volume(volume_percent.clamp(0, 100));
    }

    /// 更新打击音音量百分比（0～100）。
    #[wasm_bindgen(js_name = setHitsoundVolume)]
    pub fn set_hitsound_volume(&mut self, volume_percent: i32) {
        self.inner.set_hitsound_volume(volume_percent.clamp(0, 100));
    }

    /// 打开/关闭打击音；关闭时音乐照常输出。
    #[wasm_bindgen(js_name = setHitsoundEnabled)]
    pub fn set_hitsound_enabled(&mut self, enabled: bool) {
        self.inner.set_hitsound_enabled(enabled);
    }

    /// 热切换当前会话的 mod，数组中的每项对应一个独立 token。
    ///
    /// 转谱会改变目标模式与所需样本，切换后按新谱面重新装载样本并重建时间轴。
    pub fn set_mods(&mut self, mods: JsValue) -> Result<(), JsValue> {
        let values = serde_wasm_bindgen::from_value::<Vec<String>>(mods)
            .map_err(|error| JsValue::from_str(&format!("mod 参数无效：{error}")))?;
        self.inner.set_mods(values, wall_ms()).map_err(js_error)?;
        let Self {
            inner,
            custom_samples,
            use_beatmap_samples,
            ..
        } = self;
        load_samples(inner, custom_samples, *use_beatmap_samples);
        Ok(())
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
        // 抓帧分辨率跟随后端画布（再被 720p 上限截断）；渲染器重建会丢掉
        // 外部纹理，强制重新取帧重拷纹理，否则视频层在重建后是空的。
        if let Some(video) = self.video.as_mut() {
            video.set_capture_limit(width, height);
            video.invalidate_capture();
        }
        // 合成场景的尺寸由 core 的 RealtimeOptions 决定；只调整 surface 会让
        // 画面仍按旧尺寸渲染并贴在左上角，因此需要同步更新 core。
        self.inner.set_render_size(width, height).map_err(js_error)
    }

    // ── 背景视频（默认关闭；开启后逐帧解码并叠在背景图上） ─────────────

    /// 谱面是否有可用的背景视频（`.osz` 里取到了可解的 mp4）。
    #[wasm_bindgen(js_name = hasBackgroundVideo)]
    pub fn has_background_video(&self) -> bool {
        self.video.is_some()
    }

    /// 开关背景视频（默认关闭）；这份谱面没有可用视频时保持关闭。
    #[wasm_bindgen(js_name = setBackgroundVideo)]
    pub fn set_background_video(&mut self, enabled: bool) {
        self.video_enabled = enabled && self.video.is_some();
        if !self.video_enabled && self.video_showing {
            self.video_showing = false;
            self.inner.clear_background_video();
        }
    }

    // ── 故事板（默认关闭；开启后按 .osb / [Events] 合成故事板层） ───────

    /// 这份谱面是否有可绘制的故事板元素（决定开关是否可用）。
    #[wasm_bindgen(js_name = hasStoryboard)]
    pub fn has_storyboard(&self) -> bool {
        self.inner.has_storyboard()
    }

    /// 开关故事板绘制（默认关闭）；没有故事板素材时切换无效果。
    #[wasm_bindgen(js_name = setStoryboard)]
    pub fn set_storyboard(&mut self, enabled: bool) {
        self.inner.set_storyboard_enabled(enabled);
    }

    /// 按当前时钟推进背景视频层：只在画面真的变化时才拷贝/写入会话。
    ///
    /// 时间对齐由 [`video::BackgroundVideo`] 驱动（与 CLI 导出同一套语义）；
    /// 新画面走 GPU 直拷（`copy_external_frame`，帧像素不进 CPU），淡入淡出
    /// 的可见度由会话在合成时按帧时间计算——同一画面不会重复拷贝。
    fn update_background_video(&mut self, chart_ms: i64) {
        let playing = self.inner.playing();
        let rate = self.inner.rate();
        let Some(video) = self.video.as_mut() else {
            return;
        };
        if !self.video_enabled || video.visibility_alpha(chart_ms) <= 0.0 {
            if self.video_showing {
                self.video_showing = false;
                self.inner.clear_background_video();
            }
            return;
        }
        match video.capture_at(chart_ms - video.start_ms, playing, rate) {
            video::Capture::Source(canvas) => {
                // 主路径：画布直接拷进外部纹理槽位（GPU→GPU），帧像素不进 CPU。
                let source = wgpu::ExternalImageSource::HTMLCanvasElement(canvas);
                if self
                    .renderer
                    .copy_external_frame(VIDEO_TEXTURE_SLOT, &source)
                    .is_err()
                {
                    return;
                }
                let video_time_ms = chart_ms - video.start_ms;
                let duration_ms = video.duration_ms();
                self.inner.set_background_video_external(
                    VIDEO_TEXTURE_SLOT,
                    video_time_ms,
                    duration_ms,
                );
                self.video_showing = true;
            }
            video::Capture::Pixels(frame) => {
                // 回退路径（老浏览器没有 canvas filter）：像素移入会话合成。
                let image = ImageData {
                    width: frame.width,
                    height: frame.height,
                    rgba: frame.rgba,
                };
                let video_time_ms = chart_ms - video.start_ms;
                let duration_ms = video.duration_ms();
                if self
                    .inner
                    .set_background_video(image, video_time_ms, duration_ms)
                    .is_ok()
                {
                    self.video_showing = true;
                }
            }
            video::Capture::Unchanged => {}
            video::Capture::Missing => {
                if self.video_showing {
                    self.video_showing = false;
                    self.inner.clear_background_video();
                }
            }
        }
    }

    // ── 信息 ───────────────────────────────────────────────────────────

    #[wasm_bindgen(js_name = durationMs)]
    pub fn duration_ms(&self) -> f64 {
        self.inner.timeline().duration_ms as f64
    }

    #[wasm_bindgen(js_name = absoluteStartMs)]
    pub fn absolute_start_ms(&self) -> f64 {
        self.inner.timeline().absolute_start_ms as f64
    }

    #[wasm_bindgen(js_name = beatmapSpeed)]
    pub fn beatmap_speed(&self) -> f64 {
        self.inner.timeline().beatmap_speed
    }

    #[wasm_bindgen(js_name = audioSampleRate)]
    pub fn audio_sample_rate(&self) -> u32 {
        self.inner.audio_sample_rate()
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
}

/// 按「谱面自带条目 > 内嵌皮肤」的优先级装载当前谱面需要的样本。
///
/// 候选名由谱面内容决定（`hitsound_required_names`），每个候选按
/// [`osu_beatmap_preview_core::sample_entry_matches`] 在 `.osz` 条目里找同名文件；
/// 找不到（或解不出有效音频）才回落到 wasm 内嵌皮肤，两边都没有按静音处理。
/// 装载完成后重建一次事件时间轴（逐样本重建会反复遍历整张谱面）。
#[cfg(target_arch = "wasm32")]
fn load_samples(
    session: &mut RealtimeSession,
    custom: &[(String, Vec<u8>)],
    use_beatmap_samples: bool,
) {
    session.reset_hitsound_samples();
    for name in session.hitsound_required_names() {
        let mut loaded = false;
        if use_beatmap_samples {
            if let Some((entry, bytes)) = custom
                .iter()
                .find(|(entry, _)| osu_beatmap_preview_core::sample_entry_matches(entry, &name))
            {
                let data = decode::decode_sample(&name, bytes, extension_of(entry));
                // 空样本视为「取不到」：回落内嵌皮肤，与旧 Web 端的解码失败语义一致
                //（argon pro 的静音滑行音本身就是内嵌资源，回落后仍是静音）。
                if data.frames() > 0 {
                    session.set_hitsound_sample(
                        &name,
                        data.channels,
                        data.sample_rate,
                        data.loop_len,
                    );
                    loaded = true;
                }
            }
        }
        if !loaded {
            if let Some(bytes) = hitsound::asset_bytes(&name) {
                let data = decode::decode_sample(&name, bytes, Some("ogg"));
                if data.frames() > 0 {
                    session.set_hitsound_sample(
                        &name,
                        data.channels,
                        data.sample_rate,
                        data.loop_len,
                    );
                }
            }
        }
    }
    session.rebuild_hitsound_timeline();
}

/// 从解包内容装配故事板资源：解析 `.osu` `[Events]` 与 `.osb`，并解码引用贴图。
///
/// 没有可绘制元素返回 `None`；贴图缺失或解码失败按缺图静默跳过（与 osu! 一致）。
#[cfg(target_arch = "wasm32")]
fn build_storyboard(
    content: &archive::ArchiveContent,
    bytes: &[u8],
) -> Result<Option<StoryboardBundle>, String> {
    let osu_text = String::from_utf8_lossy(&content.beatmap);
    let osb_text = content
        .osb
        .as_deref()
        .map(|raw| String::from_utf8_lossy(raw).into_owned());
    let storyboard =
        osu_beatmap_preview_core::storyboard::parse_storyboard(&osu_text, osb_text.as_deref());
    if !storyboard.has_drawable_elements() {
        return Ok(None);
    }
    let paths = storyboard.referenced_paths();
    let raw = archive::read_storyboard_textures(bytes, &paths)?;
    let textures = raw
        .into_iter()
        .filter_map(|(path, raw)| decode::decode_image(&raw).map(|image| (path, image)))
        .collect();
    Ok(Some(StoryboardBundle {
        storyboard,
        textures,
    }))
}

/// 解析难度选择：`difficulty`（条目名）优先，其次 `bid`（`BeatmapID`），都没有取第一个。
#[cfg(target_arch = "wasm32")]
fn read_selector(options: &JsValue) -> archive::DifficultySelector {
    if let Some(entry) = option_string(options, "difficulty") {
        if !entry.trim().is_empty() {
            return archive::DifficultySelector::ByEntry(entry);
        }
    }
    if let Ok(value) = js_sys::Reflect::get(options, &JsValue::from_str("bid")) {
        if let Some(bid) = value.as_f64() {
            if bid.is_finite() && bid > 0.0 {
                return archive::DifficultySelector::ById(bid as u64);
            }
        }
        if let Some(text) = value.as_string() {
            if let Ok(bid) = text.trim().parse::<u64>() {
                return archive::DifficultySelector::ById(bid);
            }
        }
    }
    archive::DifficultySelector::First
}

#[cfg(target_arch = "wasm32")]
fn option_string(options: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(options, &JsValue::from_str(key))
        .ok()
        .and_then(|value| value.as_string())
}

#[cfg(target_arch = "wasm32")]
fn option_bool(options: &JsValue, key: &str) -> Option<bool> {
    js_sys::Reflect::get(options, &JsValue::from_str(key))
        .ok()
        .and_then(|value| value.as_bool())
}

/// 条目扩展名（小写、不含点），用作 symphonia 的格式提示。
#[cfg(target_arch = "wasm32")]
fn extension_of(name: &str) -> Option<&str> {
    name.rsplit_once('.').map(|(_, extension)| extension)
}

/// 解析 `.osu` / `.osz` 字节并返回谱面内部信息（全量字段）与难度清单。
///
/// `options` 与 [`WebGpuSession::create`] 的难度选择一致（`bid` / `difficulty`）；
/// 单独的 `.osu` 没有难度清单（返回空数组），`.osz` 里有几行就返回几行，
/// 供加载页直接渲染难度下拉。
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(js_name = beatmapInfo)]
pub fn beatmap_info(bytes: Vec<u8>, options: Option<JsValue>) -> Result<JsValue, JsValue> {
    let options = options.unwrap_or(JsValue::UNDEFINED);
    let content = archive::read_input(&bytes, &read_selector(&options), false)
        .map_err(|error| JsValue::from_str(&error))?;
    let beatmap = parse_beatmap_bytes(&content.beatmap).map_err(js_error)?;
    let info = BeatmapInfo::from_beatmap(&beatmap);
    // 默认序列化会把 map 变成 JS Map、把 None 变成 undefined；这里统一成
    // 普通对象与 null，宿主可以直接用 `info.title` 取值。
    let serializer = serde_wasm_bindgen::Serializer::new()
        .serialize_maps_as_objects(true)
        .serialize_missing_as_null(true);
    let value = serde::Serialize::serialize(&info, &serializer).map_err(js_error)?;
    // 单独的 `.osu` 不显示难度清单；`.osz` 的清单来自全部顶层 `.osu`。
    let difficulties = if is_plain_osu(&bytes) {
        js_sys::Array::new()
    } else {
        let list = js_sys::Array::new();
        for difficulty in &content.difficulties {
            let row = js_sys::Object::new();
            js_sys::Reflect::set(
                &row,
                &JsValue::from_str("entry"),
                &JsValue::from_str(&difficulty.entry),
            )?;
            js_sys::Reflect::set(
                &row,
                &JsValue::from_str("label"),
                &JsValue::from_str(&difficulty.label),
            )?;
            js_sys::Reflect::set(
                &row,
                &JsValue::from_str("beatmapId"),
                &match difficulty.beatmap_id {
                    Some(id) => JsValue::from_f64(id as f64),
                    None => JsValue::NULL,
                },
            )?;
            list.push(&row);
        }
        list
    };
    js_sys::Reflect::set(&value, &JsValue::from_str("difficulties"), &difficulties)?;
    Ok(value)
}

/// 字节是否是单独的 `.osu`（而不是 `.osz` 整包）。
#[cfg(target_arch = "wasm32")]
fn is_plain_osu(bytes: &[u8]) -> bool {
    !bytes.starts_with(b"PK\x03\x04")
        && !bytes.starts_with(b"PK\x05\x06")
        && !bytes.starts_with(b"PK\x07\x08")
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

/// 返回某个模式在实时预览里可选的 Mod token 列表（展示顺序即数组顺序）。
///
/// 列表来自 core 的支持矩阵，网页端直接拿它渲染 Mod 面板：两边各写一份很容易
/// 在新增 Mod（例如 HD/FL）后走偏。列表中 `DA` 需要调用方补参数后提交，
/// 键数 Mod 已经展开成 `1K`…`10K`。
/// `mode` 接受 `standard` / `taiko` / `catch` / `mania`（也接受 `std` 与 `ctb`）。
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(js_name = supportedMods)]
pub fn supported_mods(mode: &str) -> Result<js_sys::Array, JsValue> {
    let key = mode.trim().to_ascii_lowercase();
    let mode_index = match key.as_str() {
        "standard" | "std" => 0,
        "taiko" => 1,
        "catch" | "ctb" => 2,
        "mania" => 3,
        _ => {
            return Err(JsValue::from_str(&format!(
                "未知模式 '{mode}'，可选 standard/taiko/catch/mania"
            )))
        }
    };
    let list = js_sys::Array::new();
    for token in osu_beatmap_preview_core::model::mods::supported_mod_tokens(mode_index) {
        list.push(&JsValue::from_str(&token));
    }
    Ok(list)
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
