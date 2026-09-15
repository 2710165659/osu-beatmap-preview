//! 各模式共用的取样与 timing point 辅助。

use crate::domain::models::{Beatmap, HitAddition, HitSample, SampleBank, TimingPoint};

use super::sample::SampleResolver;
use super::timeline::TimelineBuilder;

/// timing point 提供的默认打击音参数；物件没有自带 `hitSample` 时使用。
///
/// 用值类型返回而不是 `Vec<HitSample>`：绝大多数物件都不带 `hitSample`，
/// 每个物件都分配一个 `Vec` 在大谱面上是纯浪费。
#[derive(Debug, Clone, Copy)]
pub(super) struct DefaultSample {
    pub(super) bank: SampleBank,
    pub(super) volume: i32,
}

impl DefaultSample {
    pub(super) fn at(beatmap: &Beatmap, time: i64) -> Option<Self> {
        timing_point_at(beatmap, time).map(|point| Self {
            bank: timing_sample_bank(beatmap, point),
            volume: point.sample_volume,
        })
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

/// 物件头部的（音效组, 音量）：优先用物件自带的 `hitSample`，否则回退到 timing point。
pub(super) fn head_sample(samples: &[HitSample], beatmap: &Beatmap, time: i64) -> (SampleBank, i32) {
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
pub(super) fn push_declared_samples<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    samples: &[HitSample],
    hitsound: i32,
    beatmap: &Beatmap,
    time: i64,
) {
    push_declared_samples_at(builder, samples, hitsound, beatmap, time, time as f64);
}

/// 与 [`push_declared_samples`] 相同，但把「解析默认参数的时刻」与「发声时刻」分开：
/// 转盘的样本参数取自转盘起始处，发声却在结束处（判定成立时）。
pub(super) fn push_declared_samples_at<R: SampleResolver>(
    builder: &mut TimelineBuilder<R>,
    samples: &[HitSample],
    hitsound: i32,
    beatmap: &Beatmap,
    lookup_time: i64,
    time_ms: f64,
) {
    if samples.is_empty() {
        if let Some(default) = DefaultSample::at(beatmap, lookup_time) {
            builder.push_named(default.bank, "hitnormal", default.volume, time_ms, 0.0, false);
            for addition in HitAddition::all_from_hitsound(hitsound) {
                builder.push_named(
                    default.bank,
                    addition.suffix(),
                    default.volume,
                    time_ms,
                    0.0,
                    false,
                );
            }
        }
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
