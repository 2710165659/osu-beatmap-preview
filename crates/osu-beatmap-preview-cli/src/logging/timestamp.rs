//! 本地时间戳（毫秒精度，本地时区获取失败时回退 UTC）。

use time::OffsetDateTime;

use super::constants::LOCAL_FORMAT;

pub fn now_local_millis() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    now.format(LOCAL_FORMAT)
        .unwrap_or_else(|_| "????-??-?? ??:??:??.???".to_string())
}
