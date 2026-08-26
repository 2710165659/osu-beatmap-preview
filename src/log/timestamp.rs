//! 本地时间戳（毫秒精度，本地时区获取失败时回退 UTC）。

use time::OffsetDateTime;

pub fn now_local_millis() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let description = time::format_description::parse_owned::<2>(
        crate::config::current()
            .logging
            .timestamp
            .LOCAL_FORMAT
            .as_str(),
    )
    .or_else(|_| {
        time::format_description::parse_owned::<2>(
            "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]",
        )
    });
    let Ok(description) = description else {
        return "????-??-?? ??:??:??.???".to_string();
    };
    now.format(&description)
        .unwrap_or_else(|_| "????-??-?? ??:??:??.???".to_string())
}
