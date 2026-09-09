//! 跨平台本地渲染引擎核心。
//!
//! 核心只接收宿主提供的字节和类型化配置，不访问文件系统、网络、缓存、音频设备或日志。

pub mod config;
pub mod domain;
pub mod render;

pub use domain::errors::{ErrorKind, PreviewError, Result};
pub use domain::models::{
    Beatmap, BreakPeriod, CatchHitObject, HitObjects, KvSection, ManiaHitObject, StandardHitObject,
    TaikoHitObject, TimingPoint,
};
pub use domain::mods::{mods_for_mode, parse_mods, validate_mods, ModSettings};
pub use domain::parser::parse_beatmap_bytes;
pub use domain::rulesets::{catch_convert, mania_convert, taiko_convert};
pub use render::{DrawCommand, FrameScene, Img, Rgba, SceneRect, SceneSize};

use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct ResourceBundle {
    pub beatmap: Option<Beatmap>,
    pub background: Option<ImageData>,
    pub audio: Option<AudioData>,
}
impl ResourceBundle {
    pub fn new(beatmap: Beatmap) -> Self {
        Self {
            beatmap: Some(beatmap),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioData {
    pub bytes: Arc<[u8]>,
    pub mime_type: Option<String>,
    pub start_time_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderConfig {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}
impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            scale: 1.0,
        }
    }
}
impl RenderConfig {
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width.max(1), self.height.max(1))
    }
}

#[derive(Debug, Clone)]
pub struct RealtimeOptions {
    pub convert: Option<String>,
    pub mods: Vec<String>,
    pub render: RenderConfig,
    pub video_style: render::wgpu::VideoStyle,
    pub core_config: Arc<config::CoreConfig>,
}

