//! 打击音（hit sound）时间轴与混音。
//!
//! 这个模块只负责「什么时候播放哪个样本、多大声」，不接触文件系统、网络或音频设备：
//! 样本 PCM 由宿主提供（CLI 解码内嵌音频、Web 宿主用浏览器解码），因此同一套事件与
//! 混音逻辑既能驱动 MP4 离线混音，也能驱动浏览器实时播放。

mod assets;
mod mixer;

pub use assets::{asset_bytes, asset_count, asset_names, HITSOUND_ASSETS};
pub use mixer::HitsoundMixer;

use std::collections::HashMap;

use crate::domain::models::{
    Beatmap, CatchHitObject, HitAddition, HitSample, HitObjects, ManiaHitObject, SampleBank, StandardHitObject,
    TaikoHitObject, TimingPoint,
};

/// osu! 的 `SkinnableSound` 将谱面音量百分比直接转换为线性增益。
/// 100 → 1.0，70 → 0.7，50 → 0.5；预览默认 50% 与游戏内一致。
pub fn volume_gain(volume: i32) -> f64 {
    volume.clamp(0, 100) as f64 / 100.0
}

/// 宿主没有提供采样率时使用的默认混音采样率。
pub const SAMPLE_RATE: u32 = 48_000;

/// 一个待播放的打击音事件。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayEvent {
    /// 相对于谱面零时刻的毫秒时间。
    pub start_ms: f64,
    /// 播放时长；0 表示按样本自身长度播放，循环事件则一直持续到该时长结束。
    pub duration_ms: f64,
    /// 样本在 [`SampleLibrary`] 中的 id。
    pub source_id: usize,
    /// 线性增益（已包含谱面音量与设计音量）。
    pub gain: f64,
    /// 是否为循环音（滑条滑行音）。
    pub looping: bool,
}

/// 打击音时间轴，按开始时间升序。
#[derive(Debug, Clone, Default)]
pub struct HitsoundTimeline {
    pub events: Vec<PlayEvent>,
}

impl HitsoundTimeline {
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }
}

/// 样本的声道布局。
#[derive(Debug, Clone, PartialEq)]
pub enum Channels {
    /// 单声道采样，左右声道共用。
    Mono(Vec<f32>),
    /// 交错存储的左右声道采样。
    Stereo(Vec<f32>),
}

/// 一段已解码的打击音样本。
#[derive(Debug, Clone, PartialEq)]
pub struct SampleData {
    /// 单声道样本或交错双声道样本。
    pub channels: Channels,
    pub sample_rate: u32,
    /// 循环音（`*-sliderslide`）在样本内的采样帧数；0 表示不循环。
    pub loop_len: usize,
}

impl SampleData {
    pub fn mono(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            channels: Channels::Mono(samples),
            sample_rate,
            loop_len: 0,
        }
    }

    pub fn stereo(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            channels: Channels::Stereo(samples),
            sample_rate,
            loop_len: 0,
        }
    }

    pub fn with_loop(mut self, loop_len: usize) -> Self {
        self.loop_len = loop_len;
        self
    }

    /// 采样帧数（双声道交错数据为长度的一半）。
    pub fn frames(&self) -> usize {
        match &self.channels {
            Channels::Mono(samples) => samples.len(),
            Channels::Stereo(samples) => samples.len() / 2,
        }
    }

    /// 采样帧 `frame` 的左右声道；越界返回静音。
    pub fn frame(&self, frame: usize) -> (f32, f32) {
        match &self.channels {
            Channels::Mono(samples) => {
                let value = samples.get(frame).copied().unwrap_or(0.0);
                (value, value)
            }
            Channels::Stereo(samples) => {
                let left = samples.get(frame * 2).copied().unwrap_or(0.0);
                let right = samples.get(frame * 2 + 1).copied().unwrap_or(0.0);
                (left, right)
            }
        }
    }
}

/// 已解码的打击音样本库。
///
/// 缺失的样本名不会出现在索引中，事件生成阶段直接跳过，因此「资源缺失」或
/// 「音频文件无法解码」都按静音处理，不会中断渲染。
#[derive(Debug, Clone, Default)]
pub struct SampleLibrary {
    pub(crate) sources: Vec<SampleData>,
    index: HashMap<String, usize>,
}

