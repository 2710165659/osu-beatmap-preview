//! 谱面背景视频：wasm 内驱动浏览器 `<video>` 硬解 → 逐帧抓取 RGBA。
//!
//! 视频管线整体收在这个模块里（创建元素、对时、抓帧都是 Rust 代码），前端只调
//! `setBackgroundVideo` 开关。像素由浏览器媒体栈的**硬件解码器**提供——这里
//! 刻意不用 wasm 软解：实测纯 Rust 解码 720p 需要 60～145 ms/帧（无 SIMD 的
//! 标量路径，wasm 同款），而 720p30 实时播放要求 ≤33 ms/帧，软解只能放成幻灯片。
//!
//! 抓帧链路按实时预算做过优化（目标是不拖慢 60 FPS 渲染循环）：
//! - **分辨率封顶**：抓帧画布 = 视频原生尺寸与「预览画布、720p 上限」取小——
//!   每帧的画布回读、拷贝与纹理上传都随像素数线性增长；
//! - **暗化在 GPU 完成**：`drawImage` 挂 canvas `filter: brightness(...)`；
//!   老浏览器不支持 filter 时回退整数查找表暗化（不是浮点逐像素循环）；
//! - **只在换帧时抓**：`<video>` 的呈现帧变化才抓一次，未变化返回
//!   [`Capture::Unchanged`]，调用方不再重复上传像素。
//!
//! 行为语义与 CLI 的 `media/background_video.rs` 一致（对齐 osu! 的
//! `DrawableStoryboardVideo`）：
//! - 视频时间 = 谱面时间 − `Video` 事件偏移（[`BackgroundVideo::start_ms`]）；
//! - 开始处淡入、结束前淡出，窗口外不参与合成（可见度公式在 core 的
//!   `render::wgpu::composition::visibility_alpha`，与 CLI 同一份语义）。

use osu_beatmap_preview_core::render::wgpu::composition::visibility_alpha;
use wasm_bindgen::JsCast;

/// 抓帧分辨率硬上限：暗化背景不需要超过 720p 的细节，帧率优先。
const MAX_CAPTURE_WIDTH: u32 = 1280;
const MAX_CAPTURE_HEIGHT: u32 = 720;

/// 抓取结果：只有画面真的变了才值得走「上传 → 合成」的整条链路。
pub enum Capture {
    /// 有新画面（已按 [`BackgroundVideo`] 的尺寸约定缩放、暗化）。
    New(VideoFrame),
    /// 画面没变：沿用已推送的帧。
    Unchanged,
    /// 还没有画面（视频未开始/未就绪/解不出来），调用方回退背景图。
    Missing,
}

/// 抓取的一帧（RGBA8，不透明、已暗化）。
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// 背景视频：隐藏的 `<video>`（硬解）+ 隐藏画布（抓帧）。
///
/// `<video>` 挂在 `body` 下的 1×1 近乎透明层里：脱离文档的视频在部分浏览器
/// 不会被解码/呈现，而 `display:none` 又可能让它停止出帧。
pub struct BackgroundVideo {
    element: web_sys::HtmlVideoElement,
    canvas: web_sys::HtmlCanvasElement,
    context: web_sys::CanvasRenderingContext2d,
    object_url: String,
    /// 上次抓帧的视频时间（秒）与「是否已有画面」：时间没变就跳过抓帧，
    /// 像素只有一份、直接交给调用方，不在这里缓存副本。
    captured_time_s: f64,
    has_frame: bool,
    /// 抓帧分辨率上限（预览画布尺寸与 [`MAX_CAPTURE_WIDTH`]×[`MAX_CAPTURE_HEIGHT`] 取小）。
    capture_limit: (u32, u32),
    /// 暗化亮度（= 1 − `BACKGROUND_DIM`）；canvas filter 不可用时走 CPU 回退。
    dim_brightness: f64,
    /// canvas filter（GPU 暗化）是否可用。
    dim_on_gpu: bool,
    /// `Video` 事件偏移：视频第 0 帧对应的谱面时间（毫秒）。
    pub start_ms: i64,
}

