//! 官方来源端口；测试使用本地假端口，不能依赖实时网络或认证环境变量。
use super::*;
use crate::download::official::Lookup;

pub(super) trait Downloads {
    fn lookup(&self, checksum: &str) -> Result<Lookup>;
    fn beatmap(&self, lookup: &Lookup) -> Result<PathBuf>;
    fn archive(&self, lookup: &Lookup) -> Result<PathBuf>;
}

pub(super) struct Official<'a> {
    pub cache: &'a Path,
    pub no_cache: bool,
    pub deadline: &'a RequestDeadline,
    pub known: Option<&'a Lookup>,
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
        crate::download::official::lookup(checksum, self.deadline)
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
