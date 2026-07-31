use crate::core::errors::{PreviewError, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::Instant;

const OSZ_MIRRORS: [&str; 4] = [
    "https://mirror.nekoha.moe/api/download/{}?noVideo=1",
    "https://txy1.sayobot.cn/beatmaps/download/novideo/{}",
    "https://osu.direct/api/d/{}",
    "https://catboy.best/d/{}",
];
const MIB_BYTES: u64 = 1024 * 1024;
const MAX_OSZ_BYTES: u64 = 50 * MIB_BYTES;
const MIRROR_ATTEMPTS: usize = 3;
const PARALLEL_PARTS: usize = 4;
const USER_AGENT: &str = "osu-beatmap-preview/1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContentRange {
    start: u64,
    end: u64,
    total: u64,
}

enum RangeProbe {
    Supported(u64),
    FullResponse(ureq::Response),
}

pub fn download_beatmapset_archive(
    set_id: u64,
    temp_dir: &Path,
    no_cache: bool,
) -> Result<PathBuf> {
    std::fs::create_dir_all(temp_dir)
        .map_err(|e| PreviewError::download(format!("failed to create osz cache dir: {e}")))?;
    let target_path = temp_dir.join(format!("{set_id}.osz"));
    if !no_cache && valid_osz(&target_path) {
        let size_mib = target_path
            .metadata()
            .map(|m| m.len() as f64 / MIB_BYTES as f64)
            .unwrap_or(0.0);
        crate::log::event(
            "download-osz",
            "done",
            None,
            &format!("set={set_id} cache hit ({size_mib:.1} MiB)"),
        );
        crate::log::record_cache(crate::log::CacheKind::Osz, "hit");
        return Ok(target_path);
    }

    let part_path = temp_dir.join(format!("{set_id}.osz.part"));
    let agent = build_agent();
    crate::log::event(
        "download-osz",
        "start",
        None,
        &format!("set={set_id} trying {} mirrors", OSZ_MIRRORS.len()),
    );
    let started = Instant::now();
    let result = try_osz_mirrors(set_id, |url, attempt| {
        remove_if_exists(&part_path);
        download_osz_once(&agent, url, &part_path)
            .map_err(|reason| format!("attempt {attempt}/{MIRROR_ATTEMPTS}: {reason}"))
    });
    if let Err(failures) = result {
        remove_if_exists(&part_path);
        crate::log::event(
            "download-osz",
            "error",
            None,
            &format!("set={set_id} {}", failures.join("; ")),
        );
        return Err(PreviewError::download(format!(
            "failed to download beatmapset {set_id} from all mirrors: {}",
            failures.join("; ")
        )));
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;

    if target_path.exists() {
        std::fs::remove_file(&target_path)
            .map_err(|e| PreviewError::download(format!("failed to replace osz cache: {e}")))?;
    }
    std::fs::rename(&part_path, &target_path)
        .map_err(|e| PreviewError::download(format!("failed to commit osz cache: {e}")))?;
    let size_mib = target_path
        .metadata()
        .map(|m| m.len() as f64 / MIB_BYTES as f64)
        .unwrap_or(0.0);
    crate::log::event(
        "download-osz",
        "done",
        None,
        &format!("set={set_id} downloaded {size_mib:.1} MiB in {ms:.0} ms"),
    );
    crate::log::record_cache(crate::log::CacheKind::Osz, "downloaded");
    Ok(target_path)
}

fn build_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(120))
        .timeout_write(Duration::from_secs(30))
        .build()
}

fn try_osz_mirrors(
    set_id: u64,
    mut fetch: impl FnMut(&str, usize) -> std::result::Result<(), String>,
) -> std::result::Result<(), Vec<String>> {
    let mut failures = Vec::new();
    for template in OSZ_MIRRORS {
        let url = template.replace("{}", &set_id.to_string());
        for attempt in 1..=MIRROR_ATTEMPTS {
            match fetch(&url, attempt) {
                Ok(()) => return Ok(()),
                Err(reason) => failures.push(format!("{url} {reason}")),
            }
        }
    }
    Err(failures)
}

