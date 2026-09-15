//! 打击音事件与时间轴构建器。
//!
//! 一个事件是「什么时候、用哪个样本、多大声、以什么音高播放」；时间轴按开始时间升序。
//! 事件生成走栈上缓冲拼接候选名，不在热路径上分配 `String`。

use crate::domain::models::{Beatmap, HitSample, SampleBank};

use super::common::{timing_point_at, timing_sample_bank};
use super::sample::SampleResolver;
use super::volume_gain;

/// 播放频率（音高倍率）随时间的线性斜坡。
///
/// 普通打击音是恒定 1.0 倍；osu! 的转盘旋转音则会随旋转进度升高音调
/// （`DrawableSpinner`：起始 `20000/44100`、比例 `40000/44100`、上限 `100000/44100`），
/// 因此事件需要能表达「倍率随时间变化」而不只是一个常数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayFrequency {
    /// 事件开始时的倍率。
    pub start: f64,
    /// 每毫秒的倍率增量。
    pub per_ms: f64,
    /// 倍率上限；到达后保持不变。
    pub max: f64,
}

impl PlayFrequency {
    /// 恒定 1.0 倍：普通打击音与宿主显式触发的默认值。
    pub const UNITY: Self = Self {
        start: 1.0,
        per_ms: 0.0,
        max: 1.0,
    };

    /// 从 `start` 起每毫秒增加 `per_ms`，到 `max` 封顶。
    pub const fn ramp(start: f64, per_ms: f64, max: f64) -> Self {
        Self { start, per_ms, max }
    }

    /// 倍率对经过时间的积分（单位：毫秒 × 倍率）。
    ///
    /// 混音器必须用它反推样本位置，而不是「瞬时倍率 × 经过时间」：后者的瞬时播放速度
    /// 是 `f + t·f'`，音高会越跑越高，与 osu! 的线性升调不一致。
    pub fn integral(&self, elapsed_ms: f64) -> f64 {
        if !elapsed_ms.is_finite() || elapsed_ms <= 0.0 {
            return 0.0;
        }
        // 坏谱面（NaN 的 OD、非有限时长）不能让混音输出 NaN：非有限值退回 1.0 倍。
        let start = sanitize_frequency(self.start, 1.0);
        let max = sanitize_frequency(self.max, start).max(start);
        let per_ms = if self.per_ms.is_finite() && self.per_ms > 0.0 {
            self.per_ms
        } else {
            0.0
        };
        if per_ms <= 0.0 {
            return start * elapsed_ms;
        }
        // 到达上限的时刻 `cap_ms`：之前按二次曲线积分（线性升速），之后按恒定上限积分。
        let cap_ms = (max - start) / per_ms;
        let capped = start * cap_ms + 0.5 * per_ms * cap_ms * cap_ms;
        if elapsed_ms <= cap_ms {
            start * elapsed_ms + 0.5 * per_ms * elapsed_ms * elapsed_ms
        } else {
            capped + max * (elapsed_ms - cap_ms)
        }
    }
}

impl Default for PlayFrequency {
    fn default() -> Self {
        Self::UNITY
    }
}

/// 非有限或非正的倍率回退到 `fallback`。
fn sanitize_frequency(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

/// 一个待播放的打击音事件。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayEvent {
    /// 相对于谱面零时刻的毫秒时间。
    pub start_ms: f64,
    /// 播放时长；0 表示按样本自身长度播放，循环事件则一直持续到该时长结束。
    pub duration_ms: f64,
    /// 样本在 [`SampleLibrary`](crate::hitsound::SampleLibrary) 中的 id。
    pub source_id: usize,
    /// 线性增益（已包含谱面音量与设计音量）。
    pub gain: f64,
    /// 是否为循环音（滑条滑行音）。
    pub looping: bool,
    /// 播放频率（音高倍率）随时间的斜坡；普通打击音是恒定 1.0。
    pub frequency: PlayFrequency,
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
pub(super) struct TimelineBuilder<'a, R: SampleResolver> {
    resolver: &'a mut R,
    events: Vec<PlayEvent>,
}

impl<'a, R: SampleResolver> TimelineBuilder<'a, R> {
    pub(super) fn new(resolver: &'a mut R) -> Self {
        Self {
            resolver,
            events: Vec::new(),
        }
    }

    /// 按候选名序列推送事件；全部候选都缺失时按静音跳过。
    ///
    /// 候选名由调用方以栈缓冲构造（见 [`AssetCandidates`]），这里完全不分配内存。
    pub(super) fn push_at<'b, I: Iterator<Item = &'b [u8]>>(
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
        self.push_resolved(
            source_id,
            volume,
            start_ms,
            duration_ms,
            looping,
            PlayFrequency::UNITY,
        );
    }

    /// 推送一个已经解析到样本 id 的事件。
    fn push_resolved(
        &mut self,
        source_id: usize,
        volume: i32,
        start_ms: f64,
        duration_ms: f64,
        looping: bool,
        frequency: PlayFrequency,
    ) {
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
            frequency,
        });
    }

    /// 推送转盘旋转循环音：只有它需要按旋转进度调制播放倍率，因此单独开一个入口，
    /// 不让 `frequency` 参数扩散到所有普通打击音的调用点。
    pub(super) fn push_spinner(
        &mut self,
        bank: SampleBank,
        volume: i32,
        start_ms: f64,
        duration_ms: f64,
        frequency: PlayFrequency,
    ) {
        if !start_ms.is_finite() {
            return;
        }
        let candidates = AssetCandidates::new(bank.prefix(), "spinnerspin");
        let Some(source_id) = self.resolver.resolve(candidates.iter()) else {
            return;
        };
        self.push_resolved(source_id, volume, start_ms, duration_ms, true, frequency);
    }

    pub(super) fn push_named(
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
    pub(super) fn push_taiko(
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

    pub(super) fn push_sample(
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

    pub(super) fn push_samples(
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
    pub(super) fn push_transformed_samples(
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

    pub(super) fn finish(mut self) -> HitsoundTimeline {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 升调斜坡到上限后按恒定倍率积分() {
        let frequency = PlayFrequency::ramp(1.0, 0.001, 2.0);
        assert_eq!(frequency.integral(0.0), 0.0);
        assert_eq!(frequency.integral(-5.0), 0.0);
        // 1000ms 到达上限 2.0：积分 = 1000 + 0.5 × 0.001 × 1000² = 1500。
        assert!((frequency.integral(1000.0) - 1500.0).abs() < 1e-9);
        // 之后按恒定 2.0 线性增长。
        assert!((frequency.integral(2000.0) - 3500.0).abs() < 1e-9);
        assert_eq!(PlayFrequency::UNITY.integral(123.0), 123.0);
        // 坏谱面的 NaN 倍率不能让混音输出 NaN。
        assert_eq!(
            PlayFrequency::ramp(f64::NAN, f64::NAN, f64::NAN).integral(10.0),
            10.0
        );
    }

}
