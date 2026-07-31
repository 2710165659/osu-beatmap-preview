//! 多进程并发日志集成测试：通过测试钩子启动多个真实二进制实例，
//! 并发向同一日志目录写入，验证行完整性、行数与 NDJSON 可解析性。

use std::path::PathBuf;
use std::process::{Command, Stdio};

const EXE: &str = env!("CARGO_BIN_EXE_osu-beatmap-preview");
const LINES_PER_PROCESS: usize = 25;

#[test]
fn concurrent_processes_append_complete_lines() {
    let dir = unique_dir("concurrent");
    let children: Vec<_> = (0..4)
        .map(|_| {
            Command::new(EXE)
                .env("OSU_PREVIEW_LOG_TEST", LINES_PER_PROCESS.to_string())
                .env("OSU_PREVIEW_LOG_DIR", &dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn binary")
        })
        .collect();
    for mut child in children {
        assert!(child.wait().expect("wait child").success(), "child failed");
    }

    let progress = std::fs::read_to_string(dir.join("progress.log")).expect("progress.log");
    let render = std::fs::read_to_string(dir.join("render.log")).expect("render.log");

    // 4 进程 × (1 session-start + N test 事件)，行数精确且每行完整。
    let progress_lines: Vec<&str> = progress.lines().collect();
    assert_eq!(
        progress_lines.len(),
        4 * (1 + LINES_PER_PROCESS),
        "unexpected progress line count"
    );
    for line in &progress_lines {
        assert!(line.starts_with("20"), "line missing timestamp: {line}");
        assert!(line.contains(" pid="), "line missing pid: {line}");
        assert!(line.contains(" msg="), "line missing msg: {line}");
        if line.contains("step=test") {
            assert!(line.contains(" bid=test-bid"), "line missing bid: {line}");
            assert!(line.contains(" status=info"), "line missing status: {line}");
        }
    }

    let render_lines: Vec<&str> = render.lines().collect();
    assert_eq!(render_lines.len(), 4, "unexpected render line count");
    for line in &render_lines {
        let value: serde_json::Value = serde_json::from_str(line).expect("valid NDJSON line");
        assert_eq!(value["bid"], "test-bid");
        assert_eq!(value["status"], "success");
        assert_eq!(value["fmt"], "png");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

fn unique_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "osu-beatmap-preview-log-test-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}
