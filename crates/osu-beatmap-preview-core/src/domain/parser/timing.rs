//! Timing point 与 break 时段解析。

use crate::domain::models::{BreakPeriod, TimingPoint};

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
        points.push(TimingPoint {
            time: parts[0].parse().ok()?,
            beat_length: parts[1].parse().ok()?,
            meter,
            uninherited,
            kiai_mode: effects & 1 != 0,
            omit_first_bar_line: effects & 8 != 0,
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
        let mut fields = line.splitn(3, ',');
        if fields.next().map(str::trim) != Some("0") {
            continue;
        }
        let Some(_) = fields.next() else {
            continue;
        };
        let Some(remainder) = fields.next().map(str::trim) else {
            continue;
        };
        let name = if let Some(quoted) = remainder.strip_prefix('"') {
            let Some((name, _)) = quoted.split_once('"') else {
                continue;
            };
            name.trim()
        } else {
            let Some(name) = remainder.split(',').next().map(str::trim) else {
                continue;
            };
            name
        };
        if !name.is_empty() {
            return Some(name.replace('\\', "/"));
        }
    }
    None
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
}
