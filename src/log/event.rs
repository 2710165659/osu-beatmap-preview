//! 进度事件：每行一个阶段事件，供 `tail -f` 实时查看。
//!
//! 行格式：`2026-08-01 15:04:05.123 pid=1234 bid=5242890 step=download-osu
//! status=done msg="..."`（msg 为 JSON 转义，保证单行）。

use crate::log::config::enabled;
use crate::log::context;
use crate::log::timestamp::now_local_millis;
use crate::log::writer::append_line;

/// 写一条进度事件。
///
/// `bid` 传 `None` 时使用进程上下文中的当前 bid（如音频线程）；`status`
/// 常用值：`start` / `done` / `info` / `error`。
pub fn event(step: &str, status: &str, bid: Option<&str>, msg: &str) {
    let Some(cfg) = enabled() else {
        return;
    };
    let bid = bid
        .map(str::to_string)
        .or_else(context::bid)
        .unwrap_or_else(|| "-".to_string());
    append_line(&cfg.progress_path, &fit_line(step, status, &bid, msg));
}

/// 组装一行，msg 过长时按字符截断直到整行不超上限。
fn fit_line(step: &str, status: &str, bid: &str, msg: &str) -> String {
    let mut msg = msg.to_string();
    loop {
        let line = build_line(step, status, bid, &msg);
        if line.len() <= crate::config::current().logging.writer.MAX_LINE_BYTES || msg.is_empty() {
            return line;
        }
        let cut = msg.char_indices().next_back().map(|(i, _)| i).unwrap_or(0);
        msg.truncate(cut);
    }
}

fn build_line(step: &str, status: &str, bid: &str, msg: &str) -> String {
    let json_msg = serde_json::to_string(msg).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        "{} pid={} bid={} step={} status={} msg={}",
        now_local_millis(),
        std::process::id(),
        bid,
        step,
        status,
        json_msg
    )
}
