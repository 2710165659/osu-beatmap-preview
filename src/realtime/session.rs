//! 谱面、资源、配置快照和四模式预计算数据组成的可共享会话。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::application::engine::legacy::{
    convert_beatmap, object_time_bounds, resolve_convert_target,
};
use crate::domain::shared::time_selection::TimeAxis;
use crate::domain::timeout::RequestDeadline;
use crate::infrastructure::config::{load_session_snapshot, with_session_snapshot, ConfigSnapshot};
use crate::infrastructure::media::audio::{load_background_image, AudioSourceJob};
use crate::render::canvas::Img;
use crate::render::geometry::GameMode;
use crate::render::wgpu::RealtimeFrameSource;

use super::{
    FrameScene, MediaPolicy, OffscreenConfig, RealtimeError, RealtimeMode, RealtimeRequest,
    TimelineInfo,
};

#[derive(Clone)]
pub struct RealtimeSession {
    inner: Arc<SessionInner>,
}

struct SessionInner {
    source: RealtimeFrameSource,
    snapshot: ConfigSnapshot,
    timeline: TimelineInfo,
    offscreen: OffscreenConfig,
    background: Option<Arc<Img>>,
    audio_path: Option<PathBuf>,
}

impl std::fmt::Debug for RealtimeSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RealtimeSession")
            .field("mode", &self.mode())
            .field("timeline", &self.timeline())
            .field("offscreen", &self.offscreen_config())
            .field("audio_path", &self.audio_path())
            .finish()
    }
}

impl RealtimeSession {
    pub fn load(request: RealtimeRequest) -> Result<Self, RealtimeError> {
        let mut render_request = crate::RenderRequest::new(&request.bid);
        render_request.ruleset.convert = request.convert.clone();
        render_request.ruleset.mods = request.mods.clone();
        render_request.output.format = Some("mp4".to_string());
        render_request.output.scale = request.scale;
        render_request.execution.config = request.config.clone();
        render_request.execution.no_cache = request.no_cache;
        render_request.execution.logging = false;
        let validated = render_request
            .validate()
            .map_err(|error| RealtimeError::InvalidRequest(error.to_string()))?;
        let snapshot = load_session_snapshot(request.config.as_deref(), request.scale)
            .map_err(RealtimeError::InvalidRequest)?;
        let deadline = RequestDeadline::new(
            Instant::now(),
            "realtime",
            snapshot.runtime().timeout.MP4_TIMEOUT,
        );
        let cache_root = crate::infrastructure::config::resolve_path(
            snapshot.runtime().paths.CACHE_DIR.as_str(),
        );
        let beatmap_path = crate::infrastructure::download::download_beatmap_file(
            &request.bid,
            &cache_root.join("osu-download-cache"),
            request.no_cache,
            &deadline,
        )?;
        let mut beatmap = crate::domain::parser::parse_beatmap(&beatmap_path)?;
        let target_mode = match validated.ruleset.convert.as_deref() {
            Some(name) => resolve_convert_target(&beatmap, name)?,
            None => beatmap.mode(),
        };
        if target_mode != beatmap.mode() {
            beatmap = convert_beatmap(&beatmap, target_mode, validated.ruleset.mods.as_ref())?;
        }
        let (first_object_ms, last_object_ms) = object_time_bounds(&beatmap.hit_objects)
            .ok_or_else(|| RealtimeError::Load("beatmap has no hit objects".to_string()))?;
        let speed = validated
            .ruleset
            .mods
            .as_ref()
            .map(|mods| mods.speed_multiplier)
            .unwrap_or(1.0);
        let absolute_start_ms = crate::infrastructure::media::audio::full_video_start_time(
            first_object_ms,
            beatmap.audio_lead_in_ms(),
        );
        let timeline = TimelineInfo {
            absolute_start_ms,
            first_object_ms,
            last_object_ms,
            duration_ms: last_object_ms.saturating_sub(first_object_ms),
            beatmap_speed: speed,
        };
        let offscreen = offscreen_config(snapshot.runtime(), target_mode);
        let (background, audio_path) = with_session_snapshot(&snapshot, || {
            load_media(
                request.media_policy,
                &request.bid,
                &beatmap,
                &cache_root,
                request.no_cache,
                &deadline,
                game_mode(target_mode),
            )
        })?;
        let background = with_session_snapshot(&snapshot, || {
            background.as_ref().map(|image| {
                Arc::new(crate::infrastructure::media::prepare_wgpu_video_background(
                    image,
                    offscreen.width,
                    offscreen.height,
                    crate::infrastructure::media::video_style(game_mode(target_mode)),
                    game_mode(target_mode),
                ))
            })
        });
        let time_axis = TimeAxis::new(first_object_ms);
        let source = with_session_snapshot(&snapshot, || match target_mode {
            0 => crate::render::wgpu::modes::prepare_standard(
                &beatmap,
                validated.ruleset.mods.as_ref(),
                time_axis,
            ),
            1 => {
                crate::render::wgpu::modes::prepare_taiko(&beatmap, validated.ruleset.mods.as_ref())
            }
            2 => {
                crate::render::wgpu::modes::prepare_catch(&beatmap, validated.ruleset.mods.as_ref())
            }
            3 => {
                crate::render::wgpu::modes::prepare_mania(&beatmap, validated.ruleset.mods.as_ref())
            }
            _ => unreachable!("谱面模式已经由解析器校验"),
        })?;
        Ok(Self {
            inner: Arc::new(SessionInner {
                source,
                snapshot,
                timeline,
                offscreen,
                background,
                audio_path,
            }),
        })
    }

