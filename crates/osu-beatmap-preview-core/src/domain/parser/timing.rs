//! Timing point 与 break 时段解析。

use crate::domain::models::{BreakPeriod, TimingPoint, VideoEvent};

/// 将 `[TimingPoints]` 行解析为排序后的 `Vec<TimingPoint>`。
/// 区段为空时返回 `None`。
pub fn parse_timing_points(lines: &[&str]) -> Option<Vec<TimingPoint>> {
    let mut points: Vec<TimingPoint> = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 2 {
            continue;
        }
        let mut meter = if parts.len() > 2 && !parts[2].is_empty() {
            parts[2].parse::<i32>().ok()?
        } else {
            4
        };
        if meter <= 0 {
            meter = 4;
        }
        let uninherited = parts.len() < 7 || parts[6] == "1";
        let effects = if parts.len() > 7 && !parts[7].is_empty() {
            parts[7].parse::<i32>().ok()?
        } else {
            0
        };
        // 第 4～6 列依次是采样组、采样索引与采样音量；缺失时按 osu! 默认值处理。
        let sample_set = parts
            .get(3)
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(0);
        let sample_index = parts
            .get(4)
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(0)
            .max(0);
        let sample_volume = parts
            .get(5)
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(100);
        points.push(TimingPoint {
            time: parts[0].parse().ok()?,
            beat_length: parts[1].parse().ok()?,
            meter,
            uninherited,
            kiai_mode: effects & 1 != 0,
            omit_first_bar_line: effects & 8 != 0,
            sample_set,
            sample_index,
            sample_volume,
        });
    }
    // 稳定排序可保留相同时间红线/绿线在文件中的顺序。
    points.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
    if points.is_empty() {
        return None;
    }
    Some(points)
}

/// 从 `[Events]` 行解析 break 时段（类型 2 事件）。
pub fn parse_break_periods(lines: Option<&Vec<&str>>) -> Vec<BreakPeriod> {
    let Some(lines) = lines else {
        return Vec::new();
    };
    let mut breaks = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 3 || parts[0] != "2" {
            continue;
        }
        let (Ok(s), Ok(e)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) else {
            continue;
        };
        let (start_time, end_time) = (s as i64, e as i64);
        if end_time > start_time {
            breaks.push(BreakPeriod {
                start_time,
                end_time,
            });
        }
    }
    breaks
}

/// 从 `[Events]` 区段解析第一张谱面背景图文件名。
pub fn parse_background_filename(lines: Option<&Vec<&str>>) -> Option<String> {
    let lines = lines?;
    for line in lines {
        if let Some((_, name)) = parse_media_event(line, "0") {
            return Some(name);
        }
    }
    None
}

/// 从 `[Events]` 区段解析第一个背景视频事件（`Video,<start_ms>,"file"` 或旧式 `1,...`）。
///
/// 同一行的类型关键字与数字别名都接受（osu! 两种写法并存）；这里**不做扩展名
/// 白名单过滤**（osu! 会把 `Video,` 行里的非视频文件当背景图用，见
/// [`crate::domain::parser::parse_beatmap_bytes`] 的兼容分支），调用方按需筛选。
pub fn parse_video_event(lines: Option<&Vec<&str>>) -> Option<VideoEvent> {
    let lines = lines?;
    for line in lines {
        for event_type in ["Video", "1"] {
            if let Some((start_ms, filename)) = parse_media_event(line, event_type) {
                return Some(VideoEvent { filename, start_ms });
            }
        }
    }
    None
}

