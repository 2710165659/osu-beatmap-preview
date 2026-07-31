use crate::core::errors::{PreviewError, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::Instant;

pub fn download_beatmap_file(bid: &str, temp_dir: &Path, no_cache: bool) -> Result<PathBuf> {
    std::fs::create_dir_all(temp_dir)
        .map_err(|e| PreviewError::download(format!("failed to create cache dir: {e}")))?;
    let target_path = temp_dir.join(format!("{bid}.osu"));
    if !no_cache {
        if let Ok(meta) = target_path.metadata() {
            if meta.is_file() && meta.len() > 0 {
                crate::log::event(
                    "download-osu",
                    "done",
                    Some(bid),
                    &format!("cache hit ({:.1} KB)", meta.len() as f64 / 1024.0),
                );
                crate::log::record_cache(crate::log::CacheKind::Osu, "hit");
                return Ok(target_path);
            }
        }
    }

    let url = format!("https://osu.ppy.sh/osu/{bid}");
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .build();
    crate::log::event("download-osu", "start", Some(bid), &format!("fetching {url}"));
    let started = Instant::now();
    let response = agent
        .get(&url)
        .set("User-Agent", "osu-beatmap-preview/1.0")
        .call();

    let data = match response {
        Ok(resp) => {
            let mut buf = Vec::new();
            resp.into_reader().read_to_end(&mut buf).map_err(|e| {
                crate::log::event("download-osu", "error", Some(bid), &format!("read failed: {e}"));
                PreviewError::download(format!("failed to download beatmap {bid}: {e}"))
            })?;
            buf
        }
        Err(ureq::Error::Status(404, _)) => {
            crate::log::event("download-osu", "error", Some(bid), &format!("beatmap not found for bid {bid}"));
            return Err(PreviewError::download(format!(
                "beatmap not found for bid {bid}"
            )))
        }
        Err(ureq::Error::Status(code, _)) => {
            crate::log::event("download-osu", "error", Some(bid), &format!("http {code}"));
            return Err(PreviewError::download(format!(
                "failed to download beatmap {bid}: http {code}"
            )))
        }
        Err(e) => {
            crate::log::event("download-osu", "error", Some(bid), &e.to_string());
            return Err(PreviewError::download(format!(
                "failed to download beatmap {bid}: {e}"
            )))
        }
    };
    let ms = started.elapsed().as_secs_f64() * 1000.0;

    std::fs::write(&target_path, &data)
        .map_err(|e| PreviewError::download(format!("failed to write beatmap cache: {e}")))?;
    crate::log::event(
        "download-osu",
        "done",
        Some(bid),
        &format!("downloaded {:.1} KB in {ms:.0} ms", data.len() as f64 / 1024.0),
    );
    crate::log::record_cache(crate::log::CacheKind::Osu, "downloaded");
    Ok(target_path)
}
