//! 实时预览会话。

use std::sync::Arc;

use super::input::{AudioData, ImageData, RealtimeOptions, ResourceBundle};
use super::output::{RealtimeMode, TimelineInfo};
use crate::config;
use crate::model::mods::parse_mods;
use crate::model::{Beatmap, HitObjects};
use crate::processing::conversion::{catch_convert, mania_convert, taiko_convert};
use crate::processing::timeline::TimeAxis;
use crate::render::scene::FrameScene;
use crate::render::Img;
use crate::support::error::{PreviewError, Result};

/// 已完成解析、转换并准备好渲染资源的实时会话。
#[derive(Debug, Clone)]
pub struct RealtimeSession {
    beatmap: Beatmap,
    timeline: TimelineInfo,
    resources: ResourceBundle,
    mode: RealtimeMode,
    options: RealtimeOptions,
    source: crate::render::wgpu::RealtimeFrameSource,
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
        let time_axis = TimeAxis::new(first);
        let source = config::with_config(Arc::clone(&options.core_config), || match target_mode {
            0 => crate::render::wgpu::prepare_standard(&beatmap, Some(&settings), time_axis),
            1 => crate::render::wgpu::prepare_taiko(&beatmap, Some(&settings)),
            2 => crate::render::wgpu::prepare_catch(&beatmap, Some(&settings)),
            3 => crate::render::wgpu::prepare_mania(&beatmap, Some(&settings)),
            _ => unreachable!("解析器已经校验 ruleset 模式"),
        })
        .map_err(|error| PreviewError::render(error.to_string()))?;
        let background_image = bundle
            .background
            .as_ref()
            .map(background_image)
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
        let image = background_image(&background)?;
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
        crate::render::wgpu::composition::compose_video_scene(
            playfield,
            absolute_time_ms.saturating_sub(self.timeline.first_object_ms),
            self.timeline.duration_ms,
            width,
            height,
            self.background_image.as_ref(),
            self.options.video_style,
            crate::render::wgpu::game_mode(self.mode),
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
        HitObjects::Standard(v) => v
            .iter()
            .for_each(|object| add(object.start_time, object.end_time)),
        HitObjects::Taiko(v) => v
            .iter()
            .for_each(|object| add(object.start_time, object.end_time)),
        HitObjects::Catch(v) => v
            .iter()
            .for_each(|object| add(object.start_time, object.end_time)),
        HitObjects::Mania(v) => v
            .iter()
            .for_each(|object| add(object.start_time, object.end_time)),
    }
    bounds
}

fn background_image(background: &ImageData) -> Result<Arc<Img>> {
    let expected = background.width as usize * background.height as usize * 4;
    if background.width == 0 || background.height == 0 || background.rgba.len() != expected {
        return Err(PreviewError::new(
            "background RGBA length does not match dimensions",
        ));
    }
    Ok(Arc::new(Img {
        w: background.width,
        h: background.height,
        data: background.rgba.clone(),
    }))
}
