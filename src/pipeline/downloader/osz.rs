use crate::core::errors::{PreviewError, Result};
use std::cmp::Reverse;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContentRange {
    start: u64,
    end: u64,
    total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MirrorSource {
    Sayobot,
    OsuDirectPreferred(Ipv4Addr),
    OsuDirectDns,
    Nekoha,
    Catboy,
}

impl MirrorSource {
    fn name(self) -> &'static str {
        match self {
            Self::Sayobot => "sayobot",
            Self::OsuDirectPreferred(_) => "osu.direct-preferred-ip",
            Self::OsuDirectDns => "osu.direct-dns",
            Self::Nekoha => "nekoha",
            Self::Catboy => "catboy",
        }
    }

    fn url(self, set_id: u64) -> String {
        match self {
            Self::Sayobot => format!("https://txy1.sayobot.cn/beatmaps/download/novideo/{set_id}"),
            Self::OsuDirectPreferred(_) | Self::OsuDirectDns => {
                format!("https://osu.direct/api/d/{set_id}")
            }
            Self::Nekoha => {
                format!("https://mirror.nekoha.moe/api/download/{set_id}?noVideo=1")
            }
            Self::Catboy => format!("https://catboy.best/d/{set_id}"),
        }
    }

    fn preferred_ip(self) -> Option<Ipv4Addr> {
        match self {
            Self::OsuDirectPreferred(ip) => Some(ip),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct MirrorCandidate {
    source: MirrorSource,
}

struct AttemptProgress {
    started: Instant,
    bytes: AtomicU64,
    first_byte_ms: AtomicU64,
}

impl AttemptProgress {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            bytes: AtomicU64::new(0),
            first_byte_ms: AtomicU64::new(0),
        }
    }

    fn record(&self, bytes: usize) {
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

    fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    fn has_first_byte(&self) -> bool {
        self.first_byte_ms.load(Ordering::Relaxed) != 0
    }
}

#[derive(Clone)]
struct DownloadContext {
    cancel: Arc<AtomicBool>,
    progress: Arc<AttemptProgress>,
}

struct AttemptResult {
    id: usize,
    result: std::result::Result<(), String>,
}

enum HandledAttempt {
    Ignored,
    Failed,
    Won(PathBuf),
}

struct ActiveAttempt {
    id: usize,
    source: MirrorSource,
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    progress: Arc<AttemptProgress>,
    monitor: AttemptMonitor,
    first_byte_logged: bool,
    last_speed_log: Instant,
    handle: JoinHandle<()>,
}

struct AttemptMonitor {
    samples: VecDeque<(Instant, u64)>,
    fallback_triggered: bool,
}

#[derive(Debug, Clone)]
struct OszLogContext {
    request_bid: String,
    set_id: u64,
}

impl OszLogContext {
    fn new(request_bid: &str, set_id: u64) -> Self {
        Self {
            request_bid: request_bid.to_string(),
            set_id,
        }
    }

    fn event(&self, status: &str, msg: impl AsRef<str>) {
        crate::log::event(
            "download-osz",
            status,
            Some(&self.request_bid),
            &self.message(msg),
        );
    }

    fn message(&self, msg: impl AsRef<str>) -> String {
        format!(
            "bid={} set={} {}",
            self.request_bid,
            self.set_id,
            msg.as_ref()
        )
    }
}

impl AttemptMonitor {
    fn new() -> Self {
        Self {
            samples: VecDeque::new(),
            fallback_triggered: false,
        }
    }

    fn fallback_reason(
        &mut self,
        now: Instant,
        started: Instant,
        progress: &AttemptProgress,
    ) -> Option<&'static str> {
        if self.fallback_triggered {
            return None;
        }

        let bytes = progress.bytes();
        self.samples.push_back((now, bytes));
        while self.samples.len() > 1
            && self.samples.get(1).is_some_and(|(time, _)| {
                now.duration_since(*time)
                    >= crate::config::current()
                        .network
                        .downloader_osz
                        .LOW_SPEED_WINDOW
            })
        {
            self.samples.pop_front();
        }

        let reason = if !progress.has_first_byte()
            && now.duration_since(started)
                >= crate::config::current()
                    .network
                    .downloader_osz
                    .NO_FIRST_BYTE_TIMEOUT
        {
            Some("no-first-byte")
        } else if let Some((oldest, old_bytes)) = self.samples.front().copied() {
            let elapsed = now.duration_since(oldest);
            let received = bytes.saturating_sub(old_bytes);
            if elapsed
                >= crate::config::current()
                    .network
                    .downloader_osz
                    .LOW_SPEED_WINDOW
                && received.saturating_mul(1000)
                    < crate::config::current()
                        .network
                        .downloader_osz
                        .LOW_SPEED_BYTES_PER_SECOND
                        .saturating_mul(elapsed.as_millis() as u64)
            {
                Some("low-speed")
            } else {
                None
            }
        } else {
            None
        };

        if reason.is_some() {
            self.fallback_triggered = true;
        }
        reason
    }

    fn recent_bytes_per_second(&self, now: Instant, current_bytes: u64) -> u64 {
        let Some((oldest, old_bytes)) = self.samples.front().copied() else {
            return 0;
        };
        let elapsed_ms = now.duration_since(oldest).as_millis() as u64;
        if elapsed_ms == 0 {
            return 0;
        }
        current_bytes.saturating_sub(old_bytes).saturating_mul(1000) / elapsed_ms
    }
}

pub fn download_beatmapset_archive(
    request_bid: &str,
    set_id: u64,
    temp_dir: &Path,
    no_cache: bool,
) -> Result<PathBuf> {
    let log = OszLogContext::new(request_bid, set_id);
    std::fs::create_dir_all(temp_dir)
        .map_err(|e| PreviewError::download(format!("failed to create osz cache dir: {e}")))?;
    let target_path = temp_dir.join(format!("{set_id}.osz"));
    if !no_cache && valid_osz(&target_path) {
        let size_mib = target_path
            .metadata()
            .map(|m| {
                m.len() as f64 / crate::config::current().network.downloader_osz.MIB_BYTES as f64
            })
            .unwrap_or(0.0);
        log.event("done", format!("cache hit ({size_mib:.1} MiB)"));
        crate::log::record_cache(crate::log::CacheKind::Osz, "hit");
        crate::log::record_stage_status("download_osz_ms", "hit");
        return Ok(target_path);
    }

    let preferred_ip = crate::pipeline::downloader::cf_ip::read_preferred_ip(temp_dir);
    crate::pipeline::downloader::cf_ip::spawn_refresh(temp_dir, preferred_ip.is_none());
    let candidates = build_candidates(preferred_ip);
    log.event(
        "start",
        format!(
            "race={} candidates={} preferred_ip={}",
            crate::config::current()
                .network
                .downloader_osz
                .MAX_ACTIVE_ATTEMPTS,
            candidates.len(),
            preferred_ip
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "none".to_string())
        ),
    );

    let started = Instant::now();
    let winner = match run_download_race(&log, temp_dir, candidates) {
        Ok(winner) => winner,
        Err(failures) => {
            crate::pipeline::downloader::cf_ip::invalidate(temp_dir);
            crate::pipeline::downloader::cf_ip::spawn_refresh(temp_dir, true);
            log.event("error", failures.join("; "));
            return Err(PreviewError::download(format!(
                "failed to download beatmapset {set_id} from all mirrors: {}",
                failures.join("; ")
            )));
        }
    };

    if target_path.exists() {
        std::fs::remove_file(&target_path)
            .map_err(|e| PreviewError::download(format!("failed to replace osz cache: {e}")))?;
    }
    std::fs::rename(&winner, &target_path)
        .map_err(|e| PreviewError::download(format!("failed to commit osz cache: {e}")))?;
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let size_mib = target_path
        .metadata()
        .map(|m| m.len() as f64 / crate::config::current().network.downloader_osz.MIB_BYTES as f64)
        .unwrap_or(0.0);
    log.event(
        "done",
        format!("downloaded {size_mib:.1} MiB in {ms:.0} ms"),
    );
    crate::log::record_cache(crate::log::CacheKind::Osz, "downloaded");
    crate::log::record_stage("download_osz_ms", ms);
    Ok(target_path)
}

