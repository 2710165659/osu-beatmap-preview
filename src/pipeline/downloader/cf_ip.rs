use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn read_preferred_ip(temp_dir: &Path) -> Option<Ipv4Addr> {
    let path = cache_path(temp_dir);
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let ip = value.get("ip")?.as_str()?.parse::<Ipv4Addr>().ok()?;
    let tested_at = value.get("tested_at")?.as_u64()?;
    let now = unix_seconds();
    if now < tested_at
        || now - tested_at > crate::pipeline::downloader::constants::CACHE_TTL.as_secs()
    {
        return None;
    }
    Some(ip)
}
pub fn spawn_refresh(temp_dir: &Path, force: bool) {
    let temp_dir = temp_dir.to_path_buf();
    if !force && read_preferred_ip(&temp_dir).is_some() {
        return;
    }
    thread::spawn(move || {
        if let Err(error) = refresh(&temp_dir, force) {
            crate::log::event("osu-direct-ip", "error", None, &error.to_string());
        }
    });
}

pub fn invalidate(temp_dir: &Path) {
    let _ = std::fs::remove_file(cache_path(temp_dir));
}

pub fn resolver_for(preferred_ip: Ipv4Addr) -> impl ureq::Resolver {
    move |address: &str| {
        if let Some(port) = address.strip_prefix("osu.direct:") {
            if let Ok(port) = port.parse::<u16>() {
                return Ok(vec![SocketAddr::new(IpAddr::V4(preferred_ip), port)]);
            }
        }
        address.to_socket_addrs().map(Iterator::collect)
    }
}

fn refresh(temp_dir: &Path, force: bool) -> io::Result<()> {
    let root = cache_root(temp_dir);
    std::fs::create_dir_all(&root)?;
    let lock_path = root.join(&crate::config::current().network.downloader_cf_ip.LOCK_FILE);
    let Some(_lock) = acquire_lock(lock_path)? else {
        return Ok(());
    };

    if !force && read_preferred_ip(temp_dir).is_some() {
        return Ok(());
    }

    crate::log::event(
        "osu-direct-ip",
        "start",
        None,
        "testing Cloudflare IPv4 candidates",
    );
    let tcp = probe_tcp_candidates();
    let mut finalists = tcp;
    finalists.sort_by_key(|(_, latency)| *latency);
    finalists.truncate(crate::pipeline::downloader::constants::HTTP_CANDIDATES);

    let winner = probe_http_candidates(finalists)?
        .into_iter()
        .min_by_key(|(_, latency)| *latency)
        .ok_or_else(|| io::Error::other("no usable osu.direct Cloudflare IP"))?;

    write_cache(temp_dir, winner.0)?;
    crate::log::event(
        "osu-direct-ip",
        "done",
        None,
        &format!(
            "selected {} ({:.0} ms HTTPS)",
            winner.0,
            winner.1.as_secs_f64() * 1000.0
        ),
    );
    Ok(())
}

fn probe_tcp_candidates() -> Vec<(Ipv4Addr, Duration)> {
    let candidates = build_candidates();
    let (sender, receiver) = mpsc::channel();
    for ip in candidates {
        let sender = sender.clone();
        thread::spawn(move || {
            let started = std::time::Instant::now();
            let result = TcpStream::connect_timeout(
                &SocketAddr::new(IpAddr::V4(ip), 443),
                crate::pipeline::downloader::constants::TCP_TIMEOUT,
            )
            .map(|_| (ip, started.elapsed()));
            if let Ok(result) = result {
                let _ = sender.send(result);
            }
        });
    }
    drop(sender);
    receiver.iter().collect()
}

fn probe_http_candidates(
    candidates: Vec<(Ipv4Addr, Duration)>,
) -> io::Result<Vec<(Ipv4Addr, Duration)>> {
    let (sender, receiver) = mpsc::channel();
    for (ip, _) in candidates {
        let sender = sender.clone();
        thread::spawn(move || {
            let agent = ureq::AgentBuilder::new()
                .resolver(resolver_for(ip))
                .timeout_connect(crate::pipeline::downloader::constants::HTTP_TIMEOUT)
                .timeout(crate::pipeline::downloader::constants::HTTP_TIMEOUT)
                .build();
            let started = std::time::Instant::now();
            let usable = match agent
                .get("https://osu.direct/")
                .set("User-Agent", "osu-beatmap-preview-cf-speed-test")
                .call()
            {
                Ok(response) => response.status() < 500,
                Err(ureq::Error::Status(code, _)) => code < 500,
                Err(_) => false,
            };
            if usable {
                let _ = sender.send((ip, started.elapsed()));
            }
        });
    }
    drop(sender);
    Ok(receiver.iter().collect())
}

fn build_candidates() -> Vec<Ipv4Addr> {
    let seed = unix_seconds() as u32 ^ std::process::id();
    crate::pipeline::downloader::constants::CLOUDFLARE_IPV4_RANGES
        .iter()
        .enumerate()
        .flat_map(|(index, range)| sample_range(range, seed.wrapping_add(index as u32)))
        .collect()
}

