use crate::errors::{PreviewError, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn download_beatmap_file(bid: &str, temp_dir: &Path, no_cache: bool) -> Result<PathBuf> {
    std::fs::create_dir_all(temp_dir)
        .map_err(|e| PreviewError::new(format!("failed to create cache dir: {e}")))?;
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
                .map_err(|e| PreviewError::new(format!("failed to download beatmap {bid}: {e}")))?;
            buf
        }
        Err(ureq::Error::Status(404, _)) => {
            return Err(PreviewError::new(format!("beatmap not found for bid {bid}")))
        }
        Err(ureq::Error::Status(code, _)) => {
            return Err(PreviewError::new(format!(
                "failed to download beatmap {bid}: http {code}"
            )))
        }
        Err(e) => {
            return Err(PreviewError::new(format!(
                "failed to download beatmap {bid}: {e}"
            )))
        }
    };

    std::fs::write(&target_path, &data)
        .map_err(|e| PreviewError::new(format!("failed to write beatmap cache: {e}")))?;
    Ok(target_path)
}
