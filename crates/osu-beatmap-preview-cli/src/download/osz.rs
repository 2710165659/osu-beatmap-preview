use crate::download::downloader;
#[cfg(test)]
pub(crate) use crate::download::downloader::{parse_content_range, split_ranges, ContentRange};
pub(crate) use crate::download::downloader::{AttemptProgress, DownloadContext};
use osu_beatmap_preview_core::support::error::{PreviewError, Result};
use osu_beatmap_preview_core::support::timeout::RequestDeadline;
use std::cmp::Reverse;
use std::collections::VecDeque;
use std::fs::File;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

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

    /// 构造下载 URL。
    ///
    /// `download_video` 决定要带视频的完整包还是 novideo 去视频包：取值来自
    /// 当前模式的 `ENABLE_BACKGROUND_VIDEO`（背景视频开启才有素材需求），
    /// 去视频包则明显更省流量。只有 sayobot / nekoha 提供两种变体，
    /// 其余镜像本来就是完整包。
    fn url(self, set_id: u64, download_video: bool) -> String {
        match self {
            Self::Sayobot => {
                if download_video {
                    format!("https://txy1.sayobot.cn/beatmaps/download/{set_id}")
                } else {
                    format!("https://txy1.sayobot.cn/beatmaps/download/novideo/{set_id}")
                }
            }
            Self::OsuDirectPreferred(_) | Self::OsuDirectDns => {
                format!("https://osu.direct/api/d/{set_id}")
            }
            Self::Nekoha => {
                if download_video {
                    format!("https://mirror.nekoha.moe/api/download/{set_id}")
                } else {
                    format!("https://mirror.nekoha.moe/api/download/{set_id}?noVideo=1")
                }
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
        crate::logging::event(
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
                    >= crate::download::downloader::constants::LOW_SPEED_WINDOW
            })
        {
            self.samples.pop_front();
        }

        let reason = if !progress.has_first_byte()
            && now.duration_since(started)
                >= crate::download::downloader::constants::NO_FIRST_BYTE_TIMEOUT
        {
            Some("no-first-byte")
        } else if let Some((oldest, old_bytes)) = self.samples.front().copied() {
            let elapsed = now.duration_since(oldest);
            let received = bytes.saturating_sub(old_bytes);
            if elapsed >= crate::download::downloader::constants::LOW_SPEED_WINDOW
                && received.saturating_mul(1000)
                    < crate::download::downloader::constants::LOW_SPEED_BYTES_PER_SECOND
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
    deadline: &RequestDeadline,
    download_video: bool,
) -> Result<PathBuf> {
    deadline.check()?;
    let log = OszLogContext::new(request_bid, set_id);
    std::fs::create_dir_all(temp_dir)
        .map_err(|e| PreviewError::download(format!("failed to create osz cache dir: {e}")))?;
    // 下载结果仍按视频变体分开保存；完整包可满足无视频请求，反向复用则会缺素材。
    let variant = if download_video { "video" } else { "novideo" };
    let target_path = temp_dir.join(format!("{set_id}-{variant}.osz"));
    if let Some(cached_path) = cached_osz(temp_dir, set_id, download_video, no_cache) {
        deadline.check()?;
        let size_mib = cached_path
            .metadata()
            .map(|m| m.len() as f64 / crate::download::downloader::constants::MIB_BYTES as f64)
            .unwrap_or(0.0);
        log.event(
            "done",
            format!(
                "cache hit ({size_mib:.1} MiB) path={}",
                cached_path.display()
            ),
        );
        crate::logging::record_cache(crate::logging::CacheKind::Osz, "hit");
        crate::logging::record_stage_status("download_osz_ms", "hit");
        return Ok(cached_path);
    }

    let preferred_ip = crate::download::cf_ip::read_preferred_ip(temp_dir);
    crate::download::cf_ip::spawn_refresh(temp_dir, preferred_ip.is_none());
    let candidates = build_candidates(preferred_ip);
    log.event(
        "start",
        format!(
            "race={} candidates={} preferred_ip={}",
            crate::config::current().download.osz.MAX_ACTIVE_ATTEMPTS,
            candidates.len(),
            preferred_ip
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "none".to_string())
        ),
    );

    let started = Instant::now();
    let winner = match run_download_race(&log, temp_dir, candidates, deadline, download_video) {
        Ok(winner) => winner,
        Err(failures) => {
            deadline.check()?;
            crate::download::cf_ip::invalidate(temp_dir);
            crate::download::cf_ip::spawn_refresh(temp_dir, true);
            log.event("error", failures.join("; "));
            return Err(PreviewError::download(format!(
                "failed to download beatmapset {set_id} from all mirrors: {}",
                failures.join("; ")
            )));
        }
    };
    deadline.check()?;

    if target_path.exists() {
        std::fs::remove_file(&target_path)
            .map_err(|e| PreviewError::download(format!("failed to replace osz cache: {e}")))?;
    }
    std::fs::rename(&winner, &target_path)
        .map_err(|e| PreviewError::download(format!("failed to commit osz cache: {e}")))?;
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let size_mib = target_path
        .metadata()
        .map(|m| m.len() as f64 / crate::download::downloader::constants::MIB_BYTES as f64)
        .unwrap_or(0.0);
    log.event(
        "done",
        format!("downloaded {size_mib:.1} MiB in {ms:.0} ms"),
    );
    crate::logging::record_cache(crate::logging::CacheKind::Osz, "downloaded");
    crate::logging::record_stage("download_osz_ms", ms);
    Ok(target_path)
}

fn cached_osz(
    temp_dir: &Path,
    set_id: u64,
    download_video: bool,
    no_cache: bool,
) -> Option<PathBuf> {
    if no_cache {
        return None;
    }
    // 优先使用请求的变体；去视频包缺失或损坏时可直接读完整包，无需复制或重新下载。
    let variants: &[&str] = if download_video {
        &["video"]
    } else {
        &["novideo", "video"]
    };
    variants
        .iter()
        .map(|variant| temp_dir.join(format!("{set_id}-{variant}.osz")))
        .find(|path| valid_osz(path))
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

fn run_download_race(
    log: &OszLogContext,
    temp_dir: &Path,
    mut candidates: Vec<MirrorCandidate>,
    request_deadline: &RequestDeadline,
    download_video: bool,
) -> std::result::Result<PathBuf, Vec<String>> {
    let (sender, receiver) = mpsc::channel();
    let mut active = Vec::new();
    let mut failures = Vec::new();
    let mut next_candidate = 0;
    let mut next_id = 0;
    let deadline = Instant::now() + crate::config::current().download.osz.DOWNLOAD_HARD_TIMEOUT;
    let mut preferred_refresh_triggered = false;

    start_next_attempt(
        log,
        temp_dir,
        &mut candidates,
        &mut next_candidate,
        &mut next_id,
        &sender,
        &mut active,
        request_deadline,
        download_video,
    );

    loop {
        if request_deadline.check().is_err() {
            for attempt in active.drain(..) {
                cancel_attempt(log, attempt);
            }
            failures.push("preview request deadline exceeded".to_string());
            return Err(failures);
        }
        if Instant::now() >= deadline {
            for attempt in active.drain(..) {
                cancel_attempt(log, attempt);
            }
            failures.push("global download deadline exceeded".to_string());
            return Err(failures);
        }

        let poll_interval = request_deadline
            .cap(crate::download::downloader::constants::POLL_INTERVAL)
            .unwrap_or_else(|_| Duration::from_millis(1));
        match receiver.recv_timeout(poll_interval) {
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
                        request_deadline,
                        download_video,
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
                            request_deadline,
                            download_video,
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
                    >= crate::download::downloader::constants::LOW_SPEED_WINDOW
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
            if active.len() >= crate::config::current().download.osz.MAX_ACTIVE_ATTEMPTS {
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
                request_deadline,
                download_video,
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
                request_deadline,
                download_video,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn start_next_attempt(
    log: &OszLogContext,
    temp_dir: &Path,
    candidates: &mut Vec<MirrorCandidate>,
    next_candidate: &mut usize,
    next_id: &mut usize,
    sender: &mpsc::Sender<AttemptResult>,
    active: &mut Vec<ActiveAttempt>,
    deadline: &RequestDeadline,
    download_video: bool,
) {
    maybe_insert_preferred_candidate(candidates, *next_candidate, temp_dir);
    if *next_candidate >= candidates.len()
        || active.len() >= crate::config::current().download.osz.MAX_ACTIVE_ATTEMPTS
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
    // 带不带视频由当前模式的 ENABLE_BACKGROUND_VIDEO 决定：背景视频开启时
    // 要完整包才有素材，关闭时用去视频包省流量。
    let url = source.url(log.set_id, download_video);
    let agent = downloader::build_agent(source.preferred_ip(), deadline);
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
    if let Some(ip) = crate::download::cf_ip::read_preferred_ip(temp_dir) {
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
                crate::download::cf_ip::invalidate(temp_dir);
                crate::download::cf_ip::spawn_refresh(temp_dir, true);
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
    let _ = attempt.handle.join();
    remove_if_exists(&attempt.path);
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
    downloader::download_with_fallback(agent, url, part_path, context, validate_downloaded_archive)
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
    if !meta.is_file()
        || meta.len() == 0
        || meta.len() > crate::config::current().download.osz.MAX_OSZ_BYTES
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
    use std::io::{BufRead, BufReader, Cursor, Write};
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
        let started =
            Instant::now() - crate::download::downloader::constants::NO_FIRST_BYTE_TIMEOUT;
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
            now - crate::download::downloader::constants::LOW_SPEED_WINDOW,
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

    /// 当前模式的 `ENABLE_BACKGROUND_VIDEO` 决定 sayobot / nekoha 用带视频的完整包
    /// 还是 novideo 去视频包；其余镜像本来就是完整包，不受开关影响。
    #[test]
    fn download_video_flag_selects_package_variant() {
        assert_eq!(
            MirrorSource::Sayobot.url(123, true),
            "https://txy1.sayobot.cn/beatmaps/download/123"
        );
        assert_eq!(
            MirrorSource::Sayobot.url(123, false),
            "https://txy1.sayobot.cn/beatmaps/download/novideo/123"
        );
        assert_eq!(
            MirrorSource::Nekoha.url(123, true),
            "https://mirror.nekoha.moe/api/download/123"
        );
        assert_eq!(
            MirrorSource::Nekoha.url(123, false),
            "https://mirror.nekoha.moe/api/download/123?noVideo=1"
        );
        assert_eq!(
            MirrorSource::Catboy.url(123, false),
            "https://catboy.best/d/123"
        );
        assert_eq!(
            MirrorSource::OsuDirectDns.url(123, true),
            "https://osu.direct/api/d/123"
        );
    }

    #[test]
    fn cached_archive_reuses_full_package_without_satisfying_video_from_novideo() {
        let root = std::env::temp_dir().join(format!(
            "osu-preview-osz-variant-cache-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let video = root.join("123-video.osz");
        let novideo = root.join("123-novideo.osz");
        let archive = make_test_osz();
        std::fs::write(&video, &archive).unwrap();
        let deadline = RequestDeadline::new(Instant::now(), "mp4", Duration::from_secs(10));
        // 从公共入口验证完整包会直接命中，且不会生成额外的 novideo 副本。
        assert_eq!(
            download_beatmapset_archive("42", 123, &root, false, &deadline, false).unwrap(),
            video
        );
        assert!(!novideo.exists());
        assert_eq!(cached_osz(&root, 123, true, false), Some(video.clone()));
        assert!(cached_osz(&root, 123, false, true).is_none());
        assert!(cached_osz(&root, 123, true, true).is_none());

        std::fs::write(&novideo, &archive).unwrap();
        assert_eq!(cached_osz(&root, 123, false, false), Some(novideo.clone()));
        std::fs::write(&novideo, b"broken").unwrap();
        assert_eq!(cached_osz(&root, 123, false, false), Some(video.clone()));
        std::fs::write(&novideo, &archive).unwrap();
        std::fs::write(&video, b"broken").unwrap();
        assert!(cached_osz(&root, 123, true, false).is_none());
        assert_eq!(cached_osz(&root, 123, false, false), Some(novideo));
        std::fs::remove_file(video).unwrap();
        assert!(cached_osz(&root, 123, true, false).is_none());
        std::fs::remove_dir_all(root).unwrap();
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
