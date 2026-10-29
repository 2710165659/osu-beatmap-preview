//! 在线成绩定位和回放下载；选择最高分或最近成绩后不替换为另一条可播放成绩。
use super::{Client, Lookup};
use crate::{ReplayMode, ReplaySource, ScoreSelection};
use osu_beatmap_preview_core::{
    replay::parse_osr, support::timeout::RequestDeadline, PreviewError, Result,
};
use serde::Deserialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

const REPLAY_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Score {
    id: u64,
    user_id: u64,
    beatmap_id: u64,
    ruleset_id: u8,
    has_replay: bool,
    #[serde(default)]
    ended_at: String,
    beatmap: Lookup,
}

pub(crate) struct DownloadedReplay {
    pub path: PathBuf,
    pub beatmap: Lookup,
}

pub(crate) fn download(
    source: &ReplaySource,
    bid: &str,
    cache: &Path,
    no_cache: bool,
    deadline: &RequestDeadline,
) -> Result<DownloadedReplay> {
    let client = Client::new(deadline)?;
    download_with(&client, source, bid, cache, no_cache)
}

fn download_with(
    client: &Client<'_>,
    source: &ReplaySource,
    bid: &str,
    cache: &Path,
    no_cache: bool,
) -> Result<DownloadedReplay> {
    let score = select(client, source, bid)?;
    validate(&score)?;
    let directory = cache.join("osr-download-cache");
    std::fs::create_dir_all(&directory)
        .map_err(|_| PreviewError::download("无法创建回放缓存目录"))?;
    let path = directory.join(format!("{}.osr", score.id));
    if !no_cache {
        let cached = std::fs::File::open(&path).and_then(|file| {
            let mut bytes = Vec::new();
            file.take(REPLAY_LIMIT + 1).read_to_end(&mut bytes)?;
            Ok(bytes)
        });
        if let Ok(bytes) = cached {
            if bytes.len() as u64 <= REPLAY_LIMIT && check_replay(&bytes, &score.beatmap).is_ok() {
                return Ok(DownloadedReplay {
                    path,
                    beatmap: score.beatmap,
                });
            }
        }
    }
    let mut bytes = Vec::new();
    let response = client.response(&format!("scores/{}/download", score.id), &[])?;
    if response
        .header("content-length")
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > REPLAY_LIMIT)
    {
        return Err(PreviewError::download("回放超过 64 MiB"));
    }
    response
        .into_reader()
        .take(REPLAY_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PreviewError::download("官方回放读取失败"))?;
    client.deadline.check()?;
    if bytes.len() as u64 > REPLAY_LIMIT {
        return Err(PreviewError::download("回放超过 64 MiB"));
    }
    check_replay(&bytes, &score.beatmap)?;
    // 完整读取和绑定验证后再替换缓存，不把失败下载当作可用文件。
    static NEXT_WRITE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = NEXT_WRITE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = directory.join(format!(
        "{}.{}.{sequence}.tmp",
        score.id,
        std::process::id()
    ));
    if std::fs::write(&temporary, bytes)
        .and_then(|_| std::fs::rename(&temporary, &path))
        .is_err()
    {
        let _ = std::fs::remove_file(&temporary);
        return Err(PreviewError::download("回放缓存写入失败"));
    }
    Ok(DownloadedReplay {
        path,
        beatmap: score.beatmap,
    })
}

fn check_replay(bytes: &[u8], map: &Lookup) -> Result<()> {
    let document = parse_osr(bytes)?;
    if !document
        .header
        .beatmap_md5
        .eq_ignore_ascii_case(&map.checksum)
    {
        return Err(PreviewError::download(
            "BeatmapMismatch: 下载回放与成绩谱面 MD5 不符",
        ));
    }
    Ok(())
}

fn validate(score: &Score) -> Result<()> {
    if score.id == 0
        || score.beatmap.id != score.beatmap_id
        || score.beatmap.beatmapset_id == 0
        || score.beatmap.checksum.len() != 32
        || !score
            .beatmap
            .checksum
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(PreviewError::download("官方成绩响应缺少有效谱面信息"));
    }
    if !matches!(score.ruleset_id, 0..=2) {
        return Err(PreviewError::download(
            "只支持 Standard、Taiko 和 Catch 成绩",
        ));
    }
    if !score.has_replay {
        return Err(PreviewError::download("选中的成绩没有可下载回放"));
    }
    Ok(())
}