    pub fn scene_at_absolute(&self, absolute_time_ms: i64) -> Result<FrameScene, RealtimeError> {
        with_session_snapshot(&self.inner.snapshot, || {
            let playfield = self.inner.source.render(absolute_time_ms)?;
            crate::render::wgpu::composition::compose_video_scene(
                playfield,
                absolute_time_ms.saturating_sub(self.inner.timeline.first_object_ms),
                self.inner.timeline.duration_ms,
                self.inner.offscreen.width,
                self.inner.offscreen.height,
                self.inner.background.as_ref(),
                crate::infrastructure::media::video_style(self.inner.source.mode),
                self.inner.source.mode,
            )
        })
    }

    pub fn scene_at_gameplay(&self, gameplay_time_ms: i64) -> Result<FrameScene, RealtimeError> {
        let absolute = self
            .inner
            .timeline
            .first_object_ms
            .checked_add(gameplay_time_ms)
            .ok_or_else(|| {
                RealtimeError::InvalidRequest(
                    "gameplay time is outside the supported range".to_string(),
                )
            })?;
        self.scene_at_absolute(absolute)
    }

    pub fn timeline(&self) -> TimelineInfo {
        self.inner.timeline
    }

    pub fn duration_ms(&self) -> i64 {
        self.inner.timeline.duration_ms
    }

    pub fn audio_path(&self) -> Option<&Path> {
        self.inner.audio_path.as_deref()
    }

    pub fn mode(&self) -> RealtimeMode {
        match self.inner.source.mode {
            GameMode::Standard => RealtimeMode::Standard,
            GameMode::Taiko => RealtimeMode::Taiko,
            GameMode::Catch => RealtimeMode::Catch,
            GameMode::Mania => RealtimeMode::Mania,
        }
    }

    pub fn offscreen_config(&self) -> OffscreenConfig {
        self.inner.offscreen
    }
}

fn game_mode(mode: i32) -> GameMode {
    match mode {
        0 => GameMode::Standard,
        1 => GameMode::Taiko,
        2 => GameMode::Catch,
        3 => GameMode::Mania,
        _ => unreachable!("谱面模式已经由解析器校验"),
    }
}

fn offscreen_config(
    runtime: &crate::infrastructure::config::RuntimeConfig,
    _mode: i32,
) -> OffscreenConfig {
    crate::infrastructure::config::wgpu_config(runtime)
}

fn load_media(
    policy: MediaPolicy,
    bid: &str,
    beatmap: &crate::domain::models::Beatmap,
    cache_root: &Path,
    no_cache: bool,
    deadline: &RequestDeadline,
    mode: GameMode,
) -> Result<(Option<Img>, Option<PathBuf>), RealtimeError> {
    if policy == MediaPolicy::None {
        return Ok((None, None));
    }
    let cache = cache_root.join("osz-download-cache");
    if policy == MediaPolicy::AudioAndBackground {
        let mut job = AudioSourceJob::start(
            bid,
            beatmap.clone(),
            cache,
            no_cache,
            deadline.clone(),
            mode,
        )?;
        let background = job.take_background();
        let audio = job.wait()?;
        return Ok((background, Some(audio.path)));
    }
    let set_id = beatmap.beatmap_set_id().ok_or_else(|| {
        RealtimeError::Load("missing BeatmapSetID required for background".to_string())
    })?;
    let archive = crate::infrastructure::download::download_beatmapset_archive(
        bid, set_id, &cache, no_cache, deadline,
    )?;
    let background =
        load_background_image(beatmap.background_filename.as_deref(), &archive, deadline)?;
    Ok((background, None))
}