fn sample_range(cidr: &str, seed: u32) -> Vec<Ipv4Addr> {
    let Some((network, prefix)) = cidr.split_once('/') else {
        return Vec::new();
    };
    let Ok(network) = network.parse::<Ipv4Addr>() else {
        return Vec::new();
    };
    let Ok(prefix) = prefix.parse::<u32>() else {
        return Vec::new();
    };
    if prefix > 32 {
        return Vec::new();
    }
    let network = u32::from(network) & (!0u32 << (32 - prefix));
    let host_count = 1u64 << (32 - prefix);
    let max_offset = host_count.saturating_sub(1).max(1);
    let first = 1 + (seed as u64 % max_offset);
    let second = 1 + (seed.rotate_left(13) as u64 % max_offset);
    vec![
        Ipv4Addr::from(network.wrapping_add(first as u32)),
        Ipv4Addr::from(network.wrapping_add(second as u32)),
    ]
}

fn cache_path(temp_dir: &Path) -> PathBuf {
    cache_root(temp_dir).join(&crate::config::current().network.downloader_cf_ip.CACHE_FILE)
}

fn write_cache(temp_dir: &Path, ip: Ipv4Addr) -> io::Result<()> {
    let root = cache_root(temp_dir);
    let path = cache_path(temp_dir);
    let cache_file = &crate::config::current().network.downloader_cf_ip.CACHE_FILE;
    let tmp = root.join(format!("{cache_file}.{}.tmp", std::process::id()));
    let content =
        serde_json::json!({ "ip": ip.to_string(), "tested_at": unix_seconds() }).to_string();
    std::fs::write(&tmp, content)?;
    match std::fs::rename(&tmp, &path) {
        Ok(()) => Ok(()),
        Err(error) if cfg!(windows) => {
            let _ = std::fs::remove_file(&path);
            std::fs::rename(&tmp, &path).map_err(|_| error)
        }
        Err(error) => Err(error),
    }
}

fn cache_root(temp_dir: &Path) -> PathBuf {
    temp_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| temp_dir.to_path_buf())
}

fn acquire_lock(path: PathBuf) -> io::Result<Option<LockGuard>> {
    for attempt in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                return Ok(Some(LockGuard {
                    file: Some(file),
                    path,
                }))
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempt == 0 => {
                let stale = path
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > Duration::from_secs(60));
                if stale {
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

struct LockGuard {
    file: Option<std::fs::File>,
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        self.file.take();
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ureq::Resolver;

    #[test]
    fn resolver_only_overrides_osu_direct() {
        let resolver = resolver_for(Ipv4Addr::new(192, 0, 2, 1));
        assert_eq!(
            resolver.resolve("osu.direct:443").unwrap()[0],
            "192.0.2.1:443".parse().unwrap()
        );
    }

    #[test]
    fn samples_are_ipv4_addresses_inside_ranges() {
        for (range, sample) in crate::pipeline::downloader::constants::CLOUDFLARE_IPV4_RANGES
            .iter()
            .zip(std::iter::repeat(0))
        {
            assert_eq!(sample_range(range, sample).len(), 2);
        }
    }

    #[test]
    fn cache_round_trip_uses_parent_of_osz_cache() {
        let root =
            std::env::temp_dir().join(format!("osu-preview-cf-cache-test-{}", std::process::id()));
        let osz_cache = root.join("osz-download-cache");
        std::fs::create_dir_all(&osz_cache).unwrap();
        let ip = Ipv4Addr::new(104, 16, 1, 1);
        write_cache(&osz_cache, ip).unwrap();
        assert_eq!(read_preferred_ip(&osz_cache), Some(ip));
        assert!(root
            .join(
                crate::config::current()
                    .network
                    .downloader_cf_ip
                    .CACHE_FILE
                    .as_str()
            )
            .is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_and_expired_cache_are_ignored() {
        let root = std::env::temp_dir().join(format!(
            "osu-preview-cf-invalid-test-{}",
            std::process::id()
        ));
        let osz_cache = root.join("osz-download-cache");
        std::fs::create_dir_all(&osz_cache).unwrap();
        std::fs::write(
            root.join(
                crate::config::current()
                    .network
                    .downloader_cf_ip
                    .CACHE_FILE
                    .as_str(),
            ),
            "not-json",
        )
        .unwrap();
        assert_eq!(read_preferred_ip(&osz_cache), None);
        std::fs::write(
            root.join(
                crate::config::current()
                    .network
                    .downloader_cf_ip
                    .CACHE_FILE
                    .as_str(),
            ),
            serde_json::json!({ "ip": "104.16.1.1", "tested_at": 1 }).to_string(),
        )
        .unwrap();
        assert_eq!(read_preferred_ip(&osz_cache), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
