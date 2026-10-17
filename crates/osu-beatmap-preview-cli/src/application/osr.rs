//! OSR 应用服务：严格来源绑定、一次规则编译和只读媒体导出。
use super::{
    artifact::ArtifactName,
    output_plan::{OutputFormat, RenderPlan},
    request::ValidatedRequest,
    source_osr,
};
use crate::{
    cache,
    logging::{self, CacheKind, SummaryRecord},
    media::audio::{AudioSourceJob, AudioSourceRequest, OszLocation, ReplayAudio},
};
use osu_beatmap_preview_core::support::timeout::RequestDeadline;
use osu_beatmap_preview_core::{
    gameplay::engine::compile_standard, replay::preparation::standard::prepare_replay,
    PreviewError, Result,
};
use std::{sync::Arc, time::Instant};

pub(super) fn execute(mut request: ValidatedRequest) -> Result<serde_json::Value> {
    let started = Instant::now();
    request.output.format.get_or_insert_with(|| "mp4".into());
    if !request.source.bid.is_empty() {
        logging::set_bid(&request.source.bid);
    }
    let mut rec = SummaryRecord {
        bid: request.source.bid.clone(),
        request_type: Some("osr".into()),
        fmt: request.output.format.clone(),
        target_mode: Some(0),
        time_points: (!request.view.time_points.is_empty()).then(|| {
            request
                .view
                .time_points
                .iter()
                .map(super::legacy::format_time_point)
                .collect()
        }),
        duration_time: request.view.duration_seconds,
        no_cache: request.execution.no_cache,
        ..SummaryRecord::default()
    };
    // 汇总包住资源加载和规则准备，提前失败也要留下唯一一条错误记录。
    let result = execute_inner(request, started, &mut rec);
    rec.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    match &result {
        Ok(_) if rec.status.is_empty() => rec.status = "success".into(),
        Err(error) => {
            rec.status = "error".into();
            rec.error = Some(error.to_string());
            rec.error_kind = Some(format!("{:?}", error.kind()).to_lowercase());
            logging::event("render", "error", None, &error.to_string());
        }
        _ => {}
    }
    logging::write_summary(&rec);
    result
}