impl SampleLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一段已解码的样本；同名后写入的覆盖前一个。
    pub fn insert(&mut self, name: impl Into<String>, data: SampleData) {
        let name = name.into();
        match self.index.get(&name) {
            Some(&existing) => self.sources[existing] = data,
            None => {
                let id = self.sources.len();
                self.sources.push(data);
                self.index.insert(name, id);
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<&SampleData> {
        self.index.get(name).map(|&id| &self.sources[id])
    }

    /// 按样本 id 取回名称。
    pub fn name_of(&self, id: usize) -> Option<&str> {
        self.index
            .iter()
            .find(|(_, &value)| value == id)
            .map(|(name, _)| name.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

/// 候选文件名到样本的解析器。
///
/// 时间轴生成与「收集需要的样本名」共用同一套遍历逻辑：前者用样本库解析，
/// 后者用只记录名字的解析器，因此两者永远不会走偏。
///
/// 候选名以字节序列传入（事件生成走栈缓冲，不分配 `String`）；返回的是内嵌资源
/// 字节切片，因此默认实现要求调用方传入 `'static` 数据。
pub trait SampleResolver {
    /// 返回第一个可用候选的样本 id；全部缺失时返回 `None`（按静音处理）。
    fn resolve<'a, I: Iterator<Item = &'a [u8]>>(&mut self, candidates: I) -> Option<usize>;
}

impl SampleResolver for SampleLibrary {
    fn resolve<'a, I: Iterator<Item = &'a [u8]>>(&mut self, candidates: I) -> Option<usize> {
        candidates
            .filter_map(|candidate| std::str::from_utf8(candidate).ok())
            .find_map(|candidate| self.index.get(candidate).copied())
    }
}

/// 允许以 `&SampleLibrary` 形式复用同一个样本库（只读解析）。
impl SampleResolver for &SampleLibrary {
    fn resolve<'a, I: Iterator<Item = &'a [u8]>>(&mut self, candidates: I) -> Option<usize> {
        candidates
            .filter_map(|candidate| std::str::from_utf8(candidate).ok())
            .find_map(|candidate| self.index.get(candidate).copied())
    }
}

/// 只收集候选名的解析器，用于宿主预解码前的依赖分析。
#[derive(Debug, Default)]
pub struct CollectNames {
    names: Vec<String>,
}

impl CollectNames {
    pub fn into_names(mut self) -> Vec<String> {
        self.names.sort();
        self.names.dedup();
        self.names
    }
}

impl SampleResolver for CollectNames {
    fn resolve<'a, I: Iterator<Item = &'a [u8]>>(&mut self, candidates: I) -> Option<usize> {
        // 宿主需要知道「这名字取自哪个资源」，因此这里把候选名记成可读文本。
        self.names
            .extend(candidates.filter_map(|name| std::str::from_utf8(name).ok().map(str::to_string)));
        None
    }
}

/// 查找一个候选名字所需的缓冲长度：`bank-` 前缀 + 名字。
const ASSET_KEY_CAPACITY: usize = 32;

/// 把若干片段拼进定长缓冲；放不下时返回 `None`（调用方退化成堆分配）。
macro_rules! concat_key {
    ($capacity:expr, $parts:expr) => {{
        let parts: &[&[u8]] = $parts;
        let total: usize = parts.iter().map(|part| part.len()).sum();
        if total <= $capacity {
            let mut buffer = [0_u8; $capacity];
            let mut len = 0;
            for part in parts {
                buffer[len..len + part.len()].copy_from_slice(part);
                len += part.len();
            }
            Some((buffer, total))
        } else {
            None
        }
    }};
}

/// 栈上的候选名字缓冲，避免每个事件都分配 `String`。
///
/// 一个事件最多两个候选（`bank-name` 与裸 `name`），用定长数组即可覆盖；
/// 超长名字（异常谱面）会退化成堆分配的 `String`。
enum AssetKey {
    Stack {
        buffer: [u8; ASSET_KEY_CAPACITY],
        len: usize,
    },
    Heap(String),
}

impl AssetKey {
    /// 构造 `{bank}-{name}`（没有音效组前缀时就是 `{name}`）。
    ///
    /// 注意分隔符：文件名是 `normal-hitnormal`，漏掉 `-` 会查不到任何资源。
    fn new(prefix: Option<&str>, name: &str) -> Self {
        match prefix {
            Some(prefix) => {
                match concat_key!(ASSET_KEY_CAPACITY, &[prefix.as_bytes(), b"-", name.as_bytes()]) {
                    Some((buffer, len)) => Self::Stack { buffer, len },
                    None => Self::Heap(format!("{prefix}-{name}")),
                }
            }
            None => match concat_key!(ASSET_KEY_CAPACITY, &[name.as_bytes()]) {
                Some((buffer, len)) => Self::Stack { buffer, len },
                None => Self::Heap(name.to_string()),
            },
        }
    }

    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Stack { buffer, len } => &buffer[..*len],
            Self::Heap(value) => value.as_bytes(),
        }
    }
}

/// 一个事件的候选名字序列：先带 `bank-` 前缀，再回退到裸名字。
struct AssetCandidates<'a> {
    first: Option<AssetKey>,
    plain: &'a str,
}

impl<'a> AssetCandidates<'a> {
    fn new(prefix: Option<&str>, plain: &'a str) -> Self {
        Self {
            first: prefix.map(|prefix| AssetKey::new(Some(prefix), plain)),
            plain,
        }
    }

    fn iter(&self) -> impl Iterator<Item = &[u8]> {
        self.first
            .iter()
            .map(AssetKey::as_bytes)
            .chain(std::iter::once(self.plain.as_bytes()))
    }
}

/// taiko 的候选名：legacy taiko 会把 `taiko-` 插到文件名前面。
///
/// 目标是 `taiko-{bank}-{name}`（例如 `taiko-drum-hitnormal`），全部在栈上拼；
/// 超长名字退化成堆分配的 `String`。
enum TaikoCandidates {
    Stack {
        buffer: [u8; ASSET_KEY_CAPACITY],
        len: usize,
    },
    Heap(String),
}

impl TaikoCandidates {
    fn new(bank: &str, name: &str) -> Self {
        let total = b"taiko-".len() + bank.len() + 1 + name.len();
        if total <= ASSET_KEY_CAPACITY {
            let mut buffer = [0_u8; ASSET_KEY_CAPACITY];
            let mut len = 0;
            for part in [b"taiko-".as_slice(), bank.as_bytes(), b"-", name.as_bytes()] {
                buffer[len..len + part.len()].copy_from_slice(part);
                len += part.len();
            }
            Self::Stack { buffer, len }
        } else {
            Self::Heap(format!("taiko-{bank}-{name}"))
        }
    }

    fn iter(&self) -> impl Iterator<Item = &[u8]> {
        match self {
            Self::Stack { buffer, len } => Some(&buffer[..*len]),
            Self::Heap(value) => Some(value.as_bytes()),
        }
        .into_iter()
    }
}

/// 时间轴构建器。
///
/// 泛型而不是 `dyn SampleResolver`：解析器接口带泛型方法，且两种实现互斥
/// （样本库查找 / 收集名字），不需要动态分发。
struct TimelineBuilder<'a, R: SampleResolver> {
    resolver: &'a mut R,
    events: Vec<PlayEvent>,
}

impl<'a, R: SampleResolver> TimelineBuilder<'a, R> {
    fn new(resolver: &'a mut R) -> Self {
        Self {
            resolver,
            events: Vec::new(),
        }
    }

    /// 按候选名序列推送事件；全部候选都缺失时按静音跳过。
    ///
    /// 候选名由调用方以栈缓冲构造（见 [`AssetCandidates`]），这里完全不分配内存。
    fn push_at<'b, I: Iterator<Item = &'b [u8]>>(
        &mut self,
        candidates: I,
        volume: i32,
        start_ms: f64,
        duration_ms: f64,
        looping: bool,
    ) {
        if !start_ms.is_finite() {
            return;
        }
        let Some(source_id) = self.resolver.resolve(candidates) else {
            return;
        };
        self.events.push(PlayEvent {
            start_ms,
            duration_ms: if duration_ms.is_finite() {
                duration_ms.max(0.0)
            } else {
                0.0
            },
            source_id,
            gain: volume_gain(volume),
            looping,
        });
    }

    fn push_named(
        &mut self,
        bank: SampleBank,
        name: &str,
        volume: i32,
        start_ms: f64,
        duration_ms: f64,
        looping: bool,
    ) {
        self.push_at(
            AssetCandidates::new(bank.prefix(), name).iter(),
            volume,
            start_ms,
            duration_ms,
            looping,
        );
    }

    /// taiko 的查找名：`taiko-{bank}-{name}`。
    fn push_taiko(
        &mut self,
        bank: SampleBank,
        name: &str,
        volume: i32,
        start_ms: f64,
        duration_ms: f64,
        looping: bool,
    ) {
        let Some(prefix) = bank.prefix() else {
            return;
        };
        self.push_at(
            TaikoCandidates::new(prefix, name).iter(),
            volume,
            start_ms,
            duration_ms,
            looping,
        );
    }

    fn push_sample(
        &mut self,
        sample: &HitSample,
        beatmap: &Beatmap,
        start_ms: f64,
        duration_ms: f64,
        looping: bool,
    ) {
        let default = timing_point_at(beatmap, start_ms as i64);
        let bank = if sample.bank == SampleBank::Auto {
            default.map_or(SampleBank::Normal, |point| timing_sample_bank(beatmap, point))
        } else {
            sample.bank
        };
        let volume = if sample.volume > 0 {
            sample.volume
        } else {
            default.map_or(100, |point| point.sample_volume)
        };
        match sample.filename.as_deref() {
            // 自定义文件名优先：先查带 bank 前缀的名字，再回退到裸文件名
            // （与 `HitSampleInfo.LookupNames` 的 `Gameplay/{bank}-{name}` 规则一致）。
            Some(filename) => self.push_at(
                AssetCandidates::new(bank.prefix(), filename).iter(),
                volume,
                start_ms,
                duration_ms,
                looping,
            ),
            None => self.push_named(
                bank,
                sample.addition.suffix(),
                volume,
                start_ms,
                duration_ms,
                looping,
            ),
        }
    }

    fn push_samples(
        &mut self,
        samples: &[HitSample],
        beatmap: &Beatmap,
        start_ms: f64,
        duration_ms: f64,
    ) {
        for sample in samples {
            self.push_sample(sample, beatmap, start_ms, duration_ms, false);
        }
    }

    /// 将已有样本改成滑条/果汁流使用的样本名，同时保留其音效组、音量和自定义文件名。
    fn push_transformed_samples(
        &mut self,
        samples: &[HitSample],
        beatmap: &Beatmap,
        name: &str,
        start_ms: f64,
        duration_ms: f64,
        looping: bool,
    ) {
        for sample in samples {
            let default = timing_point_at(beatmap, start_ms as i64);
            let bank = if sample.bank == SampleBank::Auto {
                default.map_or(SampleBank::Normal, |point| timing_sample_bank(beatmap, point))
            } else {
                sample.bank
            };
            let volume = if sample.volume > 0 {
                sample.volume
            } else {
                default.map_or(100, |point| point.sample_volume)
            };
            match sample.filename.as_deref() {
                Some(filename) => self.push_at(
                    AssetCandidates::new(bank.prefix(), filename).iter(),
                    volume,
                    start_ms,
                    duration_ms,
                    looping,
                ),
                None => self.push_named(bank, name, volume, start_ms, duration_ms, looping),
            }
        }
    }

    fn finish(mut self) -> HitsoundTimeline {
        self.events.sort_by(|a, b| {
            a.start_ms
                .partial_cmp(&b.start_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        HitsoundTimeline {
            events: self.events,
        }
    }
}

/// 根据谱面与目标模式生成完整的打击音时间轴。
///
/// 模式取 `beatmap.mode()`：0/1/2/3 分别对应 standard/taiko/catch/mania。
/// standard 谱面在 taiko 模式下也会按 taiko 规则发声。
pub fn build_timeline(beatmap: &Beatmap, library: &SampleLibrary) -> HitsoundTimeline {
    // 只读借用即可完成解析：`SampleResolver` 对 `&SampleLibrary` 有实现，
    // 不需要克隆整份样本数据。
    let mut resolver = library;
    build_with(beatmap, &mut resolver)
}

/// 收集时间轴会用到的全部候选样本名，供宿主按需预解码。
pub fn referenced_names(beatmap: &Beatmap) -> Vec<String> {
    let mut collector = CollectNames::default();
    build_with(beatmap, &mut collector);
    collector.into_names()
}

fn build_with<R: SampleResolver>(beatmap: &Beatmap, resolver: &mut R) -> HitsoundTimeline {
    let mut builder = TimelineBuilder::new(resolver);
    match &beatmap.hit_objects {
        HitObjects::Taiko(objects) => {
            for object in objects {
                push_taiko_object(&mut builder, object, beatmap);
            }
        }
        HitObjects::Catch(objects) => {
            for object in objects {
                push_catch(&mut builder, object, beatmap);
            }
        }
        HitObjects::Mania(objects) => {
            for object in objects {
                push_mania(&mut builder, object, beatmap);
            }
        }
        HitObjects::Standard(objects) => {
            if beatmap.mode() == 1 {
                for object in objects {
                    push_standard_as_taiko(&mut builder, object, beatmap);
                }
            } else {
                for object in objects {
                    push_standard(&mut builder, object, beatmap);
                }
            }
        }
    }
    builder.finish()
}

// ---------------------------------------------------------------------------
// standard
// ---------------------------------------------------------------------------

fn push_standard<R: SampleResolver>(builder: &mut TimelineBuilder<R>, object: &StandardHitObject, beatmap: &Beatmap) {
    let start = object.start_time as f64;
    let (head_bank, head_volume) = head_sample(&object.samples, beatmap, object.start_time);
    push_declared_samples(builder, &object.samples, object.hitsound, beatmap, object.start_time);
    if object.hit_type & 2 != 0 {
        push_standard_slider(builder, object, beatmap, head_bank, head_volume);
    } else if object.hit_type & 8 != 0 {
        // 转盘：旋转循环音 + 奖励音；奖励音只在实际转到可计分圈数时才响，
        // 预览无法预知玩家表现，因此统一按「每次得分」的奖励音播放。
        let duration = (object.end_time - object.start_time).max(0) as f64;
        builder.push_named(
            SampleBank::Normal,
            "spinnerspin",
            head_volume,
            start,
            duration,
            true,
        );
        builder.push_named(SampleBank::Normal, "spinnerbonus", head_volume, start, 0.0, false);
        builder.push_named(
            SampleBank::Normal,
            "spinnerbonus-max",
            head_volume,
            start,
            0.0,
            false,
        );
    }
}

fn push_standard_slider<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    object: &StandardHitObject,
    beatmap: &Beatmap,
    head_bank: SampleBank,
    head_volume: i32,
) {
    let (beat_length, slider_velocity) = slider_timing(object.start_time, beatmap);
    let slider_multiplier = beatmap.difficulty.get_f64_or("SliderMultiplier", 1.4);
    let tick_rate = beatmap.difficulty.get_f64_or("SliderTickRate", 1.0);

    // 滑行音：按住滑条期间循环播放，继承头部普通音的音量。
    if object.end_time > object.start_time {
        let duration = (object.end_time - object.start_time) as f64;
        if let Some(normal) = object
            .samples
            .iter()
            .find(|sample| sample.addition == HitAddition::None)
        {
            builder.push_transformed_samples(
                std::slice::from_ref(normal),
                beatmap,
                "sliderslide",
                object.start_time as f64,
                duration,
                true,
            );
        } else {
            builder.push_named(
                head_bank,
                "sliderslide",
                head_volume,
                object.start_time as f64,
                duration,
                true,
            );
        }
        // osu! 只把头部的 whistle 复制为 sliderwhistle，其他加成音不参与滑行循环。
        if object.hitsound & 2 != 0
            || object
                .samples
                .iter()
                .any(|sample| sample.addition == HitAddition::Whistle)
        {
            if let Some(whistle) = object
                .samples
                .iter()
                .find(|sample| sample.addition == HitAddition::Whistle)
            {
                builder.push_transformed_samples(
                    std::slice::from_ref(whistle),
                    beatmap,
                    "sliderwhistle",
                    object.start_time as f64,
                    duration,
                    true,
                );
            } else {
                builder.push_named(
                    head_bank,
                    "sliderwhistle",
                    head_volume,
                    object.start_time as f64,
                    duration,
                    true,
                );
            }
        }
    }

    // 滑条 tick：使用滑条头的音效组与音量。
    let tick_times = crate::render::cpu::modes::standard::slider::slider_tick_times(
        object.slider_pixel_length,
        object.start_time,
        object.end_time,
        object.slider_repeats,
        beat_length,
        slider_velocity,
        tick_rate,
        slider_multiplier,
    );
    for time in tick_times {
        builder.push_named(head_bank, "slidertick", head_volume, time, 0.0, false);
    }

    // 重复箭头与滑条尾：使用各节点的自定义音效。
    let spans = object.slider_repeats.max(1) as usize;
    let span_duration = (object.end_time - object.start_time) as f64 / spans as f64;
    // edgeSets[1..] 对应重复节点和尾节点，尾节点在没有重复时也必须发声。
    for span in 1..=spans {
        let time = object.start_time as f64 + span as f64 * span_duration;
        match object.slider_edge_samples.get(span - 1) {
            // 节点自带音效：直接使用。
            Some(edge) if !edge.is_empty() => builder.push_samples(edge, beatmap, time, 0.0),
            // 节点没有自带音效：沿用滑条头的完整 hitsound 位掩码。
            _ => {
                builder.push_named(head_bank, "hitnormal", head_volume, time, 0.0, false);
                for addition in HitAddition::all_from_hitsound(object.hitsound) {
                    builder.push_named(head_bank, addition.suffix(), head_volume, time, 0.0, false);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// taiko
// ---------------------------------------------------------------------------

/// 音量分档阈值，与 `VolumeAwareHitSampleInfo` 保持一致。
const TAIKO_VOLUME_HARD: i32 = 90;
const TAIKO_VOLUME_MEDIUM: i32 = 60;

/// taiko 只按采样点音量选音效组，完全忽略谱面声明的音效组。
fn taiko_bank(volume: i32) -> SampleBank {
    if volume >= TAIKO_VOLUME_HARD {
        SampleBank::Drum
    } else if volume >= TAIKO_VOLUME_MEDIUM {
        SampleBank::Normal
    } else {
        SampleBank::Soft
    }
}

/// timing point 提供的默认打击音参数；物件没有自带 `hitSample` 时使用。
///
/// 用值类型返回而不是 `Vec<HitSample>`：绝大多数物件都不带 `hitSample`，
/// 每个物件都分配一个 `Vec` 在大谱面上是纯浪费。
#[derive(Debug, Clone, Copy)]
struct DefaultSample {
    bank: SampleBank,
    volume: i32,
}

impl DefaultSample {
    fn at(beatmap: &Beatmap, time: i64) -> Option<Self> {
        timing_point_at(beatmap, time).map(|point| Self {
            bank: timing_sample_bank(beatmap, point),
            volume: point.sample_volume,
        })
    }
}

/// timing point 的采样组为 0 时沿用 General.SampleSet，而不是强制回到 Normal。
fn timing_sample_bank(beatmap: &Beatmap, point: &TimingPoint) -> SampleBank {
    if point.sample_set != 0 {
        return SampleBank::from_set_id(point.sample_set);
    }
    match beatmap.general.get("SampleSet").map(str::trim).unwrap_or("") {
        value if value.eq_ignore_ascii_case("soft") => SampleBank::Soft,
        value if value.eq_ignore_ascii_case("drum") => SampleBank::Drum,
        _ => SampleBank::Normal,
    }
}

/// 物件头部的（音效组, 音量）：优先用物件自带的 `hitSample`，否则回退到 timing point。
fn head_sample(samples: &[HitSample], beatmap: &Beatmap, time: i64) -> (SampleBank, i32) {
    match samples.first() {
        Some(sample) => {
            let default = timing_point_at(beatmap, time);
            let bank = if sample.bank == SampleBank::Auto {
                default.map_or(SampleBank::Normal, |point| timing_sample_bank(beatmap, point))
            } else {
                sample.bank
            };
            let volume = if sample.volume > 0 {
                sample.volume
            } else {
                default.map_or(100, |point| point.sample_volume)
            };
            (bank, volume)
        }
        None => DefaultSample::at(beatmap, time).map_or((SampleBank::Normal, 100), |default| {
            (default.bank, default.volume)
        }),
    }
}

/// 追加物件自带的打击音；没有自带参数时由调用方按 timing point 生成。
fn push_declared_samples<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    samples: &[HitSample],
    hitsound: i32,
    beatmap: &Beatmap,
    time: i64,
) {
    if samples.is_empty() {
        if let Some(default) = DefaultSample::at(beatmap, time) {
            builder.push_named(default.bank, "hitnormal", default.volume, time as f64, 0.0, false);
            for addition in HitAddition::all_from_hitsound(hitsound) {
                builder.push_named(
                    default.bank,
                    addition.suffix(),
                    default.volume,
                    time as f64,
                    0.0,
                    false,
                );
            }
        }
        return;
    }
    builder.push_samples(samples, beatmap, time as f64, 0.0);
}

/// taiko 的默认（无自带 hitSample 时）音量。
fn taiko_volume(samples: &[HitSample], beatmap: &Beatmap, time: i64) -> i32 {
    samples
        .first()
        .map_or_else(|| DefaultSample::at(beatmap, time).map_or(100, |d| d.volume), |s| {
            if s.volume > 0 {
                s.volume
            } else {
                DefaultSample::at(beatmap, time).map_or(100, |d| d.volume)
            }
        })
}

fn push_taiko_object<R: SampleResolver>(builder: &mut TimelineBuilder<R>, object: &TaikoHitObject, beatmap: &Beatmap) {
    // taiko 忽略音效组声明，只按音量分档；音量优先取物件自带采样点，否则用 timing point。
    let volume = taiko_volume(&object.samples, beatmap, object.start_time);
    // 未设置 rim 位的是红音符（center），设置了的是蓝音符（rim）。
    let is_rim = object.hitsound & 8 != 0;
    let name = if is_rim { "hitclap" } else { "hitnormal" };
    builder.push_taiko(
        taiko_bank(volume),
        name,
        volume,
        object.start_time as f64,
        0.0,
        false,
    );
}

/// standard 谱面按 taiko 规则发声：鼓点来自转谱结果。
fn push_standard_as_taiko<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    object: &StandardHitObject,
    beatmap: &Beatmap,
) {
    for hit in crate::domain::rulesets::taiko::taiko_hitsound_events(beatmap, object) {
        // 转谱后的 hitsound 只保留 center/rim 位；音量沿用源物件（缺失时取 timing point）。
        let volume = taiko_volume(&object.samples, beatmap, hit.time_ms as i64);
        let is_rim = hit.hitsound & 8 != 0;
        let name = if is_rim { "hitclap" } else { "hitnormal" };
        builder.push_taiko(taiko_bank(volume), name, volume, hit.time_ms, 0.0, false);
    }
}

// ---------------------------------------------------------------------------
// catch
// ---------------------------------------------------------------------------

fn push_catch<R: SampleResolver>(builder: &mut TimelineBuilder<R>, object: &CatchHitObject, beatmap: &Beatmap) {
    let (head_bank, head_volume) = head_sample(&object.samples, beatmap, object.start_time);
    push_declared_samples(builder, &object.samples, object.hitsound, beatmap, object.start_time);

    if object.hit_type & 2 == 0 {
        return;
    }

    // 果汁流：小果与节点都使用 `slidertick`，时间规则与滑条 tick 一致。
    let (beat_length, slider_velocity) = slider_timing(object.start_time, beatmap);
    let slider_multiplier = beatmap.difficulty.get_f64_or("SliderMultiplier", 1.4);
    let tick_rate = beatmap.difficulty.get_f64_or("SliderTickRate", 1.0);
    let times = crate::render::cpu::modes::standard::slider::slider_tick_times(
        object.slider_pixel_length,
        object.start_time,
        object.end_time,
        object.slider_repeats,
        beat_length,
        slider_velocity,
        tick_rate,
        slider_multiplier,
    );
    for time in times {
        // 果汁流的每个小果都把头部样本名替换为 slidertick，保留所有层和音量。
        if object.samples.is_empty() {
            builder.push_named(head_bank, "slidertick", head_volume, time, 0.0, false);
            for _ in HitAddition::all_from_hitsound(object.hitsound) {
                builder.push_named(head_bank, "slidertick", head_volume, time, 0.0, false);
            }
        } else {
            builder.push_transformed_samples(
                &object.samples,
                beatmap,
                "slidertick",
                time,
                0.0,
                false,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// mania
// ---------------------------------------------------------------------------

fn push_mania<R: SampleResolver>(builder: &mut TimelineBuilder<R>, object: &ManiaHitObject, beatmap: &Beatmap) {
    // 原生 mania 长条只在头部播放样本；持续滑行音只属于转换生成的 hold。
    push_declared_samples(builder, &object.samples, 0, beatmap, object.start_time);
}

// ---------------------------------------------------------------------------
// 公共辅助
// ---------------------------------------------------------------------------

/// 返回 `start_time` 之前最后一条 timing point 生效的 (beat_length, slider_velocity)。
fn slider_timing(start_time: i64, beatmap: &Beatmap) -> (f64, f64) {
    let mut beat_length = beatmap
        .timing_points
        .first()
        .map_or(500.0, |point| point.beat_length);
    let mut slider_velocity = 1.0;
    for point in &beatmap.timing_points {
        if point.time > start_time as f64 {
            break;
        }
        if point.uninherited {
            beat_length = point.beat_length;
            slider_velocity = 1.0;
        } else if point.beat_length < 0.0 {
            slider_velocity = -100.0 / point.beat_length;
        }
    }
    (beat_length, slider_velocity)
}

fn timing_point_at(beatmap: &Beatmap, time: i64) -> Option<&TimingPoint> {
    let mut active: Option<&TimingPoint> = None;
    for point in &beatmap.timing_points {
        if point.time <= time as f64 {
            active = Some(point);
        } else {
            break;
        }
    }
    active.or_else(|| beatmap.timing_points.first())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{
        CatchHitObject, HitAddition, HitSample, HitObjects, ManiaHitObject, SampleBank,
        StandardHitObject, TaikoHitObject,
    };
    use crate::hitsound::mixer::soft_limit;

    /// 构造只有一条红线的最小谱面，用于时间轴测试。
    fn beatmap_with(mode: i32, objects: HitObjects) -> Beatmap {
        Beatmap {
            metadata: Default::default(),
            difficulty: Default::default(),
            general: Default::default(),
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
            hit_objects: objects,
            break_periods: Vec::new(),
            background_filename: None,
            combo_colors: Vec::new(),
            beat_divisor: 0,
        }
        .with_mode(mode)
    }

    trait WithMode {
        fn with_mode(self, mode: i32) -> Beatmap;
    }

    impl WithMode for Beatmap {
        fn with_mode(mut self, mode: i32) -> Beatmap {
            self.general.insert("Mode", mode.to_string());
            self
        }
    }

    fn object_sample(bank: SampleBank, volume: i32) -> Vec<HitSample> {
        vec![HitSample::new(bank, HitAddition::None, volume, None)]
    }

    fn library_with(names: &[&str]) -> SampleLibrary {
        let mut library = SampleLibrary::new();
        for name in names {
            library.insert(
                *name,
                SampleData::stereo(vec![1.0, 1.0, 1.0, 1.0], 1000),
            );
        }
        library
    }

    #[test]
    fn 滑条的音效参数取自正确列() {
        // 回归：滑条第 6 列是曲线（`B|356:192`），曾把它当成 hitSample 解析出
        // additionSet=192，导致音效组与音量全错。
        let source = "osu file format v14\n\n[General]\nMode: 0\n\n[Difficulty]\nCircleSize:4\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,0,100,1,0\n\n[HitObjects]\n256,192,2000,2,0,B|356:192,1,140\n";
        let beatmap = crate::parse_beatmap_bytes(source.as_bytes()).expect("fixture 必须可解析");
        let object = &beatmap.hit_objects.as_standard().expect("必须是 standard 谱面")[0];
        // hitSample 全为 0：物件不覆盖任何参数，交给 timing point 决定。
        assert!(object.samples.is_empty(), "samples={:?}", object.samples);
        let names = referenced_names(&beatmap);
        assert!(names.contains(&"soft-hitnormal".to_string()), "names={names:?}");
        assert!(names.contains(&"soft-sliderslide".to_string()), "names={names:?}");
    }

    #[test]
    fn 物件缺省音效参数时回退到timing_point() {
        let library = library_with(&["soft-hitnormal", "drum-hitnormal"]);
        let mut beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![
                // 没有自带 hitSample：应当使用 timing point 的 soft 组与音量。
                StandardHitObject {
                    start_time: 1000,
                    end_time: 1000,
                    hit_type: 1,
                    hitsound: 0,
                    ..Default::default()
                },
                // 自带 hitSample：应当覆盖 timing point 的 soft 组。
                StandardHitObject {
                    start_time: 2000,
                    end_time: 2000,
                    hit_type: 1,
                    hitsound: 0,
                    samples: vec![HitSample::new(SampleBank::Drum, HitAddition::None, 40, None)],
                    ..Default::default()
                },
            ]),
        );
        beatmap.timing_points[0].sample_set = crate::domain::models::SAMPLE_SET_SOFT;

        let timeline = build_timeline(&beatmap, &library);
        assert_eq!(timeline.len(), 2);
        assert_eq!(library.name_of(timeline.events[0].source_id), Some("soft-hitnormal"));
        assert_eq!(library.name_of(timeline.events[1].source_id), Some("drum-hitnormal"));
    }

    #[test]
    fn 落在窗口边界的打击音不会被跳过() {
        // 回归：曾经用 `start_ms >= window_end` 收集事件，正好落在窗口末尾的事件
        // 会被永久跳过（事件按开始时间升序，之后再也扫不到），表现为整点打击音静音。
        let mut library = SampleLibrary::new();
        // 采样率与混音一致（1000Hz），每个采样帧 1ms。
        library.insert(
            "normal-hitnormal",
            SampleData::stereo(vec![0.5, 0.5, 0.5, 0.5, 0.5, 0.5], 1000),
        );
        let mut beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                start_time: 1000,
                end_time: 1000,
                hit_type: 1,
                hitsound: 0,
                ..Default::default()
            }]),
        );
        beatmap.timing_points[0].sample_set = 1;
        let timeline = build_timeline(&beatmap, &library);
        assert_eq!(timeline.len(), 1);

        let mut mixer = HitsoundMixer::new(library, timeline, 1000);
        // 第一个窗口 [0, 1000ms)：事件正好在末尾开始，本窗口内不应有声。
        let first = mixer.render(1000);
        assert!(first.iter().all(|value| *value == 0.0));
        // 第二个窗口 [1000, 2000ms)：必须能听到这个事件。
        let second = mixer.render(1000);
        assert!(
            second.iter().any(|value| *value > 0.0),
            "窗口边界处的事件被跳过了"
        );
    }

    #[test]
    fn 音量曲线与游戏内一致() {
        assert!((volume_gain(100) - 1.0).abs() < 1e-9);
        assert!((volume_gain(50) - 0.5).abs() < 1e-12);
        assert!((volume_gain(0) - 0.0).abs() < 1e-12);
        // 越界音量按边界处理，不会 panic。
        assert!((volume_gain(-20) - volume_gain(0)).abs() < 1e-12);
        assert!((volume_gain(500) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn 缺失样本只产生静音不产生事件() {
        let beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                start_time: 1000,
                end_time: 1000,
                hit_type: 1,
                hitsound: 0,
                samples: object_sample(SampleBank::Normal, 100),
                ..Default::default()
            }]),
        );
        // 空样本库：没有任何事件，也不会 panic。
        let empty = SampleLibrary::new();
        assert!(build_timeline(&beatmap, &empty).is_empty());

        // 只有 drum 存在时，normal 物件不会命中 drum。
        let drum_only = library_with(&["drum-hitnormal"]);
        assert!(build_timeline(&beatmap, &drum_only).is_empty());

        let normal = library_with(&["normal-hitnormal"]);
        let timeline = build_timeline(&beatmap, &normal);
        assert_eq!(timeline.len(), 1);
        assert!((timeline.events[0].start_ms - 1000.0).abs() < 1e-9);
        assert!((timeline.events[0].gain - 1.0).abs() < 1e-9);
    }

    #[test]
    fn 引用名收集覆盖全部候选与裸名回退() {
        let beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                start_time: 1000,
                end_time: 1000,
                hit_type: 1,
                // 10 = 哨音(2) + 拍手(8) 位；两个加成音都必须保留。
                hitsound: 10,
                samples: vec![
                    HitSample::new(SampleBank::Soft, HitAddition::None, 80, None),
                    HitSample::new(SampleBank::Soft, HitAddition::Whistle, 80, None),
                ],
                ..Default::default()
            }]),
        );
        let names = referenced_names(&beatmap);
        // soft 音效组会同时登记带 bank 前缀的名字与裸名（裸名是 osu! 的回退查找）。
        for expected in ["soft-hitnormal", "hitnormal", "soft-hitwhistle", "hitwhistle"] {
            assert!(
                names.contains(&expected.to_string()),
                "缺少候选名 {expected}：{names:?}"
            );
        }
    }

    #[test]
    fn 自定义文件名只覆盖普通层() {
        let source = "osu file format v14\n\n[General]\nMode: 0\n\n[Difficulty]\nCircleSize:4\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,10,0:0:0:100:custom-hit.ogg\n";
        let beatmap = crate::parse_beatmap_bytes(source.as_bytes()).unwrap();
        let object = &beatmap.hit_objects.as_standard().unwrap()[0];
        assert_eq!(object.samples.len(), 3);
        assert_eq!(object.samples[0].filename.as_deref(), Some("custom-hit.ogg"));
        assert!(object.samples[1..].iter().all(|sample| sample.filename.is_none()));
    }

    #[test]
    fn taiko按音量选择音效组() {
        let objects = |volume: i32| {
            HitObjects::Taiko(vec![TaikoHitObject {
                start_time: 1000,
                end_time: 1000,
                hit_type: 0,
                hitsound: 0,
                samples: object_sample(SampleBank::Normal, volume),
            }])
        };
        let library = library_with(&[
            "taiko-soft-hitnormal",
            "taiko-normal-hitnormal",
            "taiko-drum-hitnormal",
        ]);

        let cases = [(50, "taiko-soft-hitnormal"), (70, "taiko-normal-hitnormal"), (95, "taiko-drum-hitnormal")];
        for (volume, expected) in cases {
            let beatmap = beatmap_with(1, objects(volume));
            let timeline = build_timeline(&beatmap, &library);
            assert_eq!(timeline.len(), 1, "音量 {volume}");
            assert_eq!(
                library.name_of(timeline.events[0].source_id),
                Some(expected),
                "音量 {volume}"
            );
        }
    }

    #[test]
    fn taiko蓝音符使用hitclap() {
        let library = library_with(&["taiko-normal-hitclap", "taiko-normal-hitnormal"]);
        let beatmap = beatmap_with(
            1,
            HitObjects::Taiko(vec![TaikoHitObject {
                start_time: 500,
                end_time: 500,
                // 位 3 表示 rim（蓝音符）。
                hitsound: 8,
                hit_type: 0,
                samples: object_sample(SampleBank::Normal, 70),
            }]),
        );
        let timeline = build_timeline(&beatmap, &library);
        assert_eq!(timeline.len(), 1);
        assert_eq!(
            library.name_of(timeline.events[0].source_id),
            Some("taiko-normal-hitclap")
        );
    }

    #[test]
    fn 滑条生成滑行音与tick事件() {
        let library = library_with(&["normal-sliderslide", "normal-slidertick"]);
        let beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                x: 0,
                y: 0,
                start_time: 1000,
                end_time: 3000,
                hit_type: 2,
                hitsound: 0,
                slider_type: Some("L".to_string()),
                slider_points: vec![(100, 0)],
                slider_repeats: 1,
                slider_pixel_length: 300.0,
                samples: object_sample(SampleBank::Normal, 100),
                ..Default::default()
            }]),
        );
        let timeline = build_timeline(&beatmap, &library);
        // 滑条头本身没有可用样本，因此只应出现滑行音与 tick。
        let slide = timeline
            .events
            .iter()
            .find(|event| library.name_of(event.source_id) == Some("normal-sliderslide"))
            .expect("必须生成滑行音事件");
        assert!(slide.looping);
        assert!((slide.duration_ms - 2000.0).abs() < 1e-9);
        assert!(
            timeline
                .events
                .iter()
                .any(|event| library.name_of(event.source_id) == Some("normal-slidertick")),
            "必须生成滑条 tick 事件"
        );
    }

    #[test]
    fn mania按timing_point采样组发声() {
        let mut beatmap = beatmap_with(
            3,
            HitObjects::Mania(vec![ManiaHitObject {
                lane: 0,
                start_time: 1000,
                end_time: 1000,
                is_long_note: false,
                samples: Vec::new(),
            }]),
        );
        beatmap.timing_points[0].sample_set = crate::domain::models::SAMPLE_SET_DRUM;
        beatmap.timing_points[0].sample_volume = 80;

        let library = library_with(&["drum-hitnormal"]);
        let timeline = build_timeline(&beatmap, &library);
        assert_eq!(timeline.len(), 1);
        // 采样音量 80 → 0.8 线性增益
        let expected = 0.8;
        assert!(
            (timeline.events[0].gain - expected).abs() < 1e-12,
            "gain={} expected={}",
            timeline.events[0].gain,
            expected
        );
    }

    #[test]
    fn 香蕉使用独立查找名() {
        let library = library_with(&["catch-banana"]);
        let beatmap = beatmap_with(
            2,
            HitObjects::Catch(vec![CatchHitObject {
                x: 0,
                y: 0,
                start_time: 1000,
                end_time: 1000,
                hit_type: 1,
                samples: vec![HitSample::new(
                    // 音效组不是 Custom：此时才会先查 `bank-name`，再回退到裸名字。
                    SampleBank::Normal,
                    HitAddition::None,
                    100,
                    Some("catch-banana".to_string()),
                )],
                ..Default::default()
            }]),
        );
        let timeline = build_timeline(&beatmap, &library);
        assert!(library.contains("catch-banana"));
        assert!(
            !timeline.is_empty(),
            "香蕉应产生事件，候选名 = {:?}",
            AssetCandidates::new(SampleBank::Normal.prefix(), "hitnormal")
                .iter()
                .map(|name| String::from_utf8_lossy(name).into_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(library.name_of(timeline.events[0].source_id), Some("catch-banana"));
    }

    #[test]
    fn 混音器按时间与主音量输出() {
        let library = library_with(&["normal-hitnormal"]);
        let beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                start_time: 0,
                end_time: 0,
                hit_type: 1,
                hitsound: 0,
                samples: object_sample(SampleBank::Normal, 100),
                ..Default::default()
            }]),
        );
        let timeline = build_timeline(&beatmap, &library);

        // 样本采样率 1000Hz，每个采样帧 1ms；4 帧 = 4ms。
        let mut mixer = HitsoundMixer::new(library, timeline, 1000);
        let quiet = mixer.render(2);
        assert_eq!(quiet.len(), 4);
        // 事件增益 1.0 × 主音量 0.5，样本值为 1.0。
        mixer.set_master_gain(0.5);
        mixer.seek(0.0);
        let mixed = mixer.render(2);
        let expected = soft_limit(0.5);
        assert!((mixed[0] - expected).abs() < 1e-6, "left={}", mixed[0]);
        assert!((mixed[1] - expected).abs() < 1e-6, "right={}", mixed[1]);
    }

    #[test]
    fn 混音器seek后不补播已越过的事件() {
        let library = library_with(&["normal-hitnormal"]);
        let beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                start_time: 0,
                end_time: 0,
                hit_type: 1,
                hitsound: 0,
                samples: object_sample(SampleBank::Normal, 100),
                ..Default::default()
            }]),
        );
        let timeline = build_timeline(&beatmap, &library);
        let mut mixer = HitsoundMixer::new(library, timeline, 1000);
        mixer.seek(500.0);
        let output = mixer.render(2);
        assert!(output.iter().all(|value| *value == 0.0));
    }

    #[test]
    fn 混音器主音量忽略非有限值() {
        let library = SampleLibrary::new();
        let mut mixer = HitsoundMixer::new(library, HitsoundTimeline::default(), 1000);
        mixer.set_master_gain(f64::NAN);
        assert_eq!(mixer.master_gain(), 0.0);
        mixer.set_master_gain(-1.0);
        assert_eq!(mixer.master_gain(), 0.0);
    }
}