fn select(client: &Client<'_>, source: &ReplaySource, bid: &str) -> Result<Score> {
    match source {
        ReplaySource::Score(id) => {
            let score: Score = client.get(&format!("scores/{id}"), &[])?;
            if score.id.to_string() != *id {
                return Err(PreviewError::download("官方返回的成绩 ID 不符"));
            }
            Ok(score)
        }
        ReplaySource::Player {
            user,
            selection,
            mode,
            include_fails,
        } => {
            #[derive(Deserialize)]
            struct User {
                id: u64,
            }
            // 不指定谱面时不补 mode，让官方按玩家资料的默认模式查询。
            // 指定谱面仍默认 osu，保持原有玩家加谱面来源的行为。
            let mode = mode.or((!bid.is_empty()).then_some(ReplayMode::Osu));
            // 用户名放在 URL 路径时逐字节编码，数字用户名也明确按名称查询。
            let encoded = user
                .as_bytes()
                .iter()
                .map(|b| format!("%{b:02X}"))
                .collect::<String>();
            let suffix = mode
                .map(|mode| format!("/{}", mode.as_str()))
                .unwrap_or_default();
            let user: User = client.get(&format!("users/@{encoded}{suffix}"), &[])?;
            if user.id == 0 {
                return Err(PreviewError::download("官方返回无效玩家 ID"));
            }
            let map_id = if bid.is_empty() {
                None
            } else {
                Some(
                    bid.parse::<u64>()
                        .map_err(|_| PreviewError::download("无效谱面 ID"))?,
                )
            };
            let score = match selection {
                ScoreSelection::Best => {
                    #[derive(Deserialize)]
                    struct Best {
                        score: Score,
                    }
                    client
                        .get::<Best>(
                            &format!("beatmaps/{bid}/scores/users/{}", user.id),
                            &[
                                ("mode", mode.unwrap_or(ReplayMode::Osu).as_str()),
                                ("legacy_only", "0"),
                            ],
                        )?
                        .score
                }
                ScoreSelection::Recent => recent(client, user.id, map_id, mode, *include_fails)?,
            };
            if score.user_id != user.id || map_id.is_some_and(|id| score.beatmap_id != id) {
                return Err(PreviewError::download("官方返回的玩家或谱面 ID 不符"));
            }
            Ok(score)
        }
        ReplaySource::File(_) => unreachable!("本地回放不使用官方成绩接口"),
    }
}

fn recent(
    client: &Client<'_>,
    user: u64,
    bid: Option<u64>,
    mode: Option<ReplayMode>,
    include_fails: bool,
) -> Result<Score> {
    let mut candidates = Vec::new();
    // 未指定谱面时直接取官方最近一条，不扫描其它成绩替代无回放或不兼容的结果。
    let limit = if bid.is_some() { "100" } else { "1" };
    // 近期记录有保留范围；最多扫描 1000 条，查不到时不能回退成最高分或其它谱面。
    for offset in (0..1000).step_by(100) {
        let offset = offset.to_string();
        let mut query = Vec::new();
        if let Some(mode) = mode {
            query.push(("mode", mode.as_str()));
        }
        query.extend([
            ("include_fails", if include_fails { "1" } else { "0" }),
            ("legacy_only", "0"),
            ("limit", limit),
            ("offset", offset.as_str()),
        ]);
        let scores: Vec<serde_json::Value> =
            client.get(&format!("users/{user}/scores/recent"), &query)?;
        let finished = bid.is_none() || scores.len() < 100;
        for value in scores
            .into_iter()
            .filter(|s| bid.is_none_or(|id| s["beatmap_id"].as_u64() == Some(id)))
        {
            candidates.push(
                serde_json::from_value::<Score>(value)
                    .map_err(|_| PreviewError::download("官方近期成绩响应无效"))?,
            );
        }
        if finished {
            break;
        }
    }
    // RFC3339 含不同时区，统一转换为绝对时间后再比较。
    let mut timed = Vec::new();
    for score in candidates {
        let timestamp = time::OffsetDateTime::parse(
            &score.ended_at,
            &time::format_description::well_known::Rfc3339,
        )
        .map_err(|_| PreviewError::download("官方近期成绩时间无效"))?;
        timed.push((timestamp, score.id, score));
    }
    timed
        .into_iter()
        .max_by_key(|(time, id, _)| (*time, *id))
        .map(|(_, _, score)| score)
        .ok_or_else(|| {
            PreviewError::download(if bid.is_some() {
                "近期最多 1000 条记录内没有该谱面的成绩；不会回退到最高分"
            } else {
                "玩家在所选模式下没有符合条件的近期成绩"
            })
        })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
