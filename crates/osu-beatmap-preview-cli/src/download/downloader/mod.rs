//! 通用 HTTP 下载能力：分段下载、流式回退、取消和响应校验。

use osu_beatmap_preview_core::support::timeout::RequestDeadline;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub(crate) mod constants;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContentRange {
    pub(crate) start: u64,
    pub(crate) end: u64,
    pub(crate) total: u64,
}

pub(crate) struct AttemptProgress {
    pub(crate) started: Instant,
    pub(crate) bytes: AtomicU64,
    pub(crate) first_byte_ms: AtomicU64,
}

impl AttemptProgress {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
            bytes: AtomicU64::new(0),
            first_byte_ms: AtomicU64::new(0),
        }
    }

    pub(crate) fn record(&self, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let previous = self.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        if previous == 0 {
            let elapsed = self.started.elapsed().as_millis().min(u64::MAX as u128) as u64;
            let _ = self.first_byte_ms.compare_exchange(
                0,
                elapsed.saturating_add(1),
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    pub(crate) fn has_first_byte(&self) -> bool {
        self.first_byte_ms.load(Ordering::Relaxed) != 0
    }
}

#[derive(Clone)]
pub(crate) struct DownloadContext {
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) progress: Arc<AttemptProgress>,
}

pub(crate) fn build_agent(preferred_ip: Option<Ipv4Addr>, deadline: &RequestDeadline) -> ureq::Agent {
    let min_timeout = |configured| {
        deadline
            .cap(configured)
            .unwrap_or_else(|_| Duration::from_millis(1))
    };
    let builder = ureq::AgentBuilder::new()
        .timeout_connect(min_timeout(crate::download::downloader::constants::CONNECT_TIMEOUT))
        .timeout_read(min_timeout(crate::download::downloader::constants::READ_TIMEOUT))
        .timeout_write(min_timeout(crate::download::downloader::constants::WRITE_TIMEOUT));
    if let Some(ip) = preferred_ip {
        builder
            .resolver(crate::download::cf_ip::resolver_for(ip))
            .build()
    } else {
        builder.build()
    }
}

pub(crate) fn download_with_fallback(
    agent: &ureq::Agent,
    url: &str,
    part_path: &Path,
    context: &DownloadContext,
    validate: fn(&Path) -> std::result::Result<(), String>,
) -> std::result::Result<(), String> {
    match probe_range_support(agent, url, context) {
        Ok(RangeProbe::Supported(total)) => {
            let parallel_result = download_parallel(agent, url, part_path, total, context)
                .and_then(|_| validate(part_path));
            if parallel_result.is_ok() {
                return parallel_result;
            }

            let parallel_error = parallel_result.unwrap_err();
            check_cancelled(context)?;
            remove_if_exists(part_path);
            download_single(agent, url, part_path, context)
                .and_then(|_| validate(part_path))
                .map_err(|single_error| {
                    format!(
                        "4-part parallel download failed ({parallel_error}); \
                         single-stream fallback failed ({single_error})"
                    )
                })
        }
        Ok(RangeProbe::FullResponse(response)) => download_response(*response, part_path, context)
            .and_then(|_| validate(part_path)),
        Err(probe_error) => {
            check_cancelled(context)?;
            remove_if_exists(part_path);
            download_single(agent, url, part_path, context)
                .and_then(|_| validate(part_path))
                .map_err(|single_error| {
                    format!(
                        "range probe failed ({probe_error}); \
                         single-stream fallback failed ({single_error})"
                    )
                })
        }
    }
}

enum RangeProbe {
    Supported(u64),
    FullResponse(Box<ureq::Response>),
}

fn probe_range_support(
    agent: &ureq::Agent,
    url: &str,
    context: &DownloadContext,
) -> std::result::Result<RangeProbe, String> {
    check_cancelled(context)?;
    let response = agent
        .get(url)
        .set(
            "User-Agent",
            crate::config::current().download.osz.USER_AGENT.as_str(),
        )
        .set("Accept-Encoding", "identity")
        .set("Range", "bytes=0-0")
        .call()
        .map_err(format_http_error)?;
    reject_html(&response)?;

    if response.status() != 206 {
        return Ok(RangeProbe::FullResponse(Box::new(response)));
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

    let mut probe = [0u8; 2];
    let mut read = 0;
    let mut reader = response.into_reader();
    while read < 1 {
        check_cancelled(context)?;
        let count = reader
            .read(&mut probe[read..])
            .map_err(|e| format!("failed to read range probe: {e}"))?;
        if count == 0 {
            break;
        }
        read += count;
    }
    if read != 1 {
        return Err(format!("range probe returned {read} bytes instead of 1"));
    }
    context.progress.record(1);
    Ok(RangeProbe::Supported(range.total))
}

fn download_parallel(
    agent: &ureq::Agent,
    url: &str,
    part_path: &Path,
    total: u64,
    context: &DownloadContext,
) -> std::result::Result<(), String> {
    let ranges = split_ranges(total, crate::config::current().download.osz.PARALLEL_PARTS);
    let file =
        File::create(part_path).map_err(|e| format!("failed to create temporary osz: {e}"))?;
    file.set_len(total)
        .map_err(|e| format!("failed to allocate temporary osz: {e}"))?;
    drop(file);

    let worker_cancel = Arc::new(AtomicBool::new(false));
    let handles = ranges
        .into_iter()
        .map(|range| {
            let agent = agent.clone();
            let url = url.to_string();
            let part_path = part_path.to_path_buf();
            let context = context.clone();
            let worker_cancel = worker_cancel.clone();
            thread::spawn(move || {
                let result =
                    download_range_part(&agent, &url, &part_path, range, &context, &worker_cancel);
                if result.is_err() {
                    worker_cancel.store(true, Ordering::Relaxed);
                }
                result
            })
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
    if context.cancel.load(Ordering::Relaxed) {
        remove_if_exists(part_path);
        return Err("download cancelled".to_string());
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
    context: &DownloadContext,
    worker_cancel: &AtomicBool,
) -> std::result::Result<(), String> {
    check_cancelled_with_worker(context, worker_cancel)?;
    let expected = range.end - range.start + 1;
    let range_header = format!("bytes={}-{}", range.start, range.end);
    let response = agent
        .get(url)
        .set(
            "User-Agent",
            crate::config::current().download.osz.USER_AGENT.as_str(),
        )
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
    let mut reader = response.into_reader();
    let mut buffer = vec![0u8; crate::download::downloader::constants::BUFFER_SIZE];
    loop {
        check_cancelled_with_worker(context, worker_cancel)?;
        let count = reader
            .read(&mut buffer)
            .map_err(|e| format!("failed to read {range_header}: {e}"))?;
        if count == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..count]);
        context.progress.record(count);
        if data.len() as u64 > expected {
            return Err(format!(
                "{range_header} returned more than {expected} bytes"
            ));
        }
    }
    if data.len() as u64 != expected {
        return Err(format!(
            "{range_header} returned {} bytes, expected {expected}",
            data.len()
        ));
    }

    check_cancelled_with_worker(context, worker_cancel)?;
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
    context: &DownloadContext,
) -> std::result::Result<(), String> {
    check_cancelled(context)?;
    let response = agent
        .get(url)
        .set(
            "User-Agent",
            crate::config::current().download.osz.USER_AGENT.as_str(),
        )
        .set("Accept-Encoding", "identity")
        .call()
        .map_err(format_http_error)?;
    download_response(response, part_path, context)
}

fn download_response(
    response: ureq::Response,
    part_path: &Path,
    context: &DownloadContext,
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
    let mut reader = response.into_reader();
    let mut buffer = vec![0u8; crate::download::downloader::constants::BUFFER_SIZE];
    let mut copied = 0u64;
    loop {
        check_cancelled(context)?;
        let count = reader
            .read(&mut buffer)
            .map_err(|e| format!("failed to read response: {e}"))?;
        if count == 0 {
            break;
        }
        copied = copied.saturating_add(count as u64);
        if copied > crate::config::current().download.osz.MAX_OSZ_BYTES {
            remove_if_exists(part_path);
            return Err(format!(
                "OSZ is too large while reading the response: received more than {} bytes ({:.2} MiB)",
                crate::config::current().download.osz.MAX_OSZ_BYTES,
                crate::config::current().download.osz.MAX_OSZ_BYTES as f64 / crate::download::downloader::constants::MIB_BYTES as f64,
            ));
        }
        output
            .write_all(&buffer[..count])
            .map_err(|e| format!("failed to write response: {e}"))?;
        context.progress.record(count);
    }
    output
        .flush()
        .map_err(|e| format!("failed to flush temporary osz: {e}"))?;
    if copied == 0 {
        remove_if_exists(part_path);
        return Err("server returned an empty response".to_string());
    }
    Ok(())
}

fn check_cancelled(context: &DownloadContext) -> std::result::Result<(), String> {
    if context.cancel.load(Ordering::Relaxed) {
        Err("download cancelled".to_string())
    } else {
        Ok(())
    }
}

fn check_cancelled_with_worker(
    context: &DownloadContext,
    worker_cancel: &AtomicBool,
) -> std::result::Result<(), String> {
    if context.cancel.load(Ordering::Relaxed) || worker_cancel.load(Ordering::Relaxed) {
        Err("download cancelled".to_string())
    } else {
        Ok(())
    }
}

fn format_http_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, _) => format!("http {code}"),
        other => other.to_string(),
    }
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

fn validate_declared_size(length: u64) -> std::result::Result<(), String> {
    if length == 0 {
        return Err("server declared an empty response (Content-Length: 0 bytes)".to_string());
    }
    if length > crate::config::current().download.osz.MAX_OSZ_BYTES {
        return Err(format!(
            "OSZ is too large: server declared {length} bytes ({:.2} MiB), exceeding the download limit of {} bytes ({:.2} MiB)",
            length as f64 / crate::download::downloader::constants::MIB_BYTES as f64,
            crate::config::current().download.osz.MAX_OSZ_BYTES,
            crate::config::current().download.osz.MAX_OSZ_BYTES as f64 / crate::download::downloader::constants::MIB_BYTES as f64,
        ));
    }
    Ok(())
}

pub(crate) fn parse_content_range(value: &str) -> std::result::Result<ContentRange, String> {
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

pub(crate) fn split_ranges(total: u64, requested_parts: usize) -> Vec<ContentRange> {
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

pub(crate) fn remove_if_exists(path: &Path) {
    let _ = std::fs::remove_file(path);
}
