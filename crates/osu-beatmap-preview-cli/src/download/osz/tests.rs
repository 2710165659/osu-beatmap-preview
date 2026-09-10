#![cfg(test)]
use super::*;
use std::io::{BufRead, BufReader, Cursor};
use std::net::TcpListener;
use std::time::Duration;
use zip::write::SimpleFileOptions;

#[test]
fn candidate_order_skips_missing_preferred_ip() {
    let names = build_candidates(None)
        .into_iter()
        .map(|candidate| candidate.source.name())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["sayobot", "osu.direct-dns", "nekoha", "catboy"]);
}

#[test]
fn candidate_order_places_preferred_ip_before_dns() {
    let names = build_candidates(Some(Ipv4Addr::new(192, 0, 2, 1)))
        .into_iter()
        .map(|candidate| candidate.source.name())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "sayobot",
            "osu.direct-preferred-ip",
            "osu.direct-dns",
            "nekoha",
            "catboy"
        ]
    );
}

#[test]
fn osz_log_message_includes_request_bid_and_set_id() {
    let context = OszLogContext::new("738063", 12345);
    assert_eq!(
        context.message("cache hit (1.0 MiB)"),
        "bid=738063 set=12345 cache hit (1.0 MiB)"
    );
}

#[test]
fn newly_cached_preferred_ip_is_inserted_before_dns() {
    let root = std::env::temp_dir().join(format!(
        "osu-preview-dynamic-cf-test-{}",
        std::process::id()
    ));
    let osz_cache = root.join("osz-download-cache");
    std::fs::create_dir_all(&osz_cache).unwrap();
    let tested_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        root.join("osu-direct-preferred-ip.json"),
        serde_json::json!({ "ip": "104.16.1.1", "tested_at": tested_at }).to_string(),
    )
    .unwrap();
    let mut candidates = build_candidates(None);
    maybe_insert_preferred_candidate(&mut candidates, 1, &osz_cache);
    assert!(matches!(
        candidates[1].source,
        MirrorSource::OsuDirectPreferred(ip) if ip == Ipv4Addr::new(104, 16, 1, 1)
    ));
    assert_eq!(candidates[2].source, MirrorSource::OsuDirectDns);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn attempt_paths_are_process_scoped() {
    let path = attempt_path(Path::new("cache"), 42, 3);
    let name = path.file_name().unwrap().to_string_lossy();
    assert!(name.contains(&std::process::id().to_string()));
    assert!(name.ends_with("attempt-3.osz.part"));
}

#[test]
fn no_first_byte_triggers_after_three_seconds() {
    let started = Instant::now() - crate::download::constants::NO_FIRST_BYTE_TIMEOUT;
    let progress = AttemptProgress {
        started,
        bytes: AtomicU64::new(0),
        first_byte_ms: AtomicU64::new(0),
    };
    let mut monitor = AttemptMonitor::new();
    assert_eq!(
        monitor.fallback_reason(Instant::now(), started, &progress),
        Some("no-first-byte")
    );
}

#[test]
fn low_speed_window_triggers_fallback() {
    let now = Instant::now();
    let started = now - Duration::from_secs(6);
    let progress = AttemptProgress {
        started,
        bytes: AtomicU64::new(32 * 1024),
        first_byte_ms: AtomicU64::new(1),
    };
    let mut monitor = AttemptMonitor::new();
    monitor
        .samples
        .push_back((now - crate::download::constants::LOW_SPEED_WINDOW, 0));
    assert_eq!(
        monitor.fallback_reason(now, started, &progress),
        Some("low-speed")
    );
}

#[test]
fn splits_and_validates_ranges() {
    assert_eq!(split_ranges(10, 4).len(), 4);
    assert_eq!(
        parse_content_range("bytes 10-19/100").unwrap(),
        ContentRange {
            start: 10,
            end: 19,
            total: 100
        }
    );
    assert!(parse_content_range("bytes 20-10/100").is_err());
}

#[test]
fn downloads_and_reassembles_four_http_ranges() {
    let archive = make_test_osz();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server_archive = archive.clone();
    let server = std::thread::spawn(move || {
        let mut requested_ranges = Vec::new();
        for _ in 0..=crate::config::current().download.osz.PARALLEL_PARTS {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut requested = None;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(value) = line
                    .trim()
                    .strip_prefix("Range: bytes=")
                    .or_else(|| line.trim().strip_prefix("range: bytes="))
                {
                    let (start, end) = value.split_once('-').unwrap();
                    requested = Some((start.parse::<u64>().unwrap(), end.parse::<u64>().unwrap()));
                }
            }
            let (start, end) = requested.expect("request must contain a byte range");
            requested_ranges.push((start, end));
            let body = &server_archive[start as usize..=end as usize];
            write!(
                    stream,
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    body.len(),
                    server_archive.len()
                )
                .unwrap();
            stream.write_all(body).unwrap();
        }
        requested_ranges
    });
    let dir = std::env::temp_dir().join(format!("osu-preview-osz-test-{}", std::process::id()));
    let path = dir.join("fixture.osz.part");
    std::fs::create_dir_all(&dir).unwrap();
    let context = DownloadContext {
        cancel: Arc::new(AtomicBool::new(false)),
        progress: Arc::new(AttemptProgress::new()),
    };
    let agent = ureq::AgentBuilder::new().build();
    download_osz_once(
        &agent,
        &format!("http://{address}/fixture.osz"),
        &path,
        &context,
    )
    .unwrap();
    let requested_ranges = server.join().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), archive);
    assert_eq!(requested_ranges[0], (0, 0));
    std::fs::remove_dir_all(dir).unwrap();
}

fn make_test_osz() -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut archive = zip::ZipWriter::new(cursor);
    archive
        .start_file("audio.mp3", SimpleFileOptions::default())
        .unwrap();
    archive.write_all(&vec![0x5a; 8 * 1024]).unwrap();
    archive.finish().unwrap().into_inner()
}