fn execute_inner(
    mut request: ValidatedRequest,
    started: Instant,
    rec: &mut SummaryRecord,
) -> Result<serde_json::Value> {
    let format = request.output.format.clone().expect("回放输出格式已补齐");
    let config = crate::config::current();
    let timeout = if format == "gif" {
        config.timeout.GIF_TIMEOUT
    } else {
        config.timeout.MP4_TIMEOUT
    };
    let deadline = RequestDeadline::new(started, &format, timeout);
    let cache_root = crate::config::resolve_path(&config.paths.CACHE_DIR);
    let t_source = Instant::now();
    let source = source_osr::load(&request, &cache_root, &deadline)?;
    logging::record_stage("load_replay_ms", t_source.elapsed().as_secs_f64() * 1000.0);
    // 日志保留谱面 ID，回放身份单独记录；产物命名继续沿用原有规则。
    rec.bid = source
        .beatmap
        .metadata
        .get("BeatmapID")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .to_string();
    logging::set_bid(&rec.bid);
    rec.replay_identity = Some(source.identity.clone());
    rec.osu_bytes = Some(source.bytes.len() as u64);
    super::legacy::fill_beatmap_info(&source.beatmap, rec);
    let t_prepare = Instant::now();
    let prepared = prepare_replay(&source.beatmap, &source.document).map(Arc::new);
    logging::record_stage(
        "prepare_replay_ms",
        t_prepare.elapsed().as_secs_f64() * 1000.0,
    );
    let prepared = prepared?;
    rec.mods = Some(prepared.mods.settings.tokens.join(","));
    let t_compile = Instant::now();
    let gameplay = compile_standard(&prepared, source.document.clone()).map(Arc::new);
    logging::record_stage(
        "compile_replay_ms",
        t_compile.elapsed().as_secs_f64() * 1000.0,
    );
    let gameplay = gameplay?;
    deadline.check()?;
    // 回放身份进入文件名；配置目录 hash 沿用普通请求规则。
    request.source.bid = format!("{}-osr-{}", rec.bid, source.identity);
    let directory = crate::config::output_directory(request.output.output_dir.as_deref())
        .map_err(PreviewError::new)?;
    let mut plan = RenderPlan::build(request, 0, false)?;
    // Mod 只能来自回放；内部计划仍复用普通导出的速度与音频参数。
    plan.mods = Some(prepared.mods.settings.clone());
    let path = directory.join(ArtifactName::from_plan(&plan).as_str());
    let mut sources = vec![source.osr_path.as_path(), source.osu_path.as_path()];
    sources.extend(source.osz_path.as_deref());
    let cached =
        cache::output_cache_hit_for_sources(&path, &sources, &format, plan.no_cache).is_some();
    deadline.check()?;
    if cached {
        rec.status = "cache-hit".into();
        logging::record_cache(CacheKind::Output, "hit");
        logging::event(
            "output-cache-hit",
            "done",
            Some(&rec.bid),
            &format!("serving cached output: {}", path.display()),
        );
    } else {
        logging::record_cache(CacheKind::Output, "miss");
        let t_render = Instant::now();
        logging::event(
            "render",
            "start",
            Some(&rec.bid),
            &format!(
                "type=osr fmt={format} target=standard replay={} output_cache=miss output={}",
                source.identity,
                path.display()
            ),
        );
        // 失败也记录已经花费的渲染时间；错误事件和汇总统一交由外层写入。
        let rendered = (|| {
            if plan.format == OutputFormat::Gif {
                // GIF 在启动音频/背景任务之前分流，整个流程只依赖 OSU 和 OSR。
                osu_beatmap_preview_core::config::with_config(
                    crate::config::core_config(),
                    || {
                        crate::export::standard::osr::render_gif(
                            &source.beatmap,
                            &prepared,
                            &gameplay,
                            &plan,
                            &path,
                            &deadline,
                        )
                    },
                )?;
            } else {
                let mut audio = AudioSourceJob::start(AudioSourceRequest {
                    request_bid: &rec.bid,
                    beatmap: source.beatmap.clone(),
                    osu_text: &String::from_utf8_lossy(&source.bytes),
                    osz: OszLocation::Local(source.osz_path.clone().expect("MP4 必须有谱面包")),
                    cache_dir: cache_root.join("osz-download-cache"),
                    no_cache: plan.no_cache,
                    deadline: deadline.clone(),
                    mode: crate::export::geometry::GameMode::Standard,
                })?
                .with_replay(Arc::new(ReplayAudio {
                    prepared: prepared.clone(),
                    gameplay: gameplay.clone(),
                }));
                let background = audio.take_background();
                let storyboard = audio.take_storyboard();
                osu_beatmap_preview_core::config::with_config(
                    crate::config::core_config(),
                    || {
                        crate::export::standard::osr::render(
                            &source.beatmap,
                            &prepared,
                            &gameplay,
                            &plan,
                            &path,
                            background,
                            storyboard,
                            audio,
                            &deadline,
                        )
                    },
                )?;
            }
            deadline.check()
        })();
        rec.render_ms = Some(t_render.elapsed().as_secs_f64() * 1000.0);
        rendered?;
        logging::event(
            "render",
            "done",
            Some(&rec.bid),
            &format!(
                "type=osr fmt={format} finished in {:.1}s",
                rec.render_ms.unwrap_or(0.0) / 1000.0
            ),
        );
    }
    if let Ok(meta) = path.metadata() {
        logging::record_output_bytes(meta.len());
    }
    let abs = path.canonicalize().unwrap_or(path);
    Ok(serde_json::json!({
        "status": "success", "preview-img": cache::clean_windows_path(&abs.to_string_lossy()),
        "replay-info": {"player": source.document.header.player, "identity": source.identity,
            "state": gameplay.state_at(f64::INFINITY), "recorded-score": source.document.header.total_score,
            "failure-time": gameplay.failure_time, "cache-hit": cached,
            "recorded-result-differences": gameplay.recorded_differences},
        "beatmap-info": {"meta-data": cache::format_section_keys(&source.beatmap.metadata),
            "difficulty": cache::format_section_keys(&source.beatmap.difficulty)}
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::request::{RenderRequest, ReplaySource, RequestKind};
    use osu_beatmap_preview_core::processing::validation::TimePoint;
    use std::path::{Path, PathBuf};

    #[test]
    fn replay_logs_render_cache_bypass_and_failures_once_per_request() {
        let _guard = logging::test_guard();
        let root = std::env::temp_dir().join(format!(
            "osu-preview-replay-log-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        logging::reset_for_tests();
        logging::config::init_for_tests(&root.join("logs"));
        let request = replay_request(&root);
        let result = execute(request.clone()).unwrap();
        assert_eq!(result["replay-info"]["cache-hit"], false);
        let output = PathBuf::from(result["preview-img"].as_str().unwrap());
        let output_bytes = output.metadata().unwrap().len();
        let cached = execute(request.clone()).unwrap();
        assert_eq!(cached["replay-info"]["cache-hit"], true);

        let mut bypass = request.clone();
        bypass.execution.no_cache = true;
        let result = execute(bypass.clone()).unwrap();
        assert_eq!(result["replay-info"]["cache-hit"], false);

        // 五个显式时间点超过默认四格容量，验证进入渲染后失败仍有耗时与错误汇总。
        bypass.view.time_points = (0..5)
            .map(|value| TimePoint::Seconds(value as f64))
            .collect();
        assert!(execute(bypass).is_err());
        let mut bad_source = request.clone();
        let invalid_replay = root.join("invalid.osr");
        std::fs::write(&invalid_replay, b"invalid replay").unwrap();
        bad_source.kind = RequestKind::Osr {
            source: ReplaySource::File(invalid_replay.to_string_lossy().into()),
        };
        // 未指定格式时默认 MP4，资源加载失败也必须保留这一实际格式。
        bad_source.output.format = None;
        assert!(execute(bad_source).is_err());

        let records: Vec<serde_json::Value> = std::fs::read_to_string(root.join("logs/render.log"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 5);
        for (record, status) in
            records
                .iter()
                .zip(["success", "cache-hit", "success", "error", "error"])
        {
            assert_eq!(record["status"], status);
            assert_eq!(record["request_type"], "osr");
            assert!(record["duration_ms"].as_f64().unwrap() >= 0.0);
        }
        for record in &records[..4] {
            assert_eq!(record["bid"], "42");
            assert_eq!(record["fmt"], "gif");
            assert_eq!(record["replay_identity"], result["replay-info"]["identity"]);
            assert_eq!(record["title"], "Replay fixture");
            assert!(record["prepare_replay_ms"].is_number());
            assert!(record["compile_replay_ms"].is_number());
        }
        assert_eq!(records[0]["cache"]["output"], "miss");
        assert_eq!(records[0]["output_bytes"], output_bytes);
        assert!(records[0]["render_ms"].is_number());
        assert_eq!(records[1]["cache"]["output"], "hit");
        assert_eq!(records[1]["output_bytes"], output_bytes);
        assert!(records[1].get("render_ms").is_none());
        assert_eq!(records[2]["cache"]["output"], "miss");
        assert_eq!(records[2]["no_cache"], true);
        assert!(records[3]["render_ms"].is_number());
        assert!(records[3]["error"].is_string());
        assert_eq!(records[4]["fmt"], "mp4");
        assert!(records[4]["error_kind"].is_string());
        let progress = std::fs::read_to_string(root.join("logs/progress.log")).unwrap();
        assert_eq!(progress.matches("step=render status=start").count(), 3);
        assert_eq!(progress.matches("step=render status=done").count(), 2);
        assert_eq!(progress.matches("step=render status=error").count(), 2);
        assert_eq!(
            progress
                .matches("step=output-cache-hit status=done")
                .count(),
            1
        );

        // 模拟公共入口的 --no-log 分支：不初始化日志时，命中缓存也不能追加文件。
        logging::reset_for_tests();
        let before = std::fs::read(root.join("logs/render.log")).unwrap();
        execute(request).unwrap();
        assert_eq!(std::fs::read(root.join("logs/render.log")).unwrap(), before);
        assert_eq!(
            std::fs::read_to_string(root.join("logs/progress.log")).unwrap(),
            progress
        );
        logging::reset_for_tests();
        let root = root.canonicalize().unwrap();
        assert_eq!(
            root.parent().unwrap(),
            std::env::temp_dir().canonicalize().unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn replay_request(output: &Path) -> ValidatedRequest {
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../osu-beatmap-preview-web/test/fixtures");
        let mut request = RenderRequest::new("");
        request.kind = RequestKind::Osr {
            source: ReplaySource::File(
                fixtures
                    .join("replay-standard.osr")
                    .to_string_lossy()
                    .into(),
            ),
        };
        request.source.osu_file = Some(
            fixtures
                .join("replay-standard.osu")
                .to_string_lossy()
                .into(),
        );
        request.output.format = Some("gif".into());
        request.output.fps = Some(1);
        request.output.output_dir = Some(output.join("outputs").to_string_lossy().into());
        request.view.duration_seconds = Some(0.05);
        request.validate().unwrap()
    }
}
