//! 实时预览会话。

use std::sync::Arc;

use super::clock::PreviewClock;
use super::input::{ImageData, RealtimeOptions, ResourceBundle};
use super::output::{RealtimeMode, TimelineInfo};
use super::stream::{AudioStream, RESEEK_TOLERANCE_MS};
use crate::config;
use crate::hitsound::{self, HitsoundMixer, HitsoundTimeline, SampleData, SampleLibrary};
use crate::model::mods::{parse_mods, validate_mods, ModSettings};
use crate::model::{Beatmap, HitObjects};
use crate::processing::conversion::{catch_convert, mania_convert, taiko_convert};
use crate::processing::timeline::{preview_start_ms, TimeAxis, PREVIEW_END_PADDING_MS};
use crate::render::scene::FrameScene;
use crate::render::Img;
use crate::support::error::{PreviewError, Result};

/// 已完成解析、转换并准备好渲染资源的实时会话。
///
/// 时钟、混音与输出流都在会话里：宿主只提供墙钟读数（WASM 传 `performance.now()`）、
/// 把混音结果送进音频设备，并把音频线程的消费位置转发回来，音画时间只有一处实现。
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
    /// 音乐 + 打击音混音器：与画面共用同一条谱面时间轴，会话生命周期内一直持有
    /// （打击音关闭时音乐仍要出声，开关不能用 `Option` 表达）。
    mixer: HitsoundMixer,
    /// 打击音开关；关闭时事件时间轴为空，混音输出只剩音乐。
    hitsound_enabled: bool,
    /// 用户倍速（不含 DT/HT）；总倍速 = 用户倍速 × `timeline.beatmap_speed`。
    user_rate: f64,
    /// 预览时钟：画面与音频共用的时间权威。
    clock: PreviewClock,
    /// 混音输出流：决定下一段混音的起点与长度，并跟踪音频线程的消费进度。
    stream: AudioStream,
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
        let timeline = preview_timeline(first, last, beatmap.audio_lead_in_ms(), speed);
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
        let audio = options.audio.clone();
        let mut mixer = HitsoundMixer::new(
            SampleLibrary::new(),
            HitsoundTimeline::default(),
            audio.sample_rate.max(1),
        );
        mixer.set_master_gain(hitsound::volume_gain(audio.hitsound_volume));
        mixer.set_music_gain(hitsound::volume_gain(audio.music_volume));
        let mut clock = PreviewClock::new();
        // 总倍速 = 用户倍速（初始 1.0）× 谱面变速；换 mod 时由 `set_mods` 重算。
        clock.set_rate(speed, 0.0);
        // 时钟与输出流都从预览起点开始（首个物件前的预卷段可能为负），
        // 与宿主「进度条 0 = 预览起点」的坐标一致。
        let mut stream = AudioStream::new(audio.sample_rate.max(1));
        stream.reset(timeline.absolute_start_ms as f64);
        clock.seek(timeline.absolute_start_ms as f64, 0.0);
        Ok(Self {
            source_beatmap,
            beatmap,
            timeline,
            resources: bundle,
            mode: mode_from_i32(target_mode)?,
            options,
            source,
            background_image,
            mixer,
            hitsound_enabled: audio.hitsound_enabled,
            user_rate: 1.0,
            clock,
            stream,
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
    ///
    /// `wall_ms` 是当前墙钟读数：切换可能改变谱面变速（DT/HT），总倍速随之变化，
    /// 时钟必须在这一刻换速而不是让时刻跳变。事件时间轴也随谱面重建（转谱会改变
    /// 物件与所需样本），样本随后由宿主重新装载。
    pub fn set_mods(&mut self, mods: Vec<String>, wall_ms: f64) -> Result<()> {
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
        self.timeline = preview_timeline(first, last, beatmap.audio_lead_in_ms(), speed);
        self.options.mods = mods;
        self.beatmap = beatmap.clone();
        self.resources.beatmap = Some(beatmap);
        self.source = source;
        self.clock.set_rate(self.user_rate * speed, wall_ms);
        self.rebuild_hitsound_timeline();
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

    // ── 音频：音乐与打击音同流混音 ──────────────────────────────────────
    //
    // 时间、倍速与 seek 都由会话时钟统一驱动；宿主只做两件事：把 `pull_audio` 的
    // 混音结果送进音频设备，把音频线程的消费位置用 `on_audio_clock` 转发回来。

    /// 设置背景音乐（`None` 表示清除）。
    ///
    /// 音乐按「谱面绝对毫秒 × 采样率 / 1000」逐帧采样，与打击音共用同一条时间轴，
    /// 因此倍速、seek、暂停只有一处位置计算。没有音乐的谱面（单独的 `.osu`）完全
    /// 不必调用它。
    pub fn set_music(&mut self, music: Option<SampleData>) {
        self.mixer.set_music(music);
    }

    /// 更新音乐音量百分比（0..=100）。
    pub fn set_music_volume(&mut self, volume: i32) {
        self.mixer.set_music_gain(hitsound::volume_gain(volume));
    }

    /// 打开/关闭打击音；关闭时事件时间轴为空，音乐照常输出。
    pub fn set_hitsound_enabled(&mut self, enabled: bool) {
        if self.hitsound_enabled == enabled {
            return;
        }
        self.hitsound_enabled = enabled;
        self.rebuild_hitsound_timeline();
    }

    pub fn hitsound_enabled(&self) -> bool {
        self.hitsound_enabled
    }

    /// 更新打击音音量百分比（0..=100）。
    pub fn set_hitsound_volume(&mut self, volume: i32) {
        self.mixer.set_master_gain(hitsound::volume_gain(volume));
    }

    /// 当前谱面需要样本库提供的样本名（按优先级排列，含裸名回退）。
    ///
    /// 宿主只装载它拿得到的名字；缺失的样本在混音时按静音处理。
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
        let data = SampleData {
            channels,
            sample_rate: sample_rate.max(1),
            loop_len,
        };
        self.mixer.library_mut().insert(name, data);
    }

    /// 用当前样本库重建打击音事件时间轴（样本全部放完后调用一次）。
    ///
    /// 打击音关闭时事件时间轴置空；重建会清空正在播放的声音，只在装载阶段调用。
    pub fn rebuild_hitsound_timeline(&mut self) {
        let timeline = if self.hitsound_enabled {
            hitsound::build_timeline(&self.beatmap, self.mixer.library())
        } else {
            HitsoundTimeline::default()
        };
        self.mixer.rebuild_timeline_events(timeline);
    }

    /// 清空已放入的样本并重建时间轴（保留音乐、音量与开关设置）。
    ///
    /// 切 Mod 或转谱后需要的样本集合会变，宿主重新装载即可，不必重建会话。
    pub fn reset_hitsound_samples(&mut self) {
        *self.mixer.library_mut() = SampleLibrary::new();
        self.rebuild_hitsound_timeline();
        self.mixer.stop_all();
    }

    // ── 时钟 ───────────────────────────────────────────────────────────
    //
    // 所有方法的 `wall_ms` 都是宿主的墙钟读数（WASM 传 `performance.now()`）；
    // 时间本身用「谱面绝对毫秒」表示（0 = 音频文件 0 点，首个物件前的预卷段为负）。

    /// 开始播放：时钟从当前时刻继续推进。已经播放时是空操作。
    pub fn play(&mut self, wall_ms: f64) {
        self.clock.play(wall_ms);
    }

    /// 暂停：把当前时刻固化。已经暂停时是空操作。
    pub fn pause(&mut self, wall_ms: f64) {
        self.clock.pause(wall_ms);
    }

    /// 跳到指定谱面绝对时间：时钟换锚、丢弃正在播放的声音、输出流整体重置
    /// （宿主通过 [`RealtimeSession::audio_epoch`] 感知重置，把环形缓冲一起归零）。
    ///
    /// 非有限目标忽略，WASM 入口负责向宿主报错。
    pub fn seek(&mut self, wall_ms: f64, chart_ms: f64) {
        if !chart_ms.is_finite() {
            return;
        }
        self.clock.seek(chart_ms, wall_ms);
        self.stream.reset(chart_ms);
        self.mixer.seek(chart_ms);
    }

    /// 设置用户倍速（不含 DT/HT；总倍速由会话乘上谱面变速）。
    ///
    /// 非有限或非正的倍速拒绝。换速瞬间时刻连续；音频消费速率应同步改为
    /// [`RealtimeSession::rate`] 的返回值。
    pub fn set_rate(&mut self, wall_ms: f64, user_rate: f64) {
        if !user_rate.is_finite() || user_rate <= 0.0 {
            return;
        }
        self.user_rate = user_rate;
        let speed = self.timeline.beatmap_speed;
        self.clock.set_rate(user_rate * speed, wall_ms);
    }

    /// 当前总倍速（用户倍速 × 谱面变速），即音频线程的消费速率。
    pub fn rate(&self) -> f64 {
        self.clock.rate()
    }

    /// 当前谱面绝对时间（毫秒）；画面渲染按它取场景。
    pub fn clock_ms(&self, wall_ms: f64) -> f64 {
        self.clock.current(wall_ms)
    }

    pub fn playing(&self) -> bool {
        self.clock.playing()
    }

    // ── 混音输出 ───────────────────────────────────────────────────────

    /// 按当前时钟补一段「音乐 + 打击音」混音，返回交错立体声 f32（帧数 × 2）。
    ///
    /// 补多少由预读窗口决定（可能返回空数组表示暂无需要补的数据），`max_frames`
    /// 限制单次返回量；宿主把返回数据接在环形缓冲的写入前沿之后即可。
    ///
    /// 画面时钟与音频消费位置走散（音频停滞后恢复、长时间后台）时整条流重置到画面
    /// 位置：音频跳到画面位置继续，画面绝不回跳；重置的纪元变化见
    /// [`RealtimeSession::audio_epoch`]。
    pub fn pull_audio(&mut self, wall_ms: f64, max_frames: usize) -> Vec<f32> {
        let clock_ms = self.clock.current(wall_ms);
        let consumer_ms = self.stream.consumer_chart_ms(&self.clock, wall_ms);
        let drift = consumer_ms - clock_ms;
        if drift.abs() > RESEEK_TOLERANCE_MS {
            self.stream.reset(clock_ms);
            self.mixer.seek(clock_ms);
        } else {
            self.clock.anchor_audio(consumer_ms, wall_ms);
        }
        self.stream
            .pull(&mut self.mixer, &self.clock, wall_ms, max_frames)
    }

    /// 记录音频线程回报的消费位置（自上次输出流重置起已消费的采样帧）。
    ///
    /// 位置是画面时钟锚定的依据；宿主在音频线程每次位置回报时调用。
    pub fn on_audio_clock(&mut self, consumed_frames: u64, wall_ms: f64) {
        self.stream.note_consumed(consumed_frames, wall_ms);
    }

    /// 混音输出流的重置纪元：变化表示整条流已重置，宿主必须把环形缓冲的读写指针
    /// 一起归零，并通知音频线程从头重读。
    pub fn audio_epoch(&self) -> u64 {
        self.stream.epoch()
    }

    /// 混音输出采样率（宿主音频设备的实际采样率）。
    pub fn audio_sample_rate(&self) -> u32 {
        self.mixer.sample_rate()
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

/// 计算会话时间轴。
///
/// 起点与完整预览（CLI MP4）共用 [`preview_start_ms`]；时长从实际预览起点算起，
/// 避免进度条在最后一个物件之前提前结束；末尾再保留 [`PREVIEW_END_PADDING_MS`]
/// 余韵，与 MP4 的尾部留白一致，最后一个物件后仍会继续渲染 2 秒。
fn preview_timeline(first: i64, last: i64, audio_lead_in_ms: i64, speed: f64) -> TimelineInfo {
    let absolute_start_ms = preview_start_ms(first, audio_lead_in_ms);
    TimelineInfo {
        first_object_ms: first,
        last_object_ms: last,
        absolute_start_ms,
        duration_ms: last
            .saturating_sub(absolute_start_ms)
            .saturating_add(PREVIEW_END_PADDING_MS),
        beatmap_speed: speed,
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

#[cfg(test)]
mod tests {
    use super::super::input::AudioConfig;
    use super::*;
    use crate::model::{
        HitAddition, HitObjects, HitSample, KvSection, SampleBank, StandardHitObject, TimingPoint,
    };

    /// 1kHz 音频输出的测试选项：1 采样帧 = 1ms，混音断言可以直接按帧数算时间。
    fn test_options() -> RealtimeOptions {
        RealtimeOptions {
            audio: AudioConfig {
                sample_rate: 1000,
                hitsound_enabled: true,
                hitsound_volume: 100,
                music_volume: 100,
            },
            ..RealtimeOptions::default()
        }
    }

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
        RealtimeSession::from_bundle(ResourceBundle::new(beatmap), test_options())
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

    /// 会话起点与预览起点共用同一条规则。
    #[test]
    fn session_start_shares_rule_with_preview_start() {
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
        // 时长按实际起点算：起点提前，时长相应变长；末尾再留 2s 余韵。
        assert_eq!(session.timeline().duration_ms, 5_500 + 2_000 - 1_000);
    }

    /// 最后一个物件后再留 2s 余韵，预览不会在最后一个物件的瞬间结束。
    #[test]
    fn duration_keeps_end_padding_after_last_object() {
        let session = session_with_lead_in(5_000, 0);
        // 起点 = 5_000 - 2_000 = 3_000，末尾 = 5_500 + 2_000 = 7_500。
        assert_eq!(session.timeline().duration_ms, 4_500);
    }

    /// 打击音关闭时混音只剩音乐；没有音乐时输出整段静音，但输出流照常推进。
    #[test]
    fn disabled_hitsound_leaves_only_music_in_the_mix() {
        let mut session = session();
        assert!(session.hitsound_enabled());
        session.set_hitsound_enabled(false);
        assert!(!session.hitsound_enabled());

        session.seek(0.0, 0.0);
        session.play(0.0);
        let silent = session.pull_audio(0.0, 4);
        assert_eq!(silent.len(), 8);
        assert!(silent.iter().all(|value| *value == 0.0), "{silent:?}");

        // 有音乐时关闭打击音也照常出声。
        session.set_music(Some(SampleData::stereo(vec![1.0; 8], 1000)));
        session.seek(0.0, 0.0);
        let music_only = session.pull_audio(0.0, 2);
        assert!(
            music_only.iter().any(|value| *value > 0.0),
            "music_only={music_only:?}"
        );
    }

    /// 装载样本并重建时间轴后，混音输出按时间轴发声。
    #[test]
    fn mixing_works_once_samples_are_added() {
        let mut session = session();
        // 候选名按优先级列出：带 bank 前缀的名字优先，裸名是回退查找。
        assert_eq!(
            session.hitsound_required_names(),
            vec!["hitnormal".to_string(), "normal-hitnormal".to_string()]
        );
        // 样本采样率 1000Hz（每帧 1ms），事件在谱面时间 0。
        session.set_hitsound_sample(
            "normal-hitnormal",
            hitsound::Channels::Stereo(vec![1.0, 1.0, 1.0, 1.0]),
            1000,
            0,
        );
        // 只放样本不会重建时间轴：事件时间轴还是空的，输出保持静音。
        session.seek(0.0, 0.0);
        assert!(
            session.pull_audio(0.0, 2).iter().all(|value| *value == 0.0),
            "未重建时间轴就出声了"
        );

        // 样本放完后重建一次时间轴，事件才会生效。
        session.rebuild_hitsound_timeline();
        session.seek(0.0, 0.0);
        let output = session.pull_audio(0.0, 2);
        assert_eq!(output.len(), 4);
        assert!(
            output.iter().all(|value| *value > 0.0),
            "output={output:?}"
        );
    }

    /// 音乐与打击音走同一条输出流，叠加后再统一限幅。
    #[test]
    fn music_and_hitsound_share_one_output_stream() {
        let mut session = session();
        session.set_music(Some(SampleData::stereo(vec![0.4; 8], 1000)));

        session.seek(0.0, 0.0);
        let music_only = session.pull_audio(0.0, 1);

        session.set_hitsound_sample(
            "normal-hitnormal",
            hitsound::Channels::Stereo(vec![1.0; 4]),
            1000,
            0,
        );
        session.rebuild_hitsound_timeline();
        session.seek(0.0, 0.0);
        let both = session.pull_audio(0.0, 1);

        assert_eq!(both.len(), 2);
        assert!(
            both[0] > music_only[0],
            "音乐与打击音没有叠加：music_only={} both={}",
            music_only[0],
            both[0]
        );
    }

    /// 时钟按播放/暂停/倍速/seek 推进，起点是预览起点（含负预卷）。
    #[test]
    fn clock_follows_play_pause_seek_and_rate() {
        let mut session = session();
        // 首个物件在 0ms：预览起点是它前面 2000ms 的预卷。
        assert_eq!(session.clock_ms(0.0), -2_000.0);

        session.play(0.0);
        assert!(session.playing());
        assert_eq!(session.clock_ms(500.0), -1_500.0);

        // 用户倍速与谱面变速叠加（测试谱面 speed = 1）。
        session.set_rate(500.0, 2.0);
        assert_eq!(session.rate(), 2.0);
        assert_eq!(session.clock_ms(500.0), -1_500.0);
        assert_eq!(session.clock_ms(1_500.0), 500.0);

        session.pause(1_500.0);
        assert_eq!(session.clock_ms(99_999.0), 500.0);

        session.seek(1_500.0, -2_000.0);
        assert_eq!(session.clock_ms(1_500.0), -2_000.0);
    }

    /// 换 mod 重算总倍速，用户倍速保持。
    #[test]
    fn mod_switch_recomputes_total_rate() {
        let mut session = session();
        session.set_mods(vec!["DT".to_string()], 0.0).expect("DT 必须合法");
        assert!((session.rate() - 1.5).abs() < 1e-9, "rate={}", session.rate());
        session.set_rate(0.0, 2.0);
        assert!((session.rate() - 3.0).abs() < 1e-9, "rate={}", session.rate());
    }

    /// 音频消费位置走散后输出流整体重置到画面位置，并通过纪元通知宿主。
    #[test]
    fn pull_audio_resyncs_stream_when_consumer_drifts() {
        let mut session = session();
        session.play(0.0);
        session.pull_audio(0.0, 100);
        let epoch_before = session.audio_epoch();

        // 音频线程停摆很久后画面时钟走远（10s @ 1 倍速）：下一次 pull 把整条流
        // 重置到画面位置，音频跳到画面位置继续，画面绝不回跳。
        let output = session.pull_audio(10_000.0, 100);
        assert!(session.audio_epoch() > epoch_before, "输出流没有重置");
        assert!(!output.is_empty(), "重置后应当从画面位置继续混音");

        // 正常消费报告不触发重置，画面时钟向音频位置平滑收敛。
        let epoch_after = session.audio_epoch();
        let consumed = 100;
        session.on_audio_clock(consumed, 10_000.0);
        session.pull_audio(10_016.0, 100);
        assert_eq!(session.audio_epoch(), epoch_after, "正常回报不该重置输出流");
    }

    /// 音频接口的坏输入被拒绝，不破坏会话状态。
    #[test]
    fn audio_api_rejects_bad_input() {
        let mut session = session();
        session.play(0.0);
        session.set_rate(0.0, f64::NAN);
        session.set_rate(0.0, -1.0);
        assert_eq!(session.rate(), 1.0);
        session.seek(0.0, f64::NAN);
        assert_eq!(session.clock_ms(0.0), -2_000.0);
        session.set_music_volume(500);
        session.set_hitsound_volume(-20);
        session.set_hitsound_enabled(false);
        session.set_hitsound_enabled(false);
        assert!(!session.hitsound_enabled());
    }
}