impl Default for RealtimeOptions {
    fn default() -> Self {
        Self {
            convert: None,
            mods: Vec::new(),
            render: RenderConfig::default(),
            video_style: render::wgpu::VideoStyle::default(),
            core_config: Arc::new(config::CoreConfig::default()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineInfo {
    pub first_object_ms: i64,
    pub last_object_ms: i64,
    pub absolute_start_ms: i64,
    pub duration_ms: i64,
    pub beatmap_speed: f64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeMode {
    Standard,
    Taiko,
    Catch,
    Mania,
}

#[derive(Debug, Clone)]
pub struct RealtimeSession {
    beatmap: Beatmap,
    timeline: TimelineInfo,
    resources: ResourceBundle,
    mode: RealtimeMode,
    options: RealtimeOptions,
    source: render::wgpu::RealtimeFrameSource,
    // 背景纹理在会话生命周期内保持同一 Arc，避免每帧复制像素并触发 GPU 重新上传。
    background_image: Option<Arc<Img>>,
}
impl RealtimeSession {
    pub fn from_bundle(mut bundle: ResourceBundle, options: RealtimeOptions) -> Result<Self> {
        let mut beatmap = bundle
            .beatmap
            .take()
            .ok_or_else(|| PreviewError::new("resource bundle is missing beatmap"))?;
        let settings = parse_mods(&options.mods)?;
        let source_mode = beatmap.mode();
        let target_mode = options
            .convert
            .as_deref()
            .map(parse_mode)
            .transpose()?
            .unwrap_or(source_mode);
        if target_mode != source_mode {
            if source_mode != 0 {
                return Err(PreviewError::new(
                    "mode conversion requires a standard beatmap",
                ));
            }
            beatmap = match target_mode {
                1 => taiko_convert(&beatmap, 1, Some(&settings))?,
                2 => catch_convert(&beatmap, 2, Some(&settings))?,
                3 => mania_convert(&beatmap, 3, Some(&settings))?,
                _ => return Err(PreviewError::new("unsupported conversion target")),
            };
        }
        let (first, last) = object_time_bounds(&beatmap.hit_objects)
            .ok_or_else(|| PreviewError::new("beatmap has no hit objects"))?;
        let speed = if settings.speed_multiplier > 0.0 {
            settings.speed_multiplier
        } else {
            1.0
        };
        let absolute_start_ms = first.saturating_sub(2_000);
        let timeline = TimelineInfo {
            first_object_ms: first,
            last_object_ms: last,
            absolute_start_ms,
            // 时长从实际预览起点算起，避免进度条在最后一个物件之前提前结束。
            duration_ms: last.saturating_sub(absolute_start_ms),
            beatmap_speed: speed,
        };
        bundle.beatmap = Some(beatmap.clone());
        let time_axis = domain::shared::time_selection::TimeAxis::new(first);
        let source = config::with_config(Arc::clone(&options.core_config), || match target_mode {
            0 => render::wgpu::prepare_standard(&beatmap, Some(&settings), time_axis),
            1 => render::wgpu::prepare_taiko(&beatmap, Some(&settings)),
            2 => render::wgpu::prepare_catch(&beatmap, Some(&settings)),
            3 => render::wgpu::prepare_mania(&beatmap, Some(&settings)),
            _ => unreachable!("解析器已经校验 ruleset 模式"),
        })
        .map_err(|error| PreviewError::render(error.to_string()))?;
        let background_image = bundle
            .background
            .as_ref()
            .map(|background| {
                let expected = background.width as usize * background.height as usize * 4;
                if background.width == 0
                    || background.height == 0
                    || background.rgba.len() != expected
                {
                    return Err(PreviewError::new(
                        "background RGBA length does not match dimensions",
                    ));
                }
                Ok(Arc::new(Img {
                    w: background.width,
                    h: background.height,
                    data: background.rgba.clone(),
                }))
            })
            .transpose()?;
        Ok(Self {
            beatmap,
            timeline,
            resources: bundle,
            mode: mode_from_i32(target_mode)?,
            options,
            source,
            background_image,
        })
    }
    pub fn beatmap(&self) -> &Beatmap {
        &self.beatmap
    }
    pub fn timeline(&self) -> TimelineInfo {
        self.timeline
    }
    pub fn resources(&self) -> &ResourceBundle {
        &self.resources
    }
    pub fn mode(&self) -> RealtimeMode {
        self.mode
    }
    pub fn options(&self) -> &RealtimeOptions {
        &self.options
    }
    pub fn set_background(&mut self, background: ImageData) -> Result<()> {
        let expected = background.width as usize * background.height as usize * 4;
        if background.rgba.len() != expected {
            return Err(PreviewError::new(
                "background RGBA length does not match dimensions",
            ));
        }
        let image = Arc::new(Img {
            w: background.width,
            h: background.height,
            data: background.rgba.clone(),
        });
        self.resources.background = Some(background);
        self.background_image = Some(image);
        Ok(())
    }
    pub fn set_audio(&mut self, audio: AudioData) {
        self.resources.audio = Some(audio);
    }
    pub fn scene_at_absolute(&self, absolute_time_ms: i64) -> Result<FrameScene> {
        let (width, height) = self.options.render.dimensions();
        let playfield = config::with_config(Arc::clone(&self.options.core_config), || {
            self.source.render(absolute_time_ms)
        })
        .map_err(|error| PreviewError::render(error.to_string()))?;
        render::wgpu::composition::compose_video_scene(
            playfield,
            absolute_time_ms.saturating_sub(self.timeline.first_object_ms),
            self.timeline.duration_ms,
            width,
            height,
            self.background_image.as_ref(),
            self.options.video_style,
            render::wgpu::game_mode(self.mode),
        )
        .map_err(|error| PreviewError::render(error.to_string()))
    }
}

fn parse_mode(value: &str) -> Result<i32> {
    match value.trim().to_ascii_lowercase().as_str() {
        "standard" | "std" => Ok(0),
        "taiko" => Ok(1),
        "catch" | "ctb" => Ok(2),
        "mania" => Ok(3),
        _ => Err(PreviewError::new(format!(
            "unknown convert target: '{value}'"
        ))),
    }
}
fn mode_from_i32(value: i32) -> Result<RealtimeMode> {
    match value {
        0 => Ok(RealtimeMode::Standard),
        1 => Ok(RealtimeMode::Taiko),
        2 => Ok(RealtimeMode::Catch),
        3 => Ok(RealtimeMode::Mania),
        _ => Err(PreviewError::new("unsupported beatmap mode")),
    }
}
fn object_time_bounds(objects: &HitObjects) -> Option<(i64, i64)> {
    let mut bounds: Option<(i64, i64)> = None;
    let mut add = |start: i64, end: i64| {
        bounds = Some(match bounds {
            Some((first, last)) => (first.min(start), last.max(end)),
            None => (start, end),
        })
    };
    match objects {
        HitObjects::Standard(v) => {
            for o in v {
                add(o.start_time, o.end_time)
            }
        }
        HitObjects::Taiko(v) => {
            for o in v {
                add(o.start_time, o.end_time)
            }
        }
        HitObjects::Catch(v) => {
            for o in v {
                add(o.start_time, o.end_time)
            }
        }
        HitObjects::Mania(v) => {
            for o in v {
                add(o.start_time, o.end_time)
            }
        }
    }
    bounds
}