/// 解析一条 `[Events]` 媒体事件行，返回（起始时间毫秒、归一化文件名）。
///
/// 第三字段支持带引号（文件名可含逗号）与旧式不带引号两种写法，文件名里的
/// 反斜杠归一化为 `/`。类型不匹配或文件名为空时返回 `None`。
/// 起始时间允许写成小数，向零截断为毫秒；写错时按 0 处理（背景图事件不关心
/// 该字段，视频事件的 0 也是最常见的默认值），不让单个坏字段毁掉整行。
fn parse_media_event(line: &str, event_type: &str) -> Option<(i64, String)> {
    let mut fields = line.splitn(3, ',');
    if fields.next().map(str::trim) != Some(event_type) {
        return None;
    }
    let start = fields.next().map(str::trim)?;
    let remainder = fields.next().map(str::trim)?;
    let name = if let Some(quoted) = remainder.strip_prefix('"') {
        let (name, _) = quoted.split_once('"')?;
        name.trim()
    } else {
        remainder.split(',').next().map(str::trim)?
    };
    if name.is_empty() {
        return None;
    }
    let start_ms = start
        .parse::<i64>()
        .ok()
        .or_else(|| start.parse::<f64>().ok().map(|value| value as i64))
        .unwrap_or(0);
    Some((start_ms, name.replace('\\', "/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_effects_preserve_omit_first_bar_line() {
        let points = parse_timing_points(&["0,500,4,2,0,100,1,9"]).unwrap();
        assert!(points[0].kiai_mode);
        assert!(points[0].omit_first_bar_line);
    }

    #[test]
    fn background_filename_supports_quoted_commas_and_windows_separators() {
        let lines = vec!["2,1000,2000", "0,0,\"Backgrounds\\artist, title.jpg\",0,0"];
        assert_eq!(
            parse_background_filename(Some(&lines)).as_deref(),
            Some("Backgrounds/artist, title.jpg")
        );
    }

    #[test]
    fn background_filename_supports_unquoted_legacy_events() {
        let lines = vec!["0,0,bg.png,0,0"];
        assert_eq!(
            parse_background_filename(Some(&lines)).as_deref(),
            Some("bg.png")
        );
    }

    /// 视频事件支持关键字与数字别名、带引号文件名与起始时间偏移。
    #[test]
    fn video_event_parses_keyword_and_legacy_alias() {
        let lines = vec![
            "0,0,\"bg.jpg\",0,0",
            "Video,1500,\"intro video, final.mp4\"",
            "2,1000,2000",
        ];
        assert_eq!(
            parse_video_event(Some(&lines)),
            Some(VideoEvent {
                filename: "intro video, final.mp4".to_string(),
                start_ms: 1500,
            })
        );

        let legacy = vec!["1,-250,clip.webm"];
        assert_eq!(
            parse_video_event(Some(&legacy)),
            Some(VideoEvent {
                filename: "clip.webm".to_string(),
                start_ms: -250,
            })
        );
    }

    /// 起始时间写成小数时向零截断；写坏时按 0 处理而不是丢掉整个视频事件。
    #[test]
    fn video_event_tolerates_fractional_or_invalid_start_time() {
        let fractional = vec!["Video,12.7,\"video.mp4\""];
        assert_eq!(
            parse_video_event(Some(&fractional)),
            Some(VideoEvent {
                filename: "video.mp4".to_string(),
                start_ms: 12,
            })
        );

        let broken = vec!["Video,abc,\"video.mp4\""];
        assert_eq!(
            parse_video_event(Some(&broken)),
            Some(VideoEvent {
                filename: "video.mp4".to_string(),
                start_ms: 0,
            })
        );
    }

    /// 背景图与视频互不串台：背景行不会被当成视频，反之亦然。
    #[test]
    fn media_events_do_not_cross_match() {
        let lines = vec!["0,0,\"bg.jpg\",0,0"];
        assert_eq!(parse_video_event(Some(&lines)), None);

        let video_only = vec!["Video,0,\"video.mp4\""];
        assert_eq!(parse_background_filename(Some(&video_only)), None);

        // 空文件名的事件一律跳过。
        let empty = vec!["Video,0,\"\",0,0"];
        assert_eq!(parse_video_event(Some(&empty)), None);
    }
}
