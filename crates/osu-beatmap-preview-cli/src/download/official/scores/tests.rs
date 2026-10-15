#![cfg(test)]
//! 使用本地 HTTP 服务验证官方 API 契约，不读凭据、不访问外部网络。
use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

fn mock(replies: Vec<(&'static str, serde_json::Value)>) -> (String, thread::JoinHandle<()>) {
    mock_bytes(
        replies
            .into_iter()
            .map(|(path, value)| (path.to_string(), value.to_string().into_bytes()))
            .collect(),
    )
}

fn mock_bytes(replies: Vec<(String, Vec<u8>)>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        for (expected, body) in replies {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut b = [0];
                stream.read_exact(&mut b).unwrap();
                request.push(b[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).unwrap();
            assert!(
                request.starts_with(&format!("GET /{expected} HTTP/1.1")),
                "{request}"
            );
            assert!(request
                .to_ascii_lowercase()
                .contains("x-api-version: 20220705"));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    (base, handle)
}

fn score(id: u64, date: &str) -> serde_json::Value {
    serde_json::json!({"id":id,"user_id":7,"beatmap_id":2,"ruleset_id":0,"has_replay":true,"ended_at":date,
        "beatmap":{"id":2,"beatmapset_id":3,"checksum":"a".repeat(32)}})
}

#[test]
fn best_resolves_numeric_username_as_name_and_checks_identity() {
    let (base, handle) = mock(vec![
        ("users/@%31%32%33/osu", serde_json::json!({"id":7})),
        (
            "beatmaps/2/scores/users/7?mode=osu&legacy_only=0",
            serde_json::json!({"score":score(99,"2026-01-01T00:00:00Z")}),
        ),
    ]);
    let deadline = RequestDeadline::new(Instant::now(), "test", Duration::from_secs(5));
    let client = Client {
        agent: ureq::agent(),
        token: "test".into(),
        deadline: &deadline,
        base,
    };
    let result = select(
        &client,
        &ReplaySource::Player {
            user: ReplayUser::Name("123".into()),
            selection: ScoreSelection::Best,
        },
        "2",
    )
    .unwrap();
    assert_eq!(result.id, 99);
    validate(&result).unwrap();
    handle.join().unwrap();
}

#[test]
fn recent_uses_absolute_timestamps_and_does_not_skip_unavailable_replay() {
    let mut newest = score(101, "2026-01-01T01:00:00+01:00");
    newest["has_replay"] = false.into();
    let (base, handle) = mock(vec![(
        "users/7/scores/recent?mode=osu&include_fails=1&legacy_only=0&limit=100&offset=0",
        serde_json::json!([score(100, "2026-01-01T00:30:00+02:00"), newest]),
    )]);
    let deadline = RequestDeadline::new(Instant::now(), "test", Duration::from_secs(5));
    let client = Client {
        agent: ureq::agent(),
        token: "test".into(),
        deadline: &deadline,
        base,
    };
    let result = recent(&client, 7, 2).unwrap();
    assert_eq!(result.id, 101);
    assert!(validate(&result)
        .unwrap_err()
        .to_string()
        .contains("没有可下载回放"));
    handle.join().unwrap();
}

#[test]
fn recent_missing_map_is_an_error_without_best_fallback() {
    let (base, handle) = mock(vec![(
        "users/7/scores/recent?mode=osu&include_fails=1&legacy_only=0&limit=100&offset=0",
        serde_json::json!([]),
    )]);
    let deadline = RequestDeadline::new(Instant::now(), "test", Duration::from_secs(5));
    let client = Client {
        agent: ureq::agent(),
        token: "test".into(),
        deadline: &deadline,
        base,
    };
    assert!(recent(&client, 7, 2)
        .unwrap_err()
        .to_string()
        .contains("不会回退"));
    handle.join().unwrap();
}

#[test]
fn replay_download_uses_bound_cache_and_no_cache_downloads_again() {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../osu-beatmap-preview-web/test/fixtures/replay-standard.osr"
    ));
    let mut record = score(10, "2026-01-01T00:00:00Z");
    record["beatmap"]["checksum"] = String::from_utf8(bytes[7..39].to_vec()).unwrap().into();
    let json = record.to_string().into_bytes();
    let (base, handle) = mock_bytes(vec![
        ("scores/10".into(), json.clone()),
        ("scores/10/download".into(), bytes.to_vec()),
        ("scores/10".into(), json.clone()),
        ("scores/10".into(), json),
        ("scores/10/download".into(), bytes.to_vec()),
    ]);
    let deadline = RequestDeadline::new(Instant::now(), "test", Duration::from_secs(5));
    let client = Client {
        agent: ureq::agent(),
        token: "test".into(),
        deadline: &deadline,
        base,
    };
    let cache = std::env::temp_dir().join(format!(
        "osu-score-cache-{}-{}",
        std::process::id(),
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    let source = ReplaySource::Score("10".into());
    let first = download_with(&client, &source, "", &cache, false).unwrap();
    assert_eq!(std::fs::read(&first.path).unwrap(), bytes);
    assert_eq!(
        download_with(&client, &source, "", &cache, false)
            .unwrap()
            .path,
        first.path
    );
    assert_eq!(
        download_with(&client, &source, "", &cache, true)
            .unwrap()
            .path,
        first.path
    );
    handle.join().unwrap();
    std::fs::remove_dir_all(cache).unwrap();
}

#[test]
fn replay_binding_rejects_broken_data_and_different_checksum() {
    let map = Lookup {
        id: 2,
        beatmapset_id: 3,
        checksum: "a".repeat(32),
    };
    assert!(check_replay(b"not an osr", &map).is_err());
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../osu-beatmap-preview-web/test/fixtures/replay-standard.osr"
    ));
    assert!(check_replay(bytes, &map)
        .unwrap_err()
        .to_string()
        .contains("BeatmapMismatch"));
}