fn build_candidates(preferred_ip: Option<Ipv4Addr>) -> Vec<MirrorCandidate> {
    let mut candidates = vec![MirrorCandidate {
        source: MirrorSource::Sayobot,
    }];
    if let Some(ip) = preferred_ip {
        candidates.push(MirrorCandidate {
            source: MirrorSource::OsuDirectPreferred(ip),
        });
    }
    candidates.extend([
        MirrorCandidate {
            source: MirrorSource::OsuDirectDns,
        },
        MirrorCandidate {
            source: MirrorSource::Nekoha,
        },
        MirrorCandidate {
            source: MirrorSource::Catboy,
        },
    ]);
    candidates
}

fn build_agent(preferred_ip: Option<Ipv4Addr>) -> ureq::Agent {
    let builder = ureq::AgentBuilder::new()
        .timeout_connect(
            crate::config::current()
                .network
                .downloader_osz
                .CONNECT_TIMEOUT,
        )
        .timeout_read(crate::config::current().network.downloader_osz.READ_TIMEOUT)
        .timeout_write(
            crate::config::current()
                .network
                .downloader_osz
                .WRITE_TIMEOUT,
        );
    if let Some(ip) = preferred_ip {
        builder
            .resolver(crate::pipeline::downloader::cf_ip::resolver_for(ip))
            .build()
    } else {
        builder.build()
    }
}