fn download_osz_once(
    agent: &ureq::Agent,
    url: &str,
    part_path: &Path,
) -> std::result::Result<(), String> {
    match probe_range_support(agent, url) {
        Ok(RangeProbe::Supported(total)) => {
            let parallel_result = download_parallel(agent, url, part_path, total)
                .and_then(|_| validate_downloaded_archive(part_path));
            if parallel_result.is_ok() {
                return parallel_result;
            }

            let parallel_error = parallel_result.unwrap_err();
            remove_if_exists(part_path);
            download_single(agent, url, part_path)
                .and_then(|_| validate_downloaded_archive(part_path))
                .map_err(|single_error| {
                    format!(
                        "4-part parallel download failed ({parallel_error}); \
                         single-stream fallback failed ({single_error})"
                    )
                })
        }
        Ok(RangeProbe::FullResponse(response)) => download_response(response, part_path)
            .and_then(|_| validate_downloaded_archive(part_path)),
        Err(probe_error) => {
            remove_if_exists(part_path);
            download_single(agent, url, part_path)
                .and_then(|_| validate_downloaded_archive(part_path))
                .map_err(|single_error| {
                    format!(
                        "range probe failed ({probe_error}); \
                         single-stream fallback failed ({single_error})"
                    )
                })
        }
    }
}

fn probe_range_support(agent: &ureq::Agent, url: &str) -> std::result::Result<RangeProbe, String> {
    let response = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept-Encoding", "identity")
        .set("Range", "bytes=0-0")
        .call()
        .map_err(format_http_error)?;
    reject_html(&response)?;

    if response.status() != 206 {
        return Ok(RangeProbe::FullResponse(response));
    }

    let value = response
        .header("Content-Range")
        .ok_or_else(|| "206 response omitted Content-Range".to_string())?;
    let range = parse_content_range(value)?;
    if range.start != 0 || range.end != 0 {
        return Err(format!(
            "range probe returned unexpected Content-Range '{value}'"
        ));
    }
    validate_declared_size(range.total)?;

    let mut probe = Vec::with_capacity(2);
    response
        .into_reader()
        .take(2)
        .read_to_end(&mut probe)
        .map_err(|e| format!("failed to read range probe: {e}"))?;
    if probe.len() != 1 {
        return Err(format!(
            "range probe returned {} bytes instead of 1",
            probe.len()
        ));
    }
    Ok(RangeProbe::Supported(range.total))
}

fn download_parallel(
    agent: &ureq::Agent,
    url: &str,
    part_path: &Path,
    total: u64,
) -> std::result::Result<(), String> {
    let ranges = split_ranges(total, PARALLEL_PARTS);
    let file =
        File::create(part_path).map_err(|e| format!("failed to create temporary osz: {e}"))?;
    file.set_len(total)
        .map_err(|e| format!("failed to allocate temporary osz: {e}"))?;
    drop(file);

    let handles = ranges
        .into_iter()
        .map(|range| {
            let agent = agent.clone();
            let url = url.to_string();
            let part_path = part_path.to_path_buf();
            std::thread::spawn(move || download_range_part(&agent, &url, &part_path, range))
        })
        .collect::<Vec<_>>();

    let mut failures = Vec::new();
    for handle in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => failures.push(reason),
            Err(_) => failures.push("range download worker panicked".to_string()),
        }
    }
    if !failures.is_empty() {
        remove_if_exists(part_path);
        return Err(failures.join("; "));
    }

    let actual = part_path
        .metadata()
        .map_err(|e| format!("failed to inspect temporary osz: {e}"))?
        .len();
    if actual != total {
        remove_if_exists(part_path);
        return Err(format!(
            "parallel download produced {actual} bytes, expected {total}"
        ));
    }
    Ok(())
}

fn download_range_part(
    agent: &ureq::Agent,
    url: &str,
    part_path: &Path,
    range: ContentRange,
) -> std::result::Result<(), String> {
    let expected = range.end - range.start + 1;
    let range_header = format!("bytes={}-{}", range.start, range.end);
    let response = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept-Encoding", "identity")
        .set("Range", &range_header)
        .call()
        .map_err(format_http_error)?;
    reject_html(&response)?;
    if response.status() != 206 {
        return Err(format!(
            "{range_header} returned HTTP {}, expected 206",
            response.status()
        ));
    }

    let value = response
        .header("Content-Range")
        .ok_or_else(|| format!("{range_header} omitted Content-Range"))?;
    let actual_range = parse_content_range(value)?;
    if actual_range != range {
        return Err(format!(
            "{range_header} returned unexpected Content-Range '{value}'"
        ));
    }
    if let Some(length) = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
    {
        if length != expected {
            return Err(format!(
                "{range_header} declared {length} bytes, expected {expected}"
            ));
        }
    }

    let capacity = usize::try_from(expected)
        .map_err(|_| format!("{range_header} is too large for this platform"))?;
    let mut data = Vec::with_capacity(capacity);
    response
        .into_reader()
        .take(expected + 1)
        .read_to_end(&mut data)
        .map_err(|e| format!("failed to read {range_header}: {e}"))?;
    if data.len() as u64 != expected {
        return Err(format!(
            "{range_header} returned {} bytes, expected {expected}",
            data.len()
        ));
    }

    let mut file = OpenOptions::new()
        .write(true)
        .open(part_path)
        .map_err(|e| format!("failed to open temporary osz for {range_header}: {e}"))?;
    file.seek(SeekFrom::Start(range.start))
        .map_err(|e| format!("failed to seek temporary osz for {range_header}: {e}"))?;
    file.write_all(&data)
        .map_err(|e| format!("failed to write {range_header}: {e}"))?;
    Ok(())
}

