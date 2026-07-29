use crate::errors::{PreviewError, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const OSZ_MIRRORS: [&str; 4] = [
    "https://txy1.sayobot.cn/beatmaps/download/novideo/{}",
    "https://mirror.nekoha.moe/api/osz/{}",
    "https://osu.direct/api/d/{}",
    "https://catboy.best/d/{}",
];
const MIB_BYTES: u64 = 1024 * 1024;
const MAX_OSZ_BYTES: u64 = 50 * MIB_BYTES;
const MIRROR_ATTEMPTS: usize = 3;

pub fn download_beatmap_file(bid: &str, temp_dir: &Path, no_cache: bool) -> Result<PathBuf> {
    std::fs::create_dir_all(temp_dir)
        .map_err(|e| PreviewError::download(format!("failed to create cache dir: {e}")))?;
    let target_path = temp_dir.join(format!("{bid}.osu"));
    if !no_cache {
        if let Ok(meta) = target_path.metadata() {
            if meta.is_file() && meta.len() > 0 {
                return Ok(target_path);
            }
        }
    }

    let url = format!("https://osu.ppy.sh/osu/{bid}");
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .build();
    let response = agent
        .get(&url)
        .set("User-Agent", "osu-beatmap-preview/1.0")
        .call();

    let data = match response {
        Ok(resp) => {
            let mut buf = Vec::new();
            resp.into_reader()
                .read_to_end(&mut buf)
                .map_err(|e| PreviewError::download(format!("failed to download beatmap {bid}: {e}")))?;
            buf
        }
        Err(ureq::Error::Status(404, _)) => {
            return Err(PreviewError::download(format!("beatmap not found for bid {bid}")))
        }
        Err(ureq::Error::Status(code, _)) => {
            return Err(PreviewError::download(format!(
                "failed to download beatmap {bid}: http {code}"
            )))
        }
        Err(e) => {
            return Err(PreviewError::download(format!(
                "failed to download beatmap {bid}: {e}"
            )))
        }
    };

    std::fs::write(&target_path, &data)
        .map_err(|e| PreviewError::download(format!("failed to write beatmap cache: {e}")))?;
    Ok(target_path)
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
        return Ok(target_path);
    }

    let part_path = temp_dir.join(format!("{set_id}.osz.part"));
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(120))
        .timeout_write(Duration::from_secs(30))
        .build();
    let result = try_osz_mirrors(set_id, |url, attempt| {
        let _ = std::fs::remove_file(&part_path);
        match download_osz_once(&agent, url, &part_path) {
            Ok(()) => Ok(()),
            Err(reason) => Err(format!("attempt {attempt}/{MIRROR_ATTEMPTS}: {reason}")),
        }
    });
    if let Err(failures) = result {
        let _ = std::fs::remove_file(&part_path);
        return Err(PreviewError::download(format!(
            "failed to download beatmapset {set_id} from all mirrors: {}",
            failures.join("; ")
        )));
    }
    if target_path.exists() {
        std::fs::remove_file(&target_path).map_err(|e| {
            PreviewError::download(format!("failed to replace osz cache: {e}"))
        })?;
    }
    std::fs::rename(&part_path, &target_path).map_err(|e| {
        PreviewError::download(format!("failed to commit osz cache: {e}"))
    })?;
    Ok(target_path)
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
    let response = agent
        .get(url)
        .set("User-Agent", "osu-beatmap-preview/1.0")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("http {code}"),
            other => other.to_string(),
        })?;

    let content_type = response.header("Content-Type").unwrap_or("").to_ascii_lowercase();
    if content_type.contains("text/html") {
        return Err("server returned HTML instead of an osz archive".to_string());
    }
    if let Some(length) = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
    {
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
    }

    let mut reader = response.into_reader().take(MAX_OSZ_BYTES + 1);
    let mut data = Vec::new();
    reader
        .read_to_end(&mut data)
        .map_err(|e| format!("failed to read response: {e}"))?;
    if data.is_empty() {
        return Err("empty response".to_string());
    }
    if data.len() as u64 > MAX_OSZ_BYTES {
        return Err(format!(
            "OSZ is too large while reading the response: received more than \
             {MAX_OSZ_BYTES} bytes ({:.2} MiB); Content-Length was missing or understated",
            MAX_OSZ_BYTES as f64 / MIB_BYTES as f64,
        ));
    }
    std::fs::write(part_path, &data).map_err(|e| format!("failed to write temporary osz: {e}"))?;
    if !valid_osz(part_path) {
        return Err("response is not a valid ZIP/OSZ archive".to_string());
    }
    Ok(())
}

fn valid_osz(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_OSZ_BYTES {
        return false;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    zip::ZipArchive::new(file).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osz_mirror_order_is_stable() {
        assert_eq!(
            OSZ_MIRRORS[0],
            "https://txy1.sayobot.cn/beatmaps/download/novideo/{}"
        );
        assert_eq!(
            OSZ_MIRRORS[1],
            "https://mirror.nekoha.moe/api/osz/{}"
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
        assert!(calls[0].0.starts_with("https://txy1.sayobot.cn/"));
        assert!(calls[3].0.starts_with("https://mirror.nekoha.moe/"));
        assert!(calls[6].0.starts_with("https://osu.direct/"));
        assert_eq!(
            calls.iter().map(|call| call.1).collect::<Vec<_>>(),
            vec![1, 2, 3, 1, 2, 3, 1]
        );
    }
}