fn run_download_race(
    log: &OszLogContext,
    temp_dir: &Path,
    mut candidates: Vec<MirrorCandidate>,
) -> std::result::Result<PathBuf, Vec<String>> {
    let (sender, receiver) = mpsc::channel();
    let mut active = Vec::new();
    let mut failures = Vec::new();
    let mut next_candidate = 0;
    let mut next_id = 0;
    let deadline = Instant::now()
        + crate::config::current()
            .network
            .downloader_osz
            .DOWNLOAD_HARD_TIMEOUT;
    let mut preferred_refresh_triggered = false;

    start_next_attempt(
        log,
        temp_dir,
        &mut candidates,
        &mut next_candidate,
        &mut next_id,
        &sender,
        &mut active,
    );

    loop {
        if Instant::now() >= deadline {
            for attempt in active.drain(..) {
                cancel_attempt(log, attempt);
            }
            failures.push("global download deadline exceeded".to_string());
            return Err(failures);
        }

        match receiver.recv_timeout(
            crate::config::current()
                .network
                .downloader_osz
                .POLL_INTERVAL,
        ) {
            Ok(message) => {
                let outcome = handle_attempt_result(
                    message,
                    &mut active,
                    &mut failures,
                    &mut preferred_refresh_triggered,
                    temp_dir,
                    log,
                );
                if let HandledAttempt::Won(winner) = outcome {
                    for attempt in active.drain(..) {
                        cancel_attempt(log, attempt);
                    }
                    return Ok(winner);
                }
                if matches!(outcome, HandledAttempt::Failed) {
                    start_next_attempt(
                        log,
                        temp_dir,
                        &mut candidates,
                        &mut next_candidate,
                        &mut next_id,
                        &sender,
                        &mut active,
                    );
                }
                while let Ok(message) = receiver.try_recv() {
                    let outcome = handle_attempt_result(
                        message,
                        &mut active,
                        &mut failures,
                        &mut preferred_refresh_triggered,
                        temp_dir,
                        log,
                    );
                    if let HandledAttempt::Won(winner) = outcome {
                        for attempt in active.drain(..) {
                            cancel_attempt(log, attempt);
                        }
                        return Ok(winner);
                    }
                    if matches!(outcome, HandledAttempt::Failed) {
                        start_next_attempt(
                            log,
                            temp_dir,
                            &mut candidates,
                            &mut next_candidate,
                            &mut next_id,
                            &sender,
                            &mut active,
                        );
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                for attempt in active.drain(..) {
                    cancel_attempt(log, attempt);
                }
                failures.push("all download workers exited unexpectedly".to_string());
                return Err(failures);
            }
        }

        let now = Instant::now();
        for attempt in &mut active {
            if attempt.progress.has_first_byte() && !attempt.first_byte_logged {
                attempt.first_byte_logged = true;
                log.event(
                    "first-byte",
                    format!(
                        "attempt={} source={} after_ms={}",
                        attempt.id,
                        attempt.source.name(),
                        attempt.progress.started.elapsed().as_millis()
                    ),
                );
            }
            if attempt.progress.has_first_byte()
                && now.duration_since(attempt.last_speed_log)
                    >= crate::config::current()
                        .network
                        .downloader_osz
                        .LOW_SPEED_WINDOW
            {
                attempt.last_speed_log = now;
                let speed = attempt
                    .monitor
                    .recent_bytes_per_second(now, attempt.progress.bytes());
                log.event(
                    "speed",
                    format!(
                        "attempt={} source={} bytes={} speed_kib_s={:.1}",
                        attempt.id,
                        attempt.source.name(),
                        attempt.progress.bytes(),
                        speed as f64 / 1024.0
                    ),
                );
            }
        }
        let fallback = if next_candidate < candidates.len() {
            active.iter_mut().find_map(|attempt| {
                attempt
                    .monitor
                    .fallback_reason(now, attempt.progress.started, &attempt.progress)
                    .map(|reason| (attempt.id, reason))
            })
        } else {
            None
        };
        if let Some((attempt_id, reason)) = fallback {
            log.event(
                "fallback",
                format!(
                    "attempt={attempt_id} source={} reason={reason} active={}",
                    active
                        .iter()
                        .find(|attempt| attempt.id == attempt_id)
                        .map(|attempt| attempt.source.name())
                        .unwrap_or("unknown"),
                    active.len(),
                ),
            );
            if active.len()
                >= crate::config::current()
                    .network
                    .downloader_osz
                    .MAX_ACTIVE_ATTEMPTS
            {
                if let Some(position) = slowest_attempt(&active, now) {
                    let attempt = active.swap_remove(position);
                    cancel_attempt(log, attempt);
                }
            }
            start_next_attempt(
                log,
                temp_dir,
                &mut candidates,
                &mut next_candidate,
                &mut next_id,
                &sender,
                &mut active,
            );
        }

        if active.is_empty() {
            if next_candidate >= candidates.len() {
                return Err(failures);
            }
            start_next_attempt(
                log,
                temp_dir,
                &mut candidates,
                &mut next_candidate,
                &mut next_id,
                &sender,
                &mut active,
            );
        }
    }
}

fn start_next_attempt(
    log: &OszLogContext,
    temp_dir: &Path,
    candidates: &mut Vec<MirrorCandidate>,
    next_candidate: &mut usize,
    next_id: &mut usize,
    sender: &mpsc::Sender<AttemptResult>,
    active: &mut Vec<ActiveAttempt>,
) {
    maybe_insert_preferred_candidate(candidates, *next_candidate, temp_dir);
    if *next_candidate >= candidates.len()
        || active.len()
            >= crate::config::current()
                .network
                .downloader_osz
                .MAX_ACTIVE_ATTEMPTS
    {
        return;
    }
    let source = candidates[*next_candidate].source;
    *next_candidate += 1;
    let id = *next_id;
    *next_id += 1;
    let path = attempt_path(temp_dir, log.set_id, id);
    remove_if_exists(&path);
    let cancel = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(AttemptProgress::new());
    let context = DownloadContext {
        cancel: cancel.clone(),
        progress: progress.clone(),
    };
    let url = source.url(log.set_id);
    let agent = build_agent(source.preferred_ip());
    log.event(
        "attempt-start",
        format!("attempt={id} source={} url={url}", source.name()),
    );
    let worker_sender = sender.clone();
    let worker_path = path.clone();
    let handle = thread::spawn(move || {
        let result = download_osz_once(&agent, &url, &worker_path, &context);
        let _ = worker_sender.send(AttemptResult { id, result });
    });
    active.push(ActiveAttempt {
        id,
        source,
        path,
        cancel,
        progress,
        monitor: AttemptMonitor::new(),
        first_byte_logged: false,
        last_speed_log: Instant::now(),
        handle,
    });
}

fn maybe_insert_preferred_candidate(
    candidates: &mut Vec<MirrorCandidate>,
    next_candidate: usize,
    temp_dir: &Path,
) {
    if candidates
        .iter()
        .any(|candidate| matches!(candidate.source, MirrorSource::OsuDirectPreferred(_)))
        || !matches!(
            candidates
                .get(next_candidate)
                .map(|candidate| candidate.source),
            Some(MirrorSource::OsuDirectDns)
        )
    {
        return;
    }
    if let Some(ip) = crate::pipeline::downloader::cf_ip::read_preferred_ip(temp_dir) {
        candidates.insert(
            next_candidate,
            MirrorCandidate {
                source: MirrorSource::OsuDirectPreferred(ip),
            },
        );
    }
}

fn handle_attempt_result(
    message: AttemptResult,
    active: &mut Vec<ActiveAttempt>,
    failures: &mut Vec<String>,
    preferred_refresh_triggered: &mut bool,
    temp_dir: &Path,
    log: &OszLogContext,
) -> HandledAttempt {
    let Some(position) = active.iter().position(|attempt| attempt.id == message.id) else {
        return HandledAttempt::Ignored;
    };
    let attempt = active.swap_remove(position);
    let source = attempt.source;
    let path = attempt.path.clone();
    let _ = attempt.handle.join();
    match message.result {
        Ok(()) => {
            log.event(
                "winner",
                format!("attempt={} source={} validated", attempt.id, source.name()),
            );
            HandledAttempt::Won(path)
        }
        Err(reason) => {
            remove_if_exists(&path);
            log.event(
                "attempt-error",
                format!("attempt={} source={} {reason}", attempt.id, source.name()),
            );
            if matches!(source, MirrorSource::OsuDirectPreferred(_))
                && !*preferred_refresh_triggered
            {
                *preferred_refresh_triggered = true;
                crate::pipeline::downloader::cf_ip::invalidate(temp_dir);
                crate::pipeline::downloader::cf_ip::spawn_refresh(temp_dir, true);
            }
            failures.push(format!("{}: {reason}", source.name()));
            HandledAttempt::Failed
        }
    }
}

fn slowest_attempt(active: &[ActiveAttempt], now: Instant) -> Option<usize> {
    active
        .iter()
        .enumerate()
        .min_by_key(|(_, attempt)| {
            (
                attempt
                    .monitor
                    .recent_bytes_per_second(now, attempt.progress.bytes()),
                Reverse(attempt.id),
            )
        })
        .map(|(position, _)| position)
}

fn cancel_attempt(log: &OszLogContext, attempt: ActiveAttempt) {
    attempt.cancel.store(true, Ordering::Relaxed);
    log.event(
        "attempt-cancelled",
        format!("attempt={} source={}", attempt.id, attempt.source.name()),
    );
    thread::spawn(move || {
        let _ = attempt.handle.join();
        remove_if_exists(&attempt.path);
    });
}

fn attempt_path(temp_dir: &Path, set_id: u64, attempt_id: usize) -> PathBuf {
    temp_dir.join(format!(
        "{set_id}.{}.attempt-{attempt_id}.osz.part",
        std::process::id()
    ))
}

fn download_osz_once(
    agent: &ureq::Agent,
    url: &str,
    part_path: &Path,
    context: &DownloadContext,
) -> std::result::Result<(), String> {
    match probe_range_support(agent, url, context) {
        Ok(RangeProbe::Supported(total)) => {
            let parallel_result = download_parallel(agent, url, part_path, total, context)
                .and_then(|_| validate_downloaded_archive(part_path));
            if parallel_result.is_ok() {
                return parallel_result;
            }

            let parallel_error = parallel_result.unwrap_err();
            check_cancelled(context)?;
            remove_if_exists(part_path);
            download_single(agent, url, part_path, context)
                .and_then(|_| validate_downloaded_archive(part_path))
                .map_err(|single_error| {
                    format!(
                        "4-part parallel download failed ({parallel_error}); \
                         single-stream fallback failed ({single_error})"
                    )
                })
        }
        Ok(RangeProbe::FullResponse(response)) => download_response(*response, part_path, context)
            .and_then(|_| validate_downloaded_archive(part_path)),
        Err(probe_error) => {
            check_cancelled(context)?;
            remove_if_exists(part_path);
            download_single(agent, url, part_path, context)
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
            crate::config::current()
                .network
                .downloader_osz
                .USER_AGENT
                .as_str(),
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
    let ranges = split_ranges(
        total,
        crate::config::current()
            .network
            .downloader_osz
            .PARALLEL_PARTS,
    );
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
            crate::config::current()
                .network
                .downloader_osz
                .USER_AGENT
                .as_str(),
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
    let mut buffer = vec![0u8; crate::config::current().network.downloader_osz.BUFFER_SIZE];
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
            crate::config::current()
                .network
                .downloader_osz
                .USER_AGENT
                .as_str(),
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
    let mut buffer = vec![0u8; crate::config::current().network.downloader_osz.BUFFER_SIZE];
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
        if copied
            > crate::config::current()
                .network
                .downloader_osz
                .MAX_OSZ_BYTES
        {
            remove_if_exists(part_path);
            return Err(format!(
                "OSZ is too large while reading the response: received more than {} bytes ({:.2} MiB)",
                crate::config::current().network.downloader_osz.MAX_OSZ_BYTES,
                crate::config::current().network.downloader_osz.MAX_OSZ_BYTES as f64 / crate::config::current().network.downloader_osz.MIB_BYTES as f64,
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

fn validate_downloaded_archive(path: &Path) -> std::result::Result<(), String> {
    if valid_osz(path) {
        Ok(())
    } else {
        Err("response is not a valid ZIP/OSZ archive".to_string())
    }
}

fn validate_declared_size(length: u64) -> std::result::Result<(), String> {
    if length == 0 {
        return Err("server declared an empty response (Content-Length: 0 bytes)".to_string());
    }
    if length
        > crate::config::current()
            .network
            .downloader_osz
            .MAX_OSZ_BYTES
    {
        return Err(format!(
            "OSZ is too large: server declared {length} bytes ({:.2} MiB), exceeding the download limit of {} bytes ({:.2} MiB)",
            length as f64 / crate::config::current().network.downloader_osz.MIB_BYTES as f64,
            crate::config::current().network.downloader_osz.MAX_OSZ_BYTES,
            crate::config::current().network.downloader_osz.MAX_OSZ_BYTES as f64 / crate::config::current().network.downloader_osz.MIB_BYTES as f64,
        ));
    }
    Ok(())
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

fn valid_osz(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file()
        || meta.len() == 0
        || meta.len()
            > crate::config::current()
                .network
                .downloader_osz
                .MAX_OSZ_BYTES
    {
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
        let started = Instant::now()
            - crate::config::current()
                .network
                .downloader_osz
                .NO_FIRST_BYTE_TIMEOUT;
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
        monitor.samples.push_back((
            now - crate::config::current()
                .network
                .downloader_osz
                .LOW_SPEED_WINDOW,
            0,
        ));
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
            for _ in 0..=crate::config::current()
                .network
                .downloader_osz
                .PARALLEL_PARTS
            {
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
}
