//! OSR 资源来源：原始字节 MD5 优先，显式不匹配立即停止，不替换成网络谱面。
use super::request::{RequestKind, ValidatedRequest};
use crate::download::osr::Lookup;
use osu_beatmap_preview_core::replay::{compatibility, parse_osr, ReplayDocument};
use osu_beatmap_preview_core::support::timeout::RequestDeadline;
use osu_beatmap_preview_core::{parse_beatmap_bytes, Beatmap, PreviewError, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) struct Source {
    pub document: Arc<ReplayDocument>,
    pub identity: String,
    pub beatmap: Beatmap,
    pub bytes: Vec<u8>,
    pub osz_path: Option<PathBuf>,
    pub osu_path: PathBuf,
    pub osr_path: PathBuf,
}
pub(super) fn load(
    request: &ValidatedRequest,
    cache: &Path,
    deadline: &RequestDeadline,
) -> Result<Source> {
    let RequestKind::Osr { source } = &request.kind else {
        unreachable!()
    };
    let online = if matches!(source, crate::ReplaySource::File(_)) {
        None
    } else {
        Some(crate::download::osr::download(
            source,
            &request.source.bid,
            cache,
            request.execution.no_cache,
            deadline,
        )?)
    };
    let osr_path = match source {
        crate::ReplaySource::File(path) => PathBuf::from(path),
        _ => online.as_ref().expect("在线来源已下载").path.clone(),
    };
    let replay_bytes = read_limited(&osr_path, 64 * 1024 * 1024)?;
    let document = Arc::new(parse_osr(&replay_bytes)?);
    compatibility::select(&document)?;
    let checksum = &document.header.beatmap_md5;
    let identity = format!("{:x}", Sha256::digest(&replay_bytes))[..16].to_string();
    let downloads = Official {
        cache,
        no_cache: request.execution.no_cache,
        deadline,
        known: online.as_ref().map(|r| &r.beatmap),
    };
    let (osu_path, osz_path, bytes) = resolve_resources(request, checksum, &downloads)?;
    let beatmap = parse_beatmap_bytes(&bytes)?;
    compatibility::validate_beatmap(&beatmap)?;
    if let Some(path) = &osz_path {
        validate_archive_audio(path, &beatmap)?;
    }
    deadline.check()?;
    Ok(Source {
        document,
        identity,
        beatmap,
        bytes,
        osz_path,
        osu_path,
        osr_path,
    })
}

fn resolve_resources(
    request: &ValidatedRequest,
    checksum: &str,
    downloads: &impl Downloads,
) -> Result<(PathBuf, Option<PathBuf>, Vec<u8>)> {
    let external = request
        .source
        .osu_file
        .as_ref()
        .map(|path| {
            let bytes = read_limited(Path::new(path), 32 * 1024 * 1024)?;
            check_md5(&bytes, checksum)?;
            Ok::<_, PreviewError>((PathBuf::from(path), bytes))
        })
        .transpose()?;
    // GIF 只消费谱面与回放；即使传入 OSZ，也不读取包或检查歌曲，避免触发媒体下载。
    if request.output.format.as_deref() == Some("gif") {
        let (path, bytes) = match external {
            Some(local) => local,
            None => {
                let lookup = checked_lookup(downloads, checksum)?;
                let path = downloads.beatmap(&lookup)?;
                let bytes = read_limited(&path, 32 * 1024 * 1024)?;
                check_md5(&bytes, checksum)?;
                (path, bytes)
            }
        };
        return Ok((path, None, bytes));
    }
    let (osz_path, archive_bytes) = match request.source.osz_file.as_ref() {
        Some(path) => {
            let path = PathBuf::from(path);
            let bytes = find_md5(&path, checksum)?;
            (path, bytes)
        }
        None => {
            let lookup = checked_lookup(downloads, checksum)?;
            // 未提供 osu 时先取得官方原文，再验证包内同一个 MD5，不以 BeatmapID 代替。
            if external.is_none() {
                let path = downloads.beatmap(&lookup)?;
                check_md5(&read_limited(&path, 32 * 1024 * 1024)?, checksum)?;
            }
            let path = downloads.archive(&lookup)?;
            let bytes = find_md5(&path, checksum)?;
            (path, bytes)
        }
    };
    let (osu_path, bytes) = external.unwrap_or_else(|| (osz_path.clone(), archive_bytes));
    Ok((osu_path, Some(osz_path), bytes))
}

fn checked_lookup(downloads: &impl Downloads, checksum: &str) -> Result<Lookup> {
    let lookup = downloads.lookup(checksum)?;
    if lookup.id == 0
        || lookup.beatmapset_id == 0
        || !lookup.checksum.eq_ignore_ascii_case(checksum)
    {
        return Err(PreviewError::download(
            "official checksum lookup returned a mismatched beatmap",
        ));
    }
    Ok(lookup)
}

