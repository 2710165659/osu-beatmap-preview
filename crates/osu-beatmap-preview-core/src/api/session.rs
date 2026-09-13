//! 实时预览会话。

use std::sync::Arc;

use super::input::{ImageData, RealtimeOptions, ResourceBundle};
use super::output::{RealtimeMode, TimelineInfo};
use crate::config;
use crate::hitsound::{self, HitsoundMixer, SampleData, SampleLibrary, SAMPLE_RATE};
use crate::model::mods::{parse_mods, validate_mods, ModSettings};
use crate::model::{Beatmap, HitObjects};
use crate::processing::conversion::{catch_convert, mania_convert, taiko_convert};
use crate::processing::timeline::{preview_start_ms, TimeAxis};
use crate::render::scene::FrameScene;
use crate::render::Img;
use crate::support::error::{PreviewError, Result};

/// 已完成解析、转换并准备好渲染资源的实时会话。
#[derive(Debug, Clone)]
pub struct RealtimeSession {
    source_beatmap: Beatmap,
    beatmap: Beatmap,
    timeline: TimelineInfo,
    resources: ResourceBundle,
    mode: RealtimeMode,
    options: RealtimeOptions,
    source: crate::render::wgpu::RealtimeFrameSource,
    // 背景纹理在会话生命周期内保持同一 Arc，避免每帧复制像素并触发 GPU 重新上传。
    background_image: Option<Arc<Img>>,
    /// 打击音混音状态；由宿主通过 [`RealtimeSession::enable_hitsound`] 打开后才建立。
    hitsound: Option<HitsoundMixer>,
    /// [`RealtimeSession::render_hitsound`] 的复用的输出缓冲，避免每帧分配。
    hitsound_buffer: Vec<f32>,
}

