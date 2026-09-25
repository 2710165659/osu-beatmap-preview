//! PNG 区间段选窗：Taiko/Catch/Mania 静态图按 `--time-points` + `--duration-time`
//! 截取谱面片段时的窗口求解。
//!
//! 窗口规则与 MP4（[`crate::media::video::resolve_video_time_range`]）保持一致：
//! 请求区间超过谱面尾部时整体前移以保留请求时长；请求时长不小于整谱时长时
//! 输出完整谱面，不填充空白。

use osu_beatmap_preview_core::processing::parse::round_half_even;
use osu_beatmap_preview_core::support::error::{PreviewError, Result};

/// PNG 区间段请求：起点是谱面绝对时间（毫秒），时长来自 `--duration-time`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PngSegment {
    pub start_ms: i64,
    pub duration_ms: i64,
}

impl PngSegment {
    /// 由绝对起点（毫秒）与 `--duration-time`（秒）构造区间段。
    ///
    /// 时长按毫秒取整后至少为 1ms，并限制在 i64 可表示范围内，
    /// 防止极端参数在窗口运算中溢出。
    pub(crate) fn new(start_ms: i64, duration_seconds: f64) -> Result<Self> {
        let duration_f64 = duration_seconds * 1000.0;
        if !duration_f64.is_finite() || !(0.0..9_223_372_036_854_775_808.0).contains(&duration_f64) {
            return Err(PreviewError::new(
                "duration time is outside the supported range",
            ));
        }
        let duration_ms = round_half_even(duration_f64);
        if duration_ms < 1 {
            return Err(PreviewError::new("duration time is too small to render"));
        }
        Ok(Self {
            start_ms,
            duration_ms,
        })
    }
}

/// PNG 渲染窗口（谱面绝对时间，毫秒）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PngWindow {
    pub start_ms: i64,
    pub end_ms: i64,
}

impl PngWindow {
    pub(crate) fn duration_ms(self) -> i64 {
        self.end_ms - self.start_ms
    }
}

/// 求解 PNG 渲染窗口。
///
/// 无区间段时返回整谱范围 `[full_start_ms, full_end_ms]`（即当前整谱渲染覆盖的范围）。
/// 有区间段时：
/// - 请求时长不小于整谱时长：输出完整谱面，不填充空白；
/// - 否则把整个区间放进谱面范围内并保留请求时长——尾部超出谱面时整体前移，
///   起点早于谱面开头时整体后移。静态图没有 MP4 前置静音的语义，
///   因此窗口不允许落在谱面范围之外。
pub(crate) fn resolve_png_window(
    full_start_ms: i64,
    full_end_ms: i64,
    segment: Option<PngSegment>,
) -> Result<PngWindow> {
    let full = PngWindow {
        start_ms: full_start_ms,
        end_ms: full_end_ms.max(full_start_ms),
    };
    let Some(segment) = segment else {
        return Ok(full);
    };
    if segment.duration_ms < 1 {
        return Err(PreviewError::new("duration time must be positive"));
    }
    // 整谱装不下请求时长：直接输出完整谱面（与 MP4 的短谱面行为一致）。
    if full.duration_ms() <= segment.duration_ms {
        return Ok(full);
    }
    // 保留请求时长的前提下把窗口夹进谱面范围：clamp 已同时覆盖
    // 「尾部超出整体前移」与「起点过早整体后移」两个方向。
    let latest_start = full.end_ms - segment.duration_ms;
    let start_ms = segment.start_ms.clamp(full.start_ms, latest_start);
    Ok(PngWindow {
        start_ms,
        end_ms: start_ms + segment.duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_segment_the_full_chart_window_is_returned() {
        let window = resolve_png_window(1_000, 61_000, None).unwrap();
        assert_eq!(
            window,
            PngWindow {
                start_ms: 1_000,
                end_ms: 61_000
            }
        );
    }

    #[test]
    fn requested_window_is_kept_inside_the_chart() {
        let segment = PngSegment::new(30_000, 10.0).unwrap();
        let window = resolve_png_window(1_000, 61_000, Some(segment)).unwrap();
        assert_eq!(window, PngWindow {
            start_ms: 30_000,
            end_ms: 40_000
        });
    }

    #[test]
    fn tail_overflow_shifts_the_window_earlier_to_keep_duration() {
        // 谱面 60s，请求从 55s 起 10s：窗口整体前移到 [50s, 60s]。
        let segment = PngSegment::new(55_000, 10.0).unwrap();
        let window = resolve_png_window(0, 60_000, Some(segment)).unwrap();
        assert_eq!(window, PngWindow {
            start_ms: 50_000,
            end_ms: 60_000
        });
    }

    #[test]
    fn early_start_shifts_the_window_later_to_keep_duration() {
        let segment = PngSegment::new(-30_000, 10.0).unwrap();
        let window = resolve_png_window(5_000, 60_000, Some(segment)).unwrap();
        assert_eq!(window, PngWindow {
            start_ms: 5_000,
            end_ms: 15_000
        });
    }

    #[test]
    fn duration_not_shorter_than_chart_renders_the_whole_chart() {
        let segment = PngSegment::new(55_000, 600.0).unwrap();
        let window = resolve_png_window(1_000, 61_000, Some(segment)).unwrap();
        assert_eq!(window, PngWindow {
            start_ms: 1_000,
            end_ms: 61_000
        });
    }

    #[test]
    fn segment_duration_is_converted_to_milliseconds() {
        let segment = PngSegment::new(0, 2.5).unwrap();
        assert_eq!(segment.duration_ms, 2_500);
        assert!(PngSegment::new(0, 0.0).is_err());
        assert!(PngSegment::new(0, f64::INFINITY).is_err());
        assert!(PngSegment::new(0, 1e18).is_err());
    }
}