/// 每次请求都检查包内歌曲，缓存命中不能掩盖缺失音频；实际解码仍由媒体层完成。
fn validate_archive_audio(path: &Path, beatmap: &Beatmap) -> Result<()> {
    let audio = osu_beatmap_preview_core::processing::media::BeatmapMedia::from_beatmap(beatmap)
        .audio
        .ok_or_else(|| PreviewError::parse("missing AudioFilename required for OSR preview"))?;
    let file = std::fs::File::open(path).map_err(|e| PreviewError::new(e.to_string()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| PreviewError::parse(e.to_string()))?;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| PreviewError::parse(e.to_string()))?;
        if !entry.is_dir()
            && osu_beatmap_preview_core::normalize_entry_path(entry.name())
                .is_some_and(|name| name.eq_ignore_ascii_case(&audio.name))
        {
            let maximum = crate::config::current()
                .advance
                .video_audio
                .MAX_EXTRACTED_AUDIO_BYTES;
            if entry.size() == 0 || entry.size() > maximum {
                return Err(PreviewError::parse(
                    "OSR archive audio is empty or exceeds size limit",
                ));
            }
            return Ok(());
        }
    }
    Err(PreviewError::new(format!(
        "OSR archive is missing AudioFilename '{}'",
        audio.name
    )))
}
fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)
        .map_err(|error| PreviewError::new(format!("cannot read {}: {error}", path.display())))?;
    if file
        .metadata()
        .map_err(|e| PreviewError::new(e.to_string()))?
        .len()
        > limit
    {
        return Err(PreviewError::parse("input file exceeds size limit"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| PreviewError::new(e.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(PreviewError::parse("input file exceeds size limit"));
    }
    Ok(bytes)
}
fn check_md5(bytes: &[u8], expected: &str) -> Result<()> {
    if format!("{:x}", md5::compute(bytes)).eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(PreviewError::new(format!(
            "BeatmapMismatch: raw .osu MD5 does not match OSR {expected}"
        )))
    }
}
fn find_md5(path: &Path, checksum: &str) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)
        .map_err(|e| PreviewError::new(format!("cannot open OSZ: {e}")))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| PreviewError::parse(format!("invalid OSZ: {e}")))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| PreviewError::parse(e.to_string()))?;
        let Some(name) = osu_beatmap_preview_core::normalize_entry_path(entry.name()) else {
            continue;
        };
        if name.contains('/') || !name.to_ascii_lowercase().ends_with(".osu") || entry.is_dir() {
            continue;
        }
        if entry.size() > 32 * 1024 * 1024 {
            return Err(PreviewError::parse("OSZ beatmap exceeds size limit"));
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| PreviewError::parse(e.to_string()))?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(PreviewError::parse("OSZ beatmap exceeds size limit"));
        }
        if check_md5(&bytes, checksum).is_ok() {
            return Ok(bytes);
        }
    }
    Err(PreviewError::new(format!(
        "BeatmapMismatch: no top-level .osu with MD5 {checksum} in {}",
        path.display()
    )))
}

/// 官方成绩来源所需的资源端口；测试通过端口替换真实网络实现。
trait Downloads {
    fn lookup(&self, checksum: &str) -> Result<Lookup>;
    fn beatmap(&self, lookup: &Lookup) -> Result<PathBuf>;
    fn archive(&self, lookup: &Lookup) -> Result<PathBuf>;
}

struct Official<'a> {
    cache: &'a Path,
    no_cache: bool,
    deadline: &'a RequestDeadline,
    known: Option<&'a Lookup>,
}

impl Downloads for Official<'_> {
    fn lookup(&self, checksum: &str) -> Result<Lookup> {
        if let Some(known) = self.known {
            if !known.checksum.eq_ignore_ascii_case(checksum) {
                return Err(PreviewError::download(
                    "BeatmapMismatch: 成绩与回放 MD5 不符",
                ));
            }
            return Ok(known.clone());
        }
        crate::download::osr::lookup(checksum, self.deadline)
    }

    fn beatmap(&self, lookup: &Lookup) -> Result<PathBuf> {
        crate::download::download_beatmap_file(
            &lookup.id.to_string(),
            &self.cache.join("osu-download-cache"),
            self.no_cache,
            self.deadline,
        )
    }

    fn archive(&self, lookup: &Lookup) -> Result<PathBuf> {
        crate::download::download_beatmapset_archive(
            &lookup.id.to_string(),
            lookup.beatmapset_id,
            &self.cache.join("osz-download-cache"),
            self.no_cache,
            self.deadline,
            true,
        )
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
