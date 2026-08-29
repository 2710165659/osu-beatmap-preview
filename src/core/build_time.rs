//! 构建时间解析：将 `vergen` 注入的 ISO 8601 时间戳解析为
//! `SystemTime`，用于缓存过期检查。

use std::time::SystemTime;

const BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");

/// 返回程序构建时间。时间来自 `vergen` / `build.rs` 注入的 ISO 8601 时间戳。
///
/// 解析失败时（实际不应发生）回退到 `SystemTime::UNIX_EPOCH`，
/// 使所有缓存文件都被视为更新。
pub fn build_time() -> SystemTime {
    // vergen 的典型输出为 "2025-01-15T10:30:00.123Z" 或 "2025-01-15T10:30:00Z"。
    // 依次尝试带毫秒和不带毫秒的常见 ISO 8601 格式。
    for fmt in &[
        "%Y-%m-%dT%H:%M:%S%.3fZ",
        "%Y-%m-%dT%H:%M:%SZ",
        "%Y-%m-%dT%H:%M:%S%.fZ",
    ] {
        if let Some(dt) = chrono_like_parse(BUILD_TIMESTAMP, fmt) {
            return dt;
        }
    }
    eprintln!(
        "warning: failed to parse build timestamp '{}', falling back to UNIX_EPOCH",
        BUILD_TIMESTAMP
    );
    SystemTime::UNIX_EPOCH
}

/// 仅使用标准库的最小 ISO-8601 解析器（不引入 `chrono` 依赖）。
/// 支持 vergen 输出的子集：`YYYY-MM-DDTHH:MM:SS[.fff]Z`。
fn chrono_like_parse(s: &str, _fmt: &str) -> Option<SystemTime> {
    // 去掉末尾的 'Z'。
    let s = s.strip_suffix('Z').unwrap_or(s);

    // 分离日期和时间。
    let (date, time) = s.split_once('T')?;

    // 解析日期：YYYY-MM-DD。
    let mut date_parts = date.split('-');
    let year: i32 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    // 解析时间：HH:MM:SS 或 HH:MM:SS.fff。
    let (time_core, ms_str) = if let Some((core, frac)) = time.split_once('.') {
        (core, Some(frac))
    } else {
        (time, None)
    };

    let mut time_parts = time_core.split(':');
    let hour: u32 = time_parts.next()?.parse().ok()?;
    let min: u32 = time_parts.next()?.parse().ok()?;
    let sec: u32 = time_parts.next()?.parse().ok()?;

    if hour > 23 || min > 59 || sec > 59 {
        return None;
    }

    let millis: u32 = match ms_str {
        Some(frac) => {
            // 取最多 3 位数字，不足部分在右侧补零。
            let frac = &frac[..frac.len().min(3)];
            let padded = format!("{:0<3}", frac);
            padded.parse().ok()?
        }
        None => 0,
    };

    // 计算给定日期距离 UNIX_EPOCH 的天数。
    let days = days_from_civil(year, month, day)?;
    let secs = days as u64 * 86400 + hour as u64 * 3600 + min as u64 * 60 + sec as u64;
    let nanos = millis * 1_000_000;

    Some(SystemTime::UNIX_EPOCH + std::time::Duration::new(secs, nanos))
}

/// 计算日期距离 1970-01-01 的天数（公历日期转 Unix 纪元日数）。
/// 使用扩展格里高利历算法。
fn days_from_civil(y: i32, m: u32, d: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;

    // 将年份平移，使三月成为第一月（Howard Hinnant 算法）。
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = y - era * 400; // year of era [0, 399]
    let doy = (153 * (if m <= 2 { m + 9 } else { m - 3 }) + 2) / 5 + d - 1; // day of year [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // day of era [0, 146096]
    let days = era * 146097 + doe - 719468; // days since 1970-01-01

    Some(days)
}
