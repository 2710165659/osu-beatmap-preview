//! Utility helpers: build-time parsing, path normalization, etc.

use std::time::SystemTime;

const BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");

/// Return the program build time as a `SystemTime`, parsed from the ISO 8601
/// build timestamp injected by `vergen` / `build.rs`.
///
/// If parsing fails (should not happen in practice), falls back to
/// `SystemTime::UNIX_EPOCH` so every cached file appears newer.
pub fn build_time() -> SystemTime {
    // Typical vergen output: "2025-01-15T10:30:00.123Z" or "2025-01-15T10:30:00Z"
    // Try a few common ISO 8601 formats with millisecond / without.
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

/// Minimal ISO-8601 parser using only std (no `chrono` dependency).
/// Supports the subset vergen emits: `YYYY-MM-DDTHH:MM:SS[.fff]Z`.
fn chrono_like_parse(s: &str, _fmt: &str) -> Option<SystemTime> {
    // Strip trailing 'Z'
    let s = s.strip_suffix('Z').unwrap_or(s);

    // Split date and time
    let (date, time) = s.split_once('T')?;

    // Parse date: YYYY-MM-DD
    let mut date_parts = date.split('-');
    let year: i32 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;

    if month < 1 || month > 12 || day < 1 || day > 31 {
        return None;
    }

    // Parse time: HH:MM:SS or HH:MM:SS.fff
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
            // Take up to 3 digits, right-pad with zeros
            let frac = &frac[..frac.len().min(3)];
            let padded = format!("{:0<3}", frac);
            padded.parse().ok()?
        }
        None => 0,
    };

    // Compute days since UNIX_EPOCH for the given date
    let days = days_from_civil(year, month, day)?;
    let secs = days as u64 * 86400 + hour as u64 * 3600 + min as u64 * 60 + sec as u64;
    let nanos = millis * 1_000_000;

    Some(SystemTime::UNIX_EPOCH + std::time::Duration::new(secs, nanos))
}

/// Days since 1970-01-01 (civil → Unix epoch day count).
/// Uses the proleptic Gregorian calendar algorithm.
fn days_from_civil(y: i32, m: u32, d: u32) -> Option<i64> {
    if m < 1 || m > 12 || d < 1 || d > 31 {
        return None;
    }
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;

    // Shift year so that March is the first month (algorithm from Howard Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = y - era * 400; // year of era [0, 399]
    let doy = (153 * (if m <= 2 { m + 9 } else { m - 3 }) + 2) / 5 + d - 1; // day of year [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // day of era [0, 146096]
    let days = era * 146097 + doe - 719468; // days since 1970-01-01

    Some(days)
}
