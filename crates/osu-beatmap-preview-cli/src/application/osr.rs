//! OSR 应用服务：严格来源绑定、一次规则编译和只读媒体导出。
use super::{
    artifact::ArtifactName,
    output_plan::{OutputFormat, RenderPlan},
    request::ValidatedRequest,
    source_osr,
};
use crate::{
    cache,
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
    let format = request
        .output
        .format
        .get_or_insert_with(|| "mp4".into())
        .clone();
    let config = crate::config::current();
    let timeout = if format == "gif" {
        config.timeout.GIF_TIMEOUT
    } else {
        config.timeout.MP4_TIMEOUT
    };
    let deadline = RequestDeadline::new(started, &format, timeout);
    let cache_root = crate::config::resolve_path(&config.paths.CACHE_DIR);
    let source = source_osr::load(&request, &cache_root, &deadline)?;
    let prepared = Arc::new(prepare_replay(&source.beatmap, &source.document)?);
    let gameplay = Arc::new(compile_standard(&prepared, source.document.clone())?);
    deadline.check()?;
    // 回放身份进入文件名；配置目录 hash 沿用普通请求规则。
    request.source.bid = format!(
        "{}-osr-{}",
        source
            .beatmap
            .metadata
            .get("BeatmapID")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0),
        source.identity
    );
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
    if !cached && plan.format == OutputFormat::Gif {
        // GIF 在启动音频/背景任务之前分流，整个流程只依赖 OSU 和 OSR。
        osu_beatmap_preview_core::config::with_config(crate::config::core_config(), || {
            crate::export::standard::osr::render_gif(
                &source.beatmap,
                &prepared,
                &gameplay,
                &plan,
                &path,
                &deadline,
            )
        })?;
    } else if !cached {
        let mut audio = AudioSourceJob::start(AudioSourceRequest {
            request_bid: &plan.bid,
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
        osu_beatmap_preview_core::config::with_config(crate::config::core_config(), || {
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
        })?;
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