fn download_single(
    agent: &ureq::Agent,
    url: &str,
    part_path: &Path,
) -> std::result::Result<(), String> {
    let response = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept-Encoding", "identity")
        .call()
        .map_err(format_http_error)?;
    download_response(response, part_path)
}

fn download_response(
    response: ureq::Response,
    part_path: &Path,
) -> std::result::Result<(), String> {
    reject_html(&response)?;
    if let Some(length) = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
    {
        validate_declared_size(length)?;
    }

    let mut output =
        File::create(part_path).map_err(|e| format!("failed to create temporary osz: {e}"))?;
    let copied = std::io::copy(
        &mut response.into_reader().take(MAX_OSZ_BYTES + 1),
        &mut output,
    )
    .map_err(|e| format!("failed to read response: {e}"))?;
    output
        .flush()
        .map_err(|e| format!("failed to flush temporary osz: {e}"))?;
    if copied == 0 {
        remove_if_exists(part_path);
        return Err("server returned an empty response".to_string());
    }
    if copied > MAX_OSZ_BYTES {
        remove_if_exists(part_path);
        return Err(format!(
            "OSZ is too large while reading the response: received more than \
             {MAX_OSZ_BYTES} bytes ({:.2} MiB); Content-Length was missing or understated",
            MAX_OSZ_BYTES as f64 / MIB_BYTES as f64,
        ));
    }
    Ok(())
}

fn validate_declared_size(length: u64) -> std::result::Result<(), String> {
    if length == 0 {
        return Err("server declared an empty response (Content-Length: 0 bytes)".to_string());
    }
    if length > MAX_OSZ_BYTES {
        return Err(format!(
            "OSZ is too large: server declared {length} bytes ({:.2} MiB), \
             exceeding the download limit of {MAX_OSZ_BYTES} bytes ({:.2} MiB)",
            length as f64 / MIB_BYTES as f64,
            MAX_OSZ_BYTES as f64 / MIB_BYTES as f64,
        ));
    }
    Ok(())
}

fn reject_html(response: &ureq::Response) -> std::result::Result<(), String> {
    let content_type = response
        .header("Content-Type")
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type.contains("text/html") {
        return Err("server returned HTML instead of an osz archive".to_string());
    }
    Ok(())
}

fn format_http_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, _) => format!("http {code}"),
        other => other.to_string(),
    }
}

fn parse_content_range(value: &str) -> std::result::Result<ContentRange, String> {
    let value = value.trim();
    let range_and_total = value
        .strip_prefix("bytes ")
        .ok_or_else(|| format!("invalid Content-Range '{value}'"))?;
    let (range, total) = range_and_total
        .split_once('/')
        .ok_or_else(|| format!("invalid Content-Range '{value}'"))?;
    let (start, end) = range
        .split_once('-')
        .ok_or_else(|| format!("invalid Content-Range '{value}'"))?;
    let start = start
        .parse::<u64>()
        .map_err(|_| format!("invalid Content-Range '{value}'"))?;
    let end = end
        .parse::<u64>()
        .map_err(|_| format!("invalid Content-Range '{value}'"))?;
    let total = total
        .parse::<u64>()
        .map_err(|_| format!("invalid Content-Range '{value}'"))?;
    if start > end || end >= total {
        return Err(format!("invalid Content-Range '{value}'"));
    }
    Ok(ContentRange { start, end, total })
}

fn split_ranges(total: u64, requested_parts: usize) -> Vec<ContentRange> {
    let part_count = requested_parts.max(1).min(total as usize);
    let chunk_size = total.div_ceil(part_count as u64);
    let mut ranges = Vec::with_capacity(part_count);
    let mut start = 0;
    while start < total {
        let end = (start + chunk_size - 1).min(total - 1);
        ranges.push(ContentRange { start, end, total });
        start = end + 1;
    }
    ranges
}

fn validate_downloaded_archive(path: &Path) -> std::result::Result<(), String> {
    if valid_osz(path) {
        Ok(())
    } else {
        Err("response is not a valid ZIP/OSZ archive".to_string())
    }
}

