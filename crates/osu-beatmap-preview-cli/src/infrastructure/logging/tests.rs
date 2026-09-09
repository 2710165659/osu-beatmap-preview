#![cfg(test)]

//! 单元测试：时间戳、事件/汇总格式、转义、截断、禁用、上下文合并与并发追加。

use super::config::{init_for_tests, paths, PROGRESS_FILE, RENDER_FILE};
use super::*;
use std::path::{Path, PathBuf};

fn unique_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "osu-beatmap-preview-log-test-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn timestamp_is_local_millis_shape() {
    let _guard = test_guard();
    let ts = timestamp::now_local_millis();
    assert_eq!(ts.len(), 23, "unexpected timestamp: {ts}");
    assert_eq!(&ts[4..5], "-");
    assert_eq!(&ts[7..8], "-");
    assert_eq!(&ts[10..11], " ");
    assert_eq!(&ts[13..14], ":");
    assert_eq!(&ts[16..17], ":");
    assert_eq!(&ts[19..20], ".");
    assert!(ts[..4].chars().all(|c| c.is_ascii_digit()));
}

#[test]
fn event_and_summary_write_parseable_lines() {
    let _guard = test_guard();
    let dir = unique_dir("basic");
    reset_for_tests();
    init_for_tests(&dir);
    event("test", "info", Some("123"), "hello \"quoted\"");

    let rec = SummaryRecord {
        status: "success".to_string(),
        bid: "123".to_string(),
        duration_ms: 42.5,
        title: Some("quote \" and \n newline".to_string()),
        hit_object_count: Some(7),
        ..SummaryRecord::default()
    };
    write_summary(&rec);

    let progress = read(&dir.join(PROGRESS_FILE));
    let lines: Vec<&str> = progress.lines().collect();
    assert_eq!(lines.len(), 2, "session-start + one event");
    assert!(lines[0].contains("step=session-start"));
    assert!(lines[1].contains("bid=123"));
    assert!(lines[1].contains("msg=\"hello \\\"quoted\\\"\""));
    for line in &lines {
        assert!(line.len() <= super::constants::MAX_LINE_BYTES);
        assert!(!line.contains('\n'));
    }

    let render = read(&dir.join(RENDER_FILE));
    let lines: Vec<&str> = render.lines().collect();
    assert_eq!(lines.len(), 1);
    let value: serde_json::Value = serde_json::from_str(lines[0]).expect("valid NDJSON line");
    assert_eq!(value["bid"], "123");
    assert_eq!(value["status"], "success");
    assert_eq!(value["duration_ms"], 42.5);
    assert_eq!(value["title"], "quote \" and \n newline");
    assert_eq!(value["hit_object_count"], 7);
    assert_eq!(
        value["app_version"],
        crate::infrastructure::logging::APP_VERSION
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn long_message_is_truncated_to_line_limit() {
    let _guard = test_guard();
    let dir = unique_dir("truncate");
    reset_for_tests();
    init_for_tests(&dir);
    let long = "x".repeat(20_000);
    event("test", "info", Some("1"), &long);

    let progress = read(&dir.join(PROGRESS_FILE));
    let line = progress.lines().next_back().unwrap();
    assert!(line.len() <= super::constants::MAX_LINE_BYTES);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn disabled_without_init_writes_nothing() {
    let _guard = test_guard();
    let dir = unique_dir("disabled");
    reset_for_tests();
    assert!(paths().is_none());
    event("test", "info", Some("1"), "x");
    write_summary(&SummaryRecord {
        bid: "1".to_string(),
        ..SummaryRecord::default()
    });
    assert!(!dir.join(PROGRESS_FILE).exists());
    assert!(!dir.join(RENDER_FILE).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn summary_merges_context_cache_stages_and_video_stats() {
    let _guard = test_guard();
    let dir = unique_dir("context");
    reset_for_tests();
    init_for_tests(&dir);
    set_bid("777");
    record_cache(CacheKind::Osu, "downloaded");
    record_cache(CacheKind::Output, "hit");
    record_stage("download_osz_ms", 1234.56);
    record_stage_status("download_osz_cache", "hit");
    record_output_bytes(999);
    record_video_stats(VideoStats {
        backend: Some("NVENC".to_string()),
        resolution: Some("1280x720".to_string()),
        fps: Some(15),
        frame_count: Some(1200),
        video_ms: Some(111.23),
        encode_ms: Some(12.5),
        ..VideoStats::default()
    });
    write_summary(&SummaryRecord {
        bid: "777".to_string(),
        status: "cache-hit".to_string(),
        ..SummaryRecord::default()
    });

    let render = read(&dir.join(RENDER_FILE));
    let value: serde_json::Value = serde_json::from_str(render.lines().next().unwrap()).unwrap();
    assert_eq!(value["status"], "cache-hit");
    assert_eq!(value["cache"]["osu"], "downloaded");
    assert_eq!(value["cache"]["output"], "hit");
    assert_eq!(value["download_osz_ms"], 1234.6);
    assert_eq!(value["download_osz_cache"], "hit");
    assert_eq!(value["output_bytes"], 999);
    assert_eq!(value["backend"], "NVENC");
    assert_eq!(value["resolution"], "1280x720");
    assert_eq!(value["fps"], 15);
    assert_eq!(value["frame_count"], 1200);
    assert_eq!(value["video_ms"], 111.23);
    assert_eq!(value["encode_ms"], 12.5);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn concurrent_threads_write_complete_lines() {
    let _guard = test_guard();
    let dir = unique_dir("threads");
    reset_for_tests();
    init_for_tests(&dir);

    let handles: Vec<_> = (0..8)
        .map(|thread| {
            std::thread::spawn(move || {
                for i in 0..200 {
                    event(
                        "test",
                        "info",
                        Some("42"),
                        &format!("thread {thread} line {i}"),
                    );
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }

    let progress = read(&dir.join(PROGRESS_FILE));
    let lines: Vec<&str> = progress.lines().collect();
    assert_eq!(lines.len(), 1 + 8 * 200, "session-start + all events");
    for line in &lines {
        assert!(line.contains(" pid="), "missing pid: {line}");
        assert!(line.contains(" msg="), "missing msg: {line}");
        assert!(!line.is_empty());
        if line.contains("step=test") {
            assert!(line.contains(" bid=42"), "missing bid: {line}");
            assert!(line.contains(" status=info"), "missing status: {line}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
