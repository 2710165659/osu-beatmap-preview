/// 单条日志允许写入的最大字节数。
pub(crate) const MAX_LINE_BYTES: usize = 4096;

/// 本地日志时间戳格式。
pub(crate) const LOCAL_FORMAT: &[time::format_description::FormatItem<'static>] = time::macros::format_description!(
    "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"
);
