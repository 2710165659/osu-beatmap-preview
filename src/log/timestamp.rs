//! 本地时间戳（毫秒精度，本地时区获取失败时回退 UTC）。

use time::format_description::FormatItem;
use time::macros::format_description;
use time::OffsetDateTime;

const LOCAL_FORMAT: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]");

pub fn now_local_millis() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    now.format(LOCAL_FORMAT)
        .unwrap_or_else(|_| "????-??-?? ??:??:??.???".to_string())
}