impl BackgroundVideo {
    /// 用 `.osz` 里的视频字节创建播放器；创建失败返回错误，调用方回退背景图。
    ///
    /// `dim_brightness` 是背景暗化亮度（osu! 默认 0.3）。
    pub fn open(bytes: Vec<u8>, start_ms: i64, dim_brightness: f64) -> Result<Self, String> {
        let window = web_sys::window().ok_or("缺少 window")?;
        let document = window.document().ok_or("缺少 document")?;
        let element = document
            .create_element("video")
            .map_err(|error| format!("创建 video 元素失败：{error:?}"))?
            .dyn_into::<web_sys::HtmlVideoElement>()
            .map_err(|_| "video 元素类型不匹配".to_string())?;
        let canvas = document
            .create_element("canvas")
            .map_err(|error| format!("创建 canvas 失败：{error:?}"))?
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .map_err(|_| "canvas 元素类型不匹配".to_string())?;
        let context = canvas
            .get_context("2d")
            .map_err(|error| format!("获取 2d 上下文失败：{error:?}"))?
            .ok_or("浏览器不支持 2d 画布上下文")?
            .dyn_into::<web_sys::CanvasRenderingContext2d>()
            .map_err(|_| "2d 上下文类型不匹配".to_string())?;

        // 暗化尽量交给 GPU：drawImage 挂 brightness filter，抓下来的像素就是
        // 暗化结果，CPU 不再逐像素算。设置后读回验证，不支持时走查找表回退。
        context.set_filter(&format!("brightness({dim_brightness})"));
        let dim_on_gpu = context.filter().contains("brightness");

        let parts = js_sys::Array::new();
        parts.push(&js_sys::Uint8Array::from(bytes.as_slice()));
        let blob = web_sys::Blob::new_with_u8_array_sequence(&parts)
            .map_err(|error| format!("构造视频 Blob 失败：{error:?}"))?;
        let object_url = web_sys::Url::create_object_url_with_blob(&blob)
            .map_err(|error| format!("创建视频 URL 失败：{error:?}"))?;

        element.set_src(&object_url);
        // 静音 + playsInline：视频只是装饰，声音来自 WASM 混音；它的音轨必须
        // 彻底静音（属性 + IDL + 音量三重保险），否则会和预览音频混在一起。
        // 移动端要求 playsInline 才不会强制全屏。
        element.set_muted(true);
        element.set_volume(0.0);
        element.set_attribute("muted", "").map_err(|error| {
            format!("设置 video muted 失败：{error:?}")
        })?;
        element.set_attribute("playsinline", "").map_err(|error| {
            format!("设置 video playsinline 失败：{error:?}")
        })?;
        element.set_preload("auto");
        element.set_attribute(
            "style",
            "position:fixed;left:0;top:0;width:1px;height:1px;opacity:0.01;pointer-events:none;z-index:-1",
        )
        .map_err(|error| format!("设置 video 样式失败：{error:?}"))?;
        document
            .body()
            .ok_or("缺少 body")?
            .append_child(&element)
            .map_err(|error| format!("挂载 video 元素失败：{error:?}"))?;

        Ok(Self {
            element,
            canvas,
            context,
            object_url,
            captured_time_s: f64::NAN,
            has_frame: false,
            capture_limit: (MAX_CAPTURE_WIDTH, MAX_CAPTURE_HEIGHT),
            dim_brightness,
            dim_on_gpu,
            start_ms,
        })
    }

    /// 抓帧分辨率上限：跟随预览画布（再被 720p 硬上限截断）。
    pub fn set_capture_limit(&mut self, width: u32, height: u32) {
        self.capture_limit = (
            width.clamp(1, MAX_CAPTURE_WIDTH),
            height.clamp(1, MAX_CAPTURE_HEIGHT),
        );
    }

    /// 视频总时长（毫秒）；元数据就绪前为 0。
    pub fn duration_ms(&self) -> i64 {
        let seconds = self.element.duration();
        if seconds.is_finite() {
            (seconds * 1000.0) as i64
        } else {
            0
        }
    }

    /// 视频在谱面时间 `chart_ms` 的可见度（osu! 的 500ms 淡入淡出）。
    pub fn visibility_alpha(&self, chart_ms: i64) -> f64 {
        visibility_alpha(chart_ms - self.start_ms, self.duration_ms())
    }

    /// 同步播放位置并给出画面更新；`playing` / `rate` 跟随会话。
    ///
    /// 播放时让 `<video>` 自己推进（倍速一致才不会和时钟走散），偏差超过阈值
    /// 才 seek；暂停时精确对到目标。
    pub fn capture_at(&mut self, video_ms: i64, playing: bool, rate: f64) -> Capture {
        let target_s = (video_ms.max(0) as f64) / 1000.0;
        if playing {
            if self.element.paused() {
                if let Ok(promise) = self.element.play() {
                    // 静音视频基本不会被自动播放策略拒绝；把结果吞掉，避免
                    // 未处理的 Promise 污染控制台。
                    wasm_bindgen_futures::spawn_local(async move {
                        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
                    });
                }
            }
            let rate = rate.clamp(0.0625, 16.0);
            if (self.element.playback_rate() - rate).abs() > 1e-3 {
                self.element.set_playback_rate(rate);
            }
        } else if !self.element.paused() {
            let _ = self.element.pause();
        }
        // 播放中允许 0.3 秒漂移（与音频时钟同源，正常远小于此）；暂停帧必须对准。
        let limit_s = if playing { 0.3 } else { 0.05 };
        if (self.element.current_time() - target_s).abs() > limit_s {
            self.element.set_current_time(target_s);
        }
        self.capture()
    }

