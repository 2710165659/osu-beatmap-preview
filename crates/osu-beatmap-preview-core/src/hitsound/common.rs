//! 各模式共用的取样与 timing point 辅助。

use crate::domain::models::{Beatmap, HitAddition, HitSample, SampleBank, TimingPoint};

use super::sample::SampleResolver;
use super::timeline::TimelineBuilder;

/// 打击音取样时 timing point 查找的滞后量（毫秒）。
///
/// 与 osu! `LegacyBeatmapDecoder` 的 `CONTROL_POINT_LENIENCY` 一致：打击音参数用的是
/// 「物件 / 节点时间 + 5ms」处生效的 `SampleControlPoint`（`applySamples`），因此紧跟在
/// 物件之后 5ms 内的音量、音效组或自定义索引变化也算在它头上。
pub(super) const SAMPLE_LENIENCY_MS: i64 = 5;

/// 打击音取样使用的 timing point（带 [`SAMPLE_LENIENCY_MS`] 偏移）。
///
/// 只用于样本参数；渲染用的拍长 / SV 查找仍然是精确时刻，不能共用这个偏移。
pub(super) fn sample_point_at(beatmap: &Beatmap, time: i64) -> Option<&TimingPoint> {
    timing_point_at(beatmap, time.saturating_add(SAMPLE_LENIENCY_MS))
}

/// timing point 提供的默认打击音参数；物件没有自带 `hitSample` 时使用。
///
/// 用值类型返回而不是 `Vec<HitSample>`：绝大多数物件都不带 `hitSample`，
/// 每个物件都分配一个 `Vec` 在大谱面上是纯浪费。
#[derive(Debug, Clone, Copy)]
pub(super) struct DefaultSample {
    pub(super) bank: SampleBank,
    pub(super) volume: i32,
    /// timing point 的自定义音效索引（`.osu` 第 5 列）。
    pub(super) custom_bank: i32,
}

impl DefaultSample {
    /// 该时刻生效的默认参数；整张图没有任何 timing point 时返回 `None`
    ///（osu! 此时用 `SampleControlPoint.DEFAULT`，由 [`DefaultSample::or_default`] 兜底）。
    pub(super) fn at(beatmap: &Beatmap, time: i64) -> Option<Self> {
        sample_point_at(beatmap, time).map(|point| Self {
            bank: timing_sample_bank(beatmap, point),
            volume: point.sample_volume,
            custom_bank: point.sample_index.max(0),
        })
    }

    /// osu! `SampleControlPoint.DEFAULT`：normal 音效组、100% 音量、不使用谱面音效。
    pub(super) fn or_default(default: Option<Self>) -> Self {
        default.unwrap_or(Self {
            bank: SampleBank::Normal,
            volume: 100,
            custom_bank: 0,
        })
    }
}

/// 物件头部的取样参数：音效组、音量与自定义音效索引。
#[derive(Debug, Clone, Copy)]
pub(super) struct HeadSample {
    pub(super) bank: SampleBank,
    pub(super) volume: i32,
    pub(super) custom_bank: i32,
}

impl HeadSample {
    fn from_default(default: Option<DefaultSample>) -> Self {
        let default = DefaultSample::or_default(default);
        Self {
            bank: default.bank,
            volume: default.volume,
            custom_bank: default.custom_bank,
        }
    }
}

/// 物件的自定义音效索引：物件自己声明了就用它，否则沿用所在 timing point 的 `sample_index`。
///
/// 与 osu! 的 `LegacySampleControlPoint.ApplyTo` 一致
///（`newCustomSampleBank: legacy.CustomSampleBank > 0 ? legacy.CustomSampleBank : CustomSampleBank`）。
pub(super) fn sample_custom_bank(custom_bank: i32, point: Option<&TimingPoint>) -> i32 {
    if custom_bank > 0 {
        custom_bank
    } else {
        point.map_or(0, |point| point.sample_index.max(0))
    }
}

/// timing point 的采样组为 0 时沿用 General.SampleSet，而不是强制回到 Normal。
pub(super) fn timing_sample_bank(beatmap: &Beatmap, point: &TimingPoint) -> SampleBank {
    if point.sample_set != 0 {
        return SampleBank::from_set_id(point.sample_set);
    }
    match beatmap.general.get("SampleSet").map(str::trim).unwrap_or("") {
        value if value.eq_ignore_ascii_case("soft") => SampleBank::Soft,
        value if value.eq_ignore_ascii_case("drum") => SampleBank::Drum,
        _ => SampleBank::Normal,
    }
}

/// 物件头部的（音效组, 音量, 自定义音效索引）：优先用物件自带的 `hitSample`，
/// 否则回退到 timing point。
pub(super) fn head_sample(samples: &[HitSample], beatmap: &Beatmap, time: i64) -> HeadSample {
    match samples.first() {
        Some(sample) => {
            let default = sample_point_at(beatmap, time);
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
            HeadSample {
                bank,
                volume,
                custom_bank: sample_custom_bank(sample.custom_bank, default),
            }
        }
        None => HeadSample::from_default(DefaultSample::at(beatmap, time)),
    }
}

/// 按 timing point 的默认参数推送一个节点的打击音（普通层 + 该节点的加成音）。
///
/// 滑条节点（重复箭头、滑条尾）与没有自带 `hitSample` 的物件都走这里：osu! 会对每个
/// 节点用**它自己时刻**的 `SampleControlPoint` 补齐音效组、音量与自定义索引，因此节点
/// 跨过音量或音效组变化时，声音与头部并不相同（谱面常用「在滑条尾插入低音量绿线」来
/// 压掉尾部音效，就是靠这一点生效的）。
pub(super) fn push_default_samples<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    beatmap: &Beatmap,
    hitsound: i32,
    time_ms: f64,
) {
    let default = DefaultSample::or_default(DefaultSample::at(beatmap, time_ms as i64));
    builder.push_named(
        default.bank,
        "hitnormal",
        default.custom_bank,
        default.volume,
        time_ms,
        0.0,
        false,
    );
    for addition in HitAddition::all_from_hitsound(hitsound) {
        builder.push_named(
            default.bank,
            addition.suffix(),
            default.custom_bank,
            default.volume,
            time_ms,
            0.0,
            false,
        );
    }
}

/// 追加物件自带的打击音；没有自带参数时按该时刻的 timing point 生成。
///
/// `time_ms` 同时是「解析默认参数的时刻」与「发声时刻」：osu! 对每个物件/节点都用它自己
/// 时刻的 `SampleControlPoint`（非 `IHasRepeats` 的物件用 `GetEndTime()`，滑条节点用节点时间）。
pub(super) fn push_declared_samples<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    samples: &[HitSample],
    hitsound: i32,
    beatmap: &Beatmap,
    time_ms: f64,
) {
    if samples.is_empty() {
        push_default_samples(builder, beatmap, hitsound, time_ms);
        return;
    }
    builder.push_samples(samples, beatmap, time_ms, 0.0);
}

/// 返回 `start_time` 之前最后一条 timing point 生效的 (beat_length, slider_velocity)。
pub(super) fn slider_timing(start_time: i64, beatmap: &Beatmap) -> (f64, f64) {
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

pub(super) fn timing_point_at(beatmap: &Beatmap, time: i64) -> Option<&TimingPoint> {
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
