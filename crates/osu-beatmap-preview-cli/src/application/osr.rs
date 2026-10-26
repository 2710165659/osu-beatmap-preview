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
use osu_beatmap_preview_core::{replay::preparation::PreparedReplay, PreviewError, Result};
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
    let timeout = match format.as_str() {
        "png" => config.timeout.PNG_TIMEOUT,
        "gif" => config.timeout.GIF_TIMEOUT,
        _ => config.timeout.MP4_TIMEOUT,
    };
    let deadline = RequestDeadline::new(started, &format, timeout);
    let cache_root = crate::config::resolve_path(&config.paths.CACHE_DIR);
    let t_source = Instant::now();
    let mut source = source_osr::load(&request, &cache_root, &deadline)?;
    let is_taiko = source.document.header.ruleset_id == 1;
    if is_taiko && format == "png" {
        return Err(PreviewError::new(
            "Taiko replay preview supports GIF/MP4; PNG is not supported",
        ));
    }
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
    let prepared = PreparedReplay::prepare(&source.beatmap, &source.document).map(Arc::new);
    logging::record_stage(
        "prepare_replay_ms",
        t_prepare.elapsed().as_secs_f64() * 1000.0,
    );
    let prepared = prepared?;
    if let PreparedReplay::Taiko(p) = prepared.as_ref() {
        source.beatmap = p.beatmap.clone();
    }
    rec.target_mode = Some(i32::from(source.document.header.ruleset_id));
    rec.mods = Some(prepared.mods().settings.tokens.join(","));
    let t_compile = Instant::now();
    let gameplay = prepared.compile(source.document.clone()).map(Arc::new);
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
    let mut plan = RenderPlan::build(request, i32::from(source.document.header.ruleset_id), false)?;
    // Mod 只能来自回放；内部计划仍复用普通导出的速度与音频参数。
    plan.mods = Some(prepared.mods().settings.clone());
    super::selection::select(&source.beatmap, &mut plan)?;
    rec.selected_times_ms = plan.selected_times.clone();
    deadline.check()?;
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
                "type=osr fmt={format} target={} replay={} output_cache=miss output={}",
                if is_taiko { "taiko" } else { "standard" },
                source.identity,
                path.display()
            ),
        );
        // 失败也记录已经花费的渲染时间；错误事件和汇总统一交由外层写入。
        let rendered = (|| {
            if plan.format != OutputFormat::Mp4 {
                // PNG/GIF 在启动音频/背景任务之前分流，只依赖 OSU 和 OSR。
                osu_beatmap_preview_core::config::with_config(
                    crate::config::core_config(),
                    || match prepared.as_ref() {
                        PreparedReplay::Standard(prepared) => {
                            let render = match plan.format {
                                OutputFormat::Png => crate::export::standard::osr::render_png,
                                _ => crate::export::standard::osr::render_gif,
                            };
                            render(
                                &source.beatmap,
                                prepared,
                                &gameplay,
                                &plan,
                                &path,
                                &deadline,
                            )
                        }
                        PreparedReplay::Taiko(prepared) => crate::export::taiko::osr::render_gif(
                            Arc::clone(prepared),
                            Arc::clone(&gameplay),
                            &plan,
                            &path,
                            &deadline,
                        ),
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
                    mode: if is_taiko {
                        crate::export::geometry::GameMode::Taiko
                    } else {
                        crate::export::geometry::GameMode::Standard
                    },
                })?
                .with_replay(Arc::new(ReplayAudio {
                    prepared: prepared.clone(),
                    gameplay: gameplay.clone(),
                }));
                let background = audio.take_background();
                let storyboard = audio.take_storyboard();
                osu_beatmap_preview_core::config::with_config(
                    crate::config::core_config(),
                    || match prepared.as_ref() {
                        PreparedReplay::Standard(prepared) => crate::export::standard::osr::render(
                            &source.beatmap,
                            &prepared,
                            &gameplay,
                            &plan,
                            &path,
                            background,
                            storyboard,
                            audio,
                            &deadline,
                        ),
                        PreparedReplay::Taiko(prepared) => crate::export::taiko::osr::render(
                            Arc::clone(prepared),
                            Arc::clone(&gameplay),
                            &plan,
                            &path,
                            background,
                            storyboard,
                            audio,
                            &deadline,
                        ),
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
            "replay-info": {"player": source.document.header.player, "identity": source.identity, "rulesetId": source.document.header.ruleset_id,
            "state": gameplay.state_at(f64::INFINITY), "recorded-score": source.document.header.total_score,
            "failure-time": gameplay.failure_time, "cache-hit": cached,
            "recorded-result-differences": gameplay.recorded_differences},
        "beatmap-info": {"meta-data": cache::format_section_keys(&source.beatmap.metadata),
            "difficulty": cache::format_section_keys(&source.beatmap.difficulty)}
    }))
}

#[cfg(test)]
mod tests {
    use osu_beatmap_preview_core::render::cpu::modes::standard::constants as standard_constants;

    use super::*;
    use crate::application::request::{RenderRequest, ReplaySource, RequestKind};
    use std::path::{Path, PathBuf};

    #[test]
    fn taiko_replay_gif_exports_without_archive_and_reuses_selected_segments() {
        let _guard = logging::test_guard();
        let root = std::env::temp_dir().join(format!(
            "osu-preview-taiko-replay-gif-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut request = replay_request(&root);
        // 使用同一份原始 Standard 谱面，覆盖 Taiko 回放自动转谱及原始 MD5 绑定。
        let mut bytes =
            include_bytes!("../../../osu-beatmap-preview-web/test/fixtures/replay-classic.osr")
                .to_vec();
        bytes[0] = 1;
        let replay = root.join("taiko.osr");
        std::fs::write(&replay, bytes).unwrap();
        request.kind = RequestKind::Osr {
            source: ReplaySource::File(replay.to_string_lossy().into()),
        };
        request.source.osu_file = Some(
            Path::new(request.source.osu_file.as_ref().unwrap())
                .with_file_name("replay-classic.osu")
                .to_string_lossy()
                .into(),
        );
        request.source.osz_file = Some(root.join("missing.osz").to_string_lossy().into());
        request.view.time_points = [0.0, 0.1, 0.2, 0.3]
            .into_iter()
            .map(osu_beatmap_preview_core::processing::validation::TimePoint::Seconds)
            .collect();
        let result = execute(request.clone()).unwrap();
        assert_eq!(result["replay-info"]["rulesetId"], 1);
        assert_eq!(result["replay-info"]["cache-hit"], false);
        let path = PathBuf::from(result["preview-img"].as_str().unwrap());
        assert_eq!(path.extension().unwrap(), "gif");
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("taiko_"));
        let mut decoder = gif::DecodeOptions::new()
            .read_info(std::io::Cursor::new(std::fs::read(&path).unwrap()))
            .unwrap();
        let layout = crate::export::taiko::animation_render::build_video_animation_layout(
            crate::export::taiko::animation_render::compute_time_range(),
            crate::export::geometry::OutputFormat::Gif,
        );
        let (width, height) = crate::export::taiko::animation_render::gif_canvas_size(&layout);
        assert_eq!(
            (i64::from(decoder.width()), i64::from(decoder.height())),
            (width, height)
        );
        assert!(decoder.read_next_frame().unwrap().is_some());
        assert!(decoder.read_next_frame().unwrap().is_none());
        let cached = execute(request.clone()).unwrap();
        assert_eq!(cached["replay-info"]["cache-hit"], true);
        assert_eq!(cached["preview-img"], result["preview-img"]);
        request.output.format = Some("png".into());
        assert!(execute(request)
            .unwrap_err()
            .to_string()
            .contains("PNG is not supported"));
        let root = root.canonicalize().unwrap();
        assert_eq!(
            root.parent().unwrap(),
            std::env::temp_dir().canonicalize().unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replay_png_exports_snapshots_without_media_and_reuses_cache() {
        let _guard = logging::test_guard();
        let root = std::env::temp_dir().join(format!(
            "osu-preview-replay-png-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut request = replay_request(&root);
        request.output.format = Some("png".into());
        request.output.fps = None;
        request.view.duration_seconds = None;
        // 填满全部行的选点，避免随机补点改变缓存键。
        let config = &crate::config::current().render.standard.png;
        request.view.time_points = (0..config.structure.ROW_COUNT)
            .map(|row| {
                osu_beatmap_preview_core::processing::validation::TimePoint::Seconds(
                    row as f64 * 0.1,
                )
            })
            .collect();
        let result = execute(request.clone()).unwrap();
        assert_eq!(result["replay-info"]["cache-hit"], false);
        let path = PathBuf::from(result["preview-img"].as_str().unwrap());
        assert_eq!(path.extension().unwrap(), "png");
        let image = image::open(&path).unwrap().into_rgba8();
        let (width, height) = crate::export::standard::context::png_canvas_size();
        assert_eq!(
            (image.width(), image.height()),
            (width as u32, height as u32)
        );
        let x = standard_constants::png::sizing().PAGE_MARGIN_LEFT
            + standard_constants::png::sizing().INFO_MARGIN_LEFT;
        let y = standard_constants::png::sizing().PAGE_MARGIN_TOP
            + standard_constants::png::sizing().INFO_MARGIN_TOP;
        let layout = crate::export::standard::context::build_frame_layout(
            crate::export::geometry::OutputFormat::Png,
        );
        let cell_width = layout.frame_width as u32;
        let cell_height = layout.frame_height as u32;
        let step = layout.frame_width
            + standard_constants::png::sizing().INFO_MARGIN_LEFT
            + standard_constants::png::sizing().INFO_MARGIN_RIGHT
            + standard_constants::png::sizing().COLUMN_GAP;
        let first = image::imageops::crop_imm(&image, x as u32, y as u32, cell_width, cell_height)
            .to_image();
        let second =
            image::imageops::crop_imm(&image, (x + step) as u32, y as u32, cell_width, cell_height)
                .to_image();
        assert_ne!(first, second);
        let cached = execute(request).unwrap();
        assert_eq!(cached["replay-info"]["cache-hit"], true);
        assert_eq!(cached["preview-img"], result["preview-img"]);
        let root = root.canonicalize().unwrap();
        assert_eq!(
            root.parent().unwrap(),
            std::env::temp_dir().canonicalize().unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

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

        // 用文件阻挡输出目录，验证进入渲染后失败仍有耗时与错误汇总。
        let blocked = root.join("blocked");
        std::fs::write(&blocked, b"blocked output directory").unwrap();
        bypass.output.output_dir = Some(blocked.to_string_lossy().into());
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