fn valid_osz(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_OSZ_BYTES {
        return false;
    }
    let Ok(file) = File::open(path) else {
        return false;
    };
    zip::ZipArchive::new(file).is_ok()
}

fn remove_if_exists(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Cursor};
    use std::net::TcpListener;
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::SimpleFileOptions;

    #[test]
    fn osz_mirror_order_is_stable() {
        assert_eq!(
            OSZ_MIRRORS[0],
            "https://mirror.nekoha.moe/api/download/{}?noVideo=1"
        );
        assert_eq!(
            OSZ_MIRRORS[1],
            "https://txy1.sayobot.cn/beatmaps/download/novideo/{}"
        );
        assert_eq!(OSZ_MIRRORS[2], "https://osu.direct/api/d/{}");
        assert_eq!(OSZ_MIRRORS[3], "https://catboy.best/d/{}");
    }

    #[test]
    fn retries_each_mirror_before_falling_back() {
        let mut calls = Vec::new();
        let result = try_osz_mirrors(42, |url, attempt| {
            calls.push((url.to_string(), attempt));
            if calls.len() == 7 {
                Ok(())
            } else {
                Err("failed".to_string())
            }
        });
        assert!(result.is_ok());
        assert_eq!(calls.len(), 7);
        assert!(calls[0].0.starts_with("https://mirror.nekoha.moe/"));
        assert!(calls[3].0.starts_with("https://txy1.sayobot.cn/"));
        assert!(calls[6].0.starts_with("https://osu.direct/"));
        assert_eq!(
            calls.iter().map(|call| call.1).collect::<Vec<_>>(),
            vec![1, 2, 3, 1, 2, 3, 1]
        );
    }

    #[test]
    fn parses_and_rejects_content_ranges() {
        assert_eq!(
            parse_content_range("bytes 10-19/100").unwrap(),
            ContentRange {
                start: 10,
                end: 19,
                total: 100,
            }
        );
        assert!(parse_content_range("bytes 20-10/100").is_err());
        assert!(parse_content_range("bytes 0-100/100").is_err());
        assert!(parse_content_range("items 0-1/2").is_err());
        assert!(parse_content_range("bytes */100").is_err());
    }

    #[test]
    fn splits_download_into_contiguous_ranges() {
        let ranges = split_ranges(10, 4);
        assert_eq!(
            ranges,
            vec![
                ContentRange {
                    start: 0,
                    end: 2,
                    total: 10,
                },
                ContentRange {
                    start: 3,
                    end: 5,
                    total: 10,
                },
                ContentRange {
                    start: 6,
                    end: 8,
                    total: 10,
                },
                ContentRange {
                    start: 9,
                    end: 9,
                    total: 10,
                },
            ]
        );
        assert_eq!(split_ranges(2, 4).len(), 2);
    }

    #[test]
    fn downloads_and_reassembles_four_http_ranges() {
        let archive = make_test_osz();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_archive = archive.clone();
        let server = std::thread::spawn(move || {
            let mut requested_ranges = Vec::new();
            for _ in 0..=PARALLEL_PARTS {
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
                        requested =
                            Some((start.parse::<u64>().unwrap(), end.parse::<u64>().unwrap()));
                    }
                }

                let (start, end) = requested.expect("request must contain a byte range");
                requested_ranges.push((start, end));
                let body = &server_archive[start as usize..=end as usize];
                write!(
                    stream,
                    "HTTP/1.1 206 Partial Content\r\n\
                     Content-Length: {}\r\n\
                     Content-Range: bytes {start}-{end}/{}\r\n\
                     Content-Type: application/octet-stream\r\n\
                     Connection: close\r\n\r\n",
                    body.len(),
                    server_archive.len(),
                )
                .unwrap();
                stream.write_all(body).unwrap();
            }
            requested_ranges
        });

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let test_dir = std::env::temp_dir().join(format!(
            "osu-beatmap-preview-osz-range-test-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&test_dir).unwrap();
        let part_path = test_dir.join("fixture.osz.part");
        let url = format!("http://{address}/fixture.osz");

        download_osz_once(&build_agent(), &url, &part_path).unwrap();
        let requested_ranges = server.join().unwrap();
        assert_eq!(std::fs::read(&part_path).unwrap(), archive);
        assert_eq!(requested_ranges[0], (0, 0));
        let mut downloaded = requested_ranges[1..].to_vec();
        downloaded.sort_unstable();
        assert_eq!(
            downloaded,
            split_ranges(archive.len() as u64, PARALLEL_PARTS)
                .into_iter()
                .map(|range| (range.start, range.end))
                .collect::<Vec<_>>()
        );
        std::fs::remove_dir_all(test_dir).unwrap();
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
}