impl RealtimeSession {
    pub fn from_bundle(mut bundle: ResourceBundle, options: RealtimeOptions) -> Result<Self> {
        let mut beatmap = bundle
            .beatmap
            .take()
            .ok_or_else(|| PreviewError::new("resource bundle is missing beatmap"))?;
        let settings = parse_mods(&options.mods)?;
        let source_beatmap = beatmap.clone();
        let source_mode = source_beatmap.mode();
        let target_mode = options
            .convert
            .as_deref()
            .map(parse_mode)
            .transpose()?
            .unwrap_or(source_mode);
        validate_realtime_mods(&settings, target_mode)?;
        if target_mode != source_mode {
            if source_mode != 0 {
                return Err(PreviewError::new(
                    "mode conversion requires a standard beatmap",
                ));
            }
            beatmap = converted_beatmap(&source_beatmap, target_mode, &settings)?;
        }
        let (first, last) = object_time_bounds(&beatmap.hit_objects)
            .ok_or_else(|| PreviewError::new("beatmap has no hit objects"))?;
        let speed = if settings.speed_multiplier > 0.0 {
            settings.speed_multiplier
        } else {
            1.0
        };
        let absolute_start_ms = preview_start_ms(first, beatmap.audio_lead_in_ms());
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
        let source = prepare_source(
            &beatmap,
            target_mode,
            &settings,
            time_axis,
            &options.core_config,
        )?;
        let background_image = bundle
            .background
            .as_ref()
            .map(background_image)
            .transpose()?;
        Ok(Self {
            source_beatmap,
            beatmap,
            timeline,
            resources: bundle,
            mode: mode_from_i32(target_mode)?,
            options,
            source,
            background_image,
            hitsound: None,
            hitsound_buffer: Vec::new(),
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

    /// 返回当前会话使用的原始 mod token。
    pub fn mods(&self) -> &[String] {
        &self.options.mods
    }

    /// 原子切换 mod；新设置只有在谱面转换和渲染源都准备成功后才会提交。
    pub fn set_mods(&mut self, mods: Vec<String>) -> Result<()> {
        let settings = parse_mods(&mods)?;
        let target_mode = self.mode_as_i32();
        validate_realtime_mods(&settings, target_mode)?;
        let beatmap = converted_beatmap(&self.source_beatmap, target_mode, &settings)?;
        let (first, last) = object_time_bounds(&beatmap.hit_objects)
            .ok_or_else(|| PreviewError::new("beatmap has no hit objects"))?;
        let source = prepare_source(
            &beatmap,
            target_mode,
            &settings,
            TimeAxis::new(first),
            &self.options.core_config,
        )?;
        let speed = if settings.speed_multiplier > 0.0 {
            settings.speed_multiplier
        } else {
            1.0
        };
        let absolute_start_ms = preview_start_ms(first, beatmap.audio_lead_in_ms());
        self.timeline = TimelineInfo {
            first_object_ms: first,
            last_object_ms: last,
            absolute_start_ms,
            duration_ms: last.saturating_sub(absolute_start_ms),
            beatmap_speed: speed,
        };
        self.options.mods = mods;
        self.beatmap = beatmap.clone();
        self.resources.beatmap = Some(beatmap);
        self.source = source;
        Ok(())
    }

    pub fn set_background(&mut self, background: ImageData) -> Result<()> {
        let image = background_image(&background)?;
        self.resources.background = Some(background);
        self.background_image = Some(image);
        Ok(())
    }

    /// 更新后续 `scene_at_absolute` 使用的输出尺寸。
    ///
    /// 实时播放器可以在不重建会话的情况下切换画布分辨率；此时只更新合成
    /// 场景的尺寸，已缓存的谱面、mod 和背景资源保持不变。
    pub fn set_render_size(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Err(PreviewError::render("render dimensions must be positive"));
        }
        self.options.render.width = width;
        self.options.render.height = height;
        Ok(())
    }

    /// 建立打击音混音状态。
    ///
    /// 采样率取宿主音频设备的实际采样率，保证混音结果可以直接交给音频接口。
    /// 调用后样本库为空，宿主需要按 [`RealtimeSession::hitsound_required_names`]
    /// 逐个放入已解码的 PCM。
    pub fn enable_hitsound(&mut self, volume: i32, sample_rate: u32) {
        let library = SampleLibrary::new();
        let mut mixer = HitsoundMixer::new(
            library,
            hitsound::HitsoundTimeline::default(),
            sample_rate.max(1),
        );
        mixer.set_master_gain(hitsound::volume_gain(volume));
        self.hitsound = Some(mixer);
    }

    pub fn disable_hitsound(&mut self) {
        self.hitsound = None;
    }

    /// 清空已放入的样本并重建时间轴，保留音量与采样率设置。
    ///
    /// 切 Mod 或转谱后需要的样本集合会变，宿主用这个方法重新加载，而不必重建会话。
    pub fn reset_hitsound_samples(&mut self) {
        let Some(mixer) = self.hitsound.as_mut() else {
            return;
        };
        *mixer.library_mut() = SampleLibrary::new();
        mixer.rebuild_timeline_events(hitsound::HitsoundTimeline::default());
        mixer.stop_all();
    }

    pub fn hitsound_enabled(&self) -> bool {
        self.hitsound.is_some()
    }

    /// 需要宿主提供 PCM 的样本名（按当前目标模式与谱面内容计算）。
    ///
    /// 打击音找不到对应样本时按静音处理，因此宿主可以只加载它拿得到的文件。
    pub fn hitsound_required_names(&self) -> Vec<String> {
        hitsound::referenced_names(&self.beatmap)
    }

    /// 放入一段已解码的样本 PCM。
    ///
    /// 只放进样本库，不重建时间轴：宿主应当把需要的样本全部放完后调用一次
    /// [`RealtimeSession::rebuild_hitsound_timeline`]，否则每个样本都会遍历一遍整张
    /// 谱面（样本多时是明显的浪费）。
    pub fn set_hitsound_sample(
        &mut self,
        name: &str,
        channels: hitsound::Channels,
        sample_rate: u32,
        loop_len: usize,
    ) {
        let Some(mixer) = self.hitsound.as_mut() else {
            return;
        };
        let data = SampleData {
            channels,
            sample_rate: sample_rate.max(1),
            loop_len,
        };
        mixer.library_mut().insert(name, data);
    }

    /// 用当前样本库重新生成打击音事件时间轴（样本全部放完后调用一次）。
    pub fn rebuild_hitsound_timeline(&mut self) {
        if let Some(mixer) = self.hitsound.as_mut() {
            mixer.rebuild_timeline(&self.beatmap);
        }
    }

    pub fn set_hitsound_volume(&mut self, volume: i32) {
        if let Some(mixer) = self.hitsound.as_mut() {
            mixer.set_master_gain(hitsound::volume_gain(volume));
        }
    }

    /// 把混音位置对齐到指定谱面时间（不清空正在播放的声音）。
    pub fn position_hitsound(&mut self, chart_time_ms: f64) {
        if let Some(mixer) = self.hitsound.as_mut() {
            mixer.set_position(chart_time_ms);
        }
    }

    /// 把混音位置对齐并丢弃所有正在播放的声音。
    pub fn seek_hitsound(&mut self, chart_time_ms: f64) {
        if let Some(mixer) = self.hitsound.as_mut() {
            mixer.seek(chart_time_ms);
        }
    }

    /// 渲染一段打击音 PCM（交错立体声 f32）到内部缓冲，返回渲染的采样帧数。
    ///
    /// 未启用打击音时返回 0 并清空缓冲；缓冲内容通过
    /// [`RealtimeSession::hitsound_buffer`] 读取（长度为 `帧数 * 2`）。
    pub fn render_hitsound(&mut self, frames: usize) -> usize {
        let Some(mixer) = self.hitsound.as_mut() else {
            self.hitsound_buffer.clear();
            return 0;
        };
        self.hitsound_buffer.clear();
        self.hitsound_buffer.resize(frames * 2, 0.0);
        mixer.render_into(&mut self.hitsound_buffer);
        frames
    }

    /// 最近一次 [`RealtimeSession::render_hitsound`] 的输出。
    pub fn hitsound_buffer(&self) -> &[f32] {
        &self.hitsound_buffer
    }

    /// 取走混音输出缓冲并把内部缓冲重置为空。
    ///
    /// 供宿主一次性取走 PCM：返回 `Vec` 而不是借用，宿主（WASM 胶水层）就能把数据
    /// 直接交给 `Float32Array`，省掉一次拷贝。
    pub fn take_hitsound_buffer(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.hitsound_buffer)
    }

    pub fn hitsound_sample_rate(&self) -> u32 {
        self.hitsound
            .as_ref()
            .map_or(SAMPLE_RATE, |mixer| mixer.sample_rate())
    }

    /// 当前混音位置（谱面毫秒）。
    pub fn hitsound_position_ms(&self) -> f64 {
        self.hitsound
            .as_ref()
            .map_or(0.0, |mixer| mixer.position_ms())
    }

    /// 会话内是否存在可用样本；没有样本时宿主可以完全不启动音频输出。
    pub fn hitsound_has_samples(&self) -> bool {
        self.hitsound
            .as_ref()
            .is_some_and(|mixer| !mixer.library().is_empty())
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

    fn mode_as_i32(&self) -> i32 {
        match self.mode {
            RealtimeMode::Standard => 0,
            RealtimeMode::Taiko => 1,
            RealtimeMode::Catch => 2,
            RealtimeMode::Mania => 3,
        }
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

fn converted_beatmap(
    source: &Beatmap,
    target_mode: i32,
    settings: &ModSettings,
) -> Result<Beatmap> {
    if target_mode == source.mode() {
        return Ok(source.clone());
    }
    if source.mode() != 0 {
        return Err(PreviewError::new(
            "mode conversion requires a standard beatmap",
        ));
    }
    match target_mode {
        1 => taiko_convert(source, 1, Some(settings)),
        2 => catch_convert(source, 2, Some(settings)),
        3 => mania_convert(source, 3, Some(settings)),
        _ => Err(PreviewError::new("unsupported conversion target")),
    }
}

fn validate_realtime_mods(settings: &ModSettings, target_mode: i32) -> Result<()> {
    let mode_errors = validate_mods(settings, Some(target_mode), Some("mp4"));
    if mode_errors.is_empty() {
        Ok(())
    } else {
        Err(PreviewError::new(format!(
            "mod conflict: {}",
            mode_errors.join("; ")
        )))
    }
}

fn prepare_source(
    beatmap: &Beatmap,
    target_mode: i32,
    settings: &ModSettings,
    time_axis: TimeAxis,
    core_config: &Arc<config::CoreConfig>,
) -> Result<crate::render::wgpu::RealtimeFrameSource> {
    config::with_config(Arc::clone(core_config), || match target_mode {
        0 => crate::render::wgpu::prepare_standard(beatmap, Some(settings), time_axis),
        1 => crate::render::wgpu::prepare_taiko(beatmap, Some(settings)),
        2 => crate::render::wgpu::prepare_catch(beatmap, Some(settings)),
        3 => crate::render::wgpu::prepare_mania(beatmap, Some(settings)),
        _ => unreachable!("解析器已经校验 ruleset 模式"),
    })
    .map_err(|error| PreviewError::render(error.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        HitAddition, HitObjects, HitSample, KvSection, SampleBank, StandardHitObject, TimingPoint,
    };

    /// 构造一个包含单个圆圈的实时会话，用于验证打击音接口。
    fn session() -> RealtimeSession {
        let mut general = KvSection::default();
        general.insert("Mode", "0".to_string());
        let beatmap = Beatmap {
            metadata: KvSection::default(),
            difficulty: KvSection::default(),
            general,
            timing_points: vec![TimingPoint {
                time: 0.0,
                beat_length: 500.0,
                meter: 4,
                uninherited: true,
                kiai_mode: false,
                omit_first_bar_line: false,
                sample_set: 0,
                sample_index: 0,
                sample_volume: 100,
            }],
            hit_objects: HitObjects::Standard(vec![StandardHitObject {
                x: 256,
                y: 192,
                start_time: 0,
                end_time: 0,
                hit_type: 1,
                hitsound: 0,
                samples: vec![HitSample::new(SampleBank::Normal, HitAddition::None, 100, None)],
                ..Default::default()
            }]),
            break_periods: Vec::new(),
            background_filename: None,
            combo_colors: Vec::new(),
            beat_divisor: 0,
        };
        RealtimeSession::from_bundle(ResourceBundle::new(beatmap), RealtimeOptions::default())
            .expect("测试会话必须可以创建")
    }

    /// 构造一个首个物件在 `first_ms`、带 `AudioLeadIn` 的会话。
    fn session_with_lead_in(first_ms: i64, lead_in_ms: i64) -> RealtimeSession {
        let mut general = KvSection::default();
        general.insert("Mode", "0".to_string());
        general.insert("AudioLeadIn", lead_in_ms.to_string());
        let beatmap = Beatmap {
            metadata: KvSection::default(),
            difficulty: KvSection::default(),
            general,
            timing_points: vec![TimingPoint {
                time: 0.0,
                beat_length: 500.0,
                meter: 4,
                uninherited: true,
                kiai_mode: false,
                omit_first_bar_line: false,
                sample_set: 0,
                sample_index: 0,
                sample_volume: 100,
            }],
            hit_objects: HitObjects::Standard(vec![StandardHitObject {
                x: 256,
                y: 192,
                start_time: first_ms,
                end_time: first_ms + 500,
                hit_type: 1,
                hitsound: 0,
                samples: vec![HitSample::new(
                    SampleBank::Normal,
                    HitAddition::None,
                    100,
                    None,
                )],
                ..Default::default()
            }]),
            break_periods: Vec::new(),
            background_filename: None,
            combo_colors: Vec::new(),
            beat_divisor: 0,
        };
        RealtimeSession::from_bundle(ResourceBundle::new(beatmap), RealtimeOptions::default())
            .expect("测试会话必须可以创建")
    }

    #[test]
    fn 会话起点与预览起点共用同一规则() {
        // 预览（Web）与完整视频（CLI）必须从同一个起点开始：默认首个物件前 2000ms，
        // AudioLeadIn 更大时按它提前。
        let session = session_with_lead_in(5_000, 0);
        assert_eq!(
            session.timeline().absolute_start_ms,
            preview_start_ms(5_000, 0)
        );
        assert_eq!(session.timeline().absolute_start_ms, 3_000);

        let session = session_with_lead_in(5_000, 4_000);
        assert_eq!(
            session.timeline().absolute_start_ms,
            preview_start_ms(5_000, 4_000)
        );
        assert_eq!(session.timeline().absolute_start_ms, 1_000);
        // 时长按实际起点算：起点提前，时长相应变长。
        assert_eq!(session.timeline().duration_ms, 5_500 - 1_000);
    }

    #[test]
    fn 未启用打击音时不产生混音输出() {
        let mut session = session();
        assert!(!session.hitsound_enabled());
        assert_eq!(session.render_hitsound(16), 0);
        assert!(session.hitsound_buffer().is_empty());
    }

    #[test]
    fn 启用后放入样本即可混音() {
        let mut session = session();
        // 候选名按优先级列出：带 bank 前缀的名字优先，裸名是回退查找。
        assert_eq!(
            session.hitsound_required_names(),
            vec!["hitnormal".to_string(), "normal-hitnormal".to_string()]
        );
        session.enable_hitsound(100, 1000);
        assert!(session.hitsound_enabled());
        // 还没有样本：仍然是静音，但不会 panic。
        assert_eq!(session.render_hitsound(4), 4);
        assert!(session.hitsound_buffer().iter().all(|value| *value == 0.0));
        assert!(!session.hitsound_has_samples());

        // 样本采样率 1000Hz（每帧 1ms），事件在谱面时间 0。
        session.set_hitsound_sample(
            "normal-hitnormal",
            hitsound::Channels::Stereo(vec![1.0, 1.0, 1.0, 1.0]),
            1000,
            0,
        );
        // 只放样本不会重建时间轴：混音位置尚未推进时仍然是静音。
        assert!(session.hitsound_has_samples());
        assert_eq!(session.render_hitsound(2), 2);
        assert!(session.hitsound_buffer().iter().all(|value| *value == 0.0));

        // 样本放完后重建一次时间轴，事件才会生效。
        session.rebuild_hitsound_timeline();
        session.seek_hitsound(0.0);
        assert_eq!(session.render_hitsound(2), 2);
        assert_eq!(session.hitsound_buffer().len(), 4);
        assert!(
            session.hitsound_buffer().iter().all(|value| *value > 0.0),
            "buffer={:?} position={}",
            session.hitsound_buffer(),
            session.hitsound_position_ms()
        );
        assert_eq!(session.hitsound_sample_rate(), 1000);
    }

    #[test]
    fn seek与音量接口不会panic() {
        let mut session = session();
        session.enable_hitsound(50, 1000);
        session.set_hitsound_sample(
            "normal-hitnormal",
            hitsound::Channels::Mono(vec![1.0]),
            1000,
            0,
        );
        session.seek_hitsound(0.0);
        session.position_hitsound(0.0);
        assert_eq!(session.hitsound_position_ms(), 0.0);
        session.set_hitsound_volume(0);
        session.disable_hitsound();
        assert!(!session.hitsound_enabled());
        // 关闭后所有接口退化静音。
        session.set_hitsound_volume(100);
        assert_eq!(session.render_hitsound(4), 0);
    }
}