    /// 抓取当前呈现的画面；时间没变就跳过（回读与拷贝都按像素数计费）。
    fn capture(&mut self) -> Capture {
        if self.element.ready_state() < 2 {
            return self.capture_or_missing();
        }
        let time_s = self.element.current_time();
        if self.has_frame && (time_s - self.captured_time_s).abs() < 0.02 {
            return Capture::Unchanged;
        }
        let (video_w, video_h) = (self.element.video_width(), self.element.video_height());
        if video_w == 0 || video_h == 0 {
            return self.capture_or_missing();
        }
        // 抓帧分辨率封顶：等比缩到「预览画布 / 720p」之内，绝不放大。
        let (limit_w, limit_h) = self.capture_limit;
        let scale = ((limit_w as f64 / video_w as f64).min(limit_h as f64 / video_h as f64)).min(1.0);
        let width = ((video_w as f64 * scale).round() as u32).max(1);
        let height = ((video_h as f64 * scale).round() as u32).max(1);
        if self.canvas.width() != width {
            self.canvas.set_width(width);
        }
        if self.canvas.height() != height {
            self.canvas.set_height(height);
        }
        // filter 每次绘制前都要重设：改 canvas 宽高会把整个 2D 上下文状态
        //（含 filter）重置回默认值，只在 open 时设一次会被首次抓帧的缩放清掉。
        if self.dim_on_gpu {
            self.context
                .set_filter(&format!("brightness({})", self.dim_brightness));
        }
        let captured = self
            .context
            .draw_image_with_html_video_element_and_dw_and_dh(
                &self.element,
                0.0,
                0.0,
                width as f64,
                height as f64,
            )
            .and_then(|_| {
                self.context
                    .get_image_data(0.0, 0.0, width as f64, height as f64)
            })
            .map(|pixels| pixels.data().to_vec())
            .map(|mut rgba| {
                if !self.dim_on_gpu {
                    dim_rgba_lut(&mut rgba, self.dim_brightness);
                }
                VideoFrame {
                    width,
                    height,
                    rgba,
                }
            });
        self.captured_time_s = time_s;
        match captured {
            Ok(frame) => {
                self.has_frame = true;
                Capture::New(frame)
            }
            // 画布被跨域内容污染等场景：按「没有帧」处理，继续回退背景图。
            Err(_) => Capture::Missing,
        }
    }

    /// 时间未变时区分「画面没变」与「还没有画面」。
    fn capture_or_missing(&self) -> Capture {
        if self.has_frame {
            Capture::Unchanged
        } else {
            Capture::Missing
        }
    }
}

/// 整数查找表暗化（canvas filter 不可用时的回退）。
///
/// 逐字节查表替换，比「浮点乘 + round 逐通道」快数倍——这条路径要跑在
/// 每个视频帧上，实时预算里经不起浮点循环。
fn dim_rgba_lut(rgba: &mut [u8], brightness: f64) {
    let table: [u8; 256] =
        std::array::from_fn(|value| (value as f64 * brightness).round().clamp(0.0, 255.0) as u8);
    for pixel in rgba.chunks_exact_mut(4) {
        for channel in &mut pixel[..3] {
            *channel = table[*channel as usize];
        }
    }
}

impl Drop for BackgroundVideo {
    fn drop(&mut self) {
        let _ = self.element.pause();
        if let Some(parent) = self.element.parent_node() {
            let _ = parent.remove_child(&self.element);
        }
        let _ = web_sys::Url::revoke_object_url(&self.object_url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 查找表暗化与浮点公式逐通道一致（含边界值），alpha 不动。
    #[test]
    fn dim_lut_matches_brightness_formula() {
        let mut rgba = vec![0, 30, 100, 255, 200, 255, 50, 128];
        dim_rgba_lut(&mut rgba, 0.3);
        assert_eq!(&rgba[..4], &[0, 9, 30, 255]);
        assert_eq!(&rgba[4..], &[60, 77, 15, 128]);
    }
}
