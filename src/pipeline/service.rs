use crate::common::time_selection::{GifRenderOptions, TimeAxis};
use crate::core::errors::{PreviewError, Result};
use crate::core::models::{Beatmap, HitObjects};
use crate::core::mods::ModSettings;
use crate::core::timeout::RequestDeadline;
use crate::core::validate::{self, TimePoint, ValidateContext};
use crate::log::{self, CacheKind, SummaryRecord};
use crate::pipeline::cache;
use crate::render::canvas::Img;
use crate::render::video::audio::AudioSourceJob;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub fn generate_preview(
    bid: &str,
    fmt: Option<&str>,
    convert: Option<&str>,
    mods: Option<ModSettings>,
    time_points: Vec<TimePoint>,
    duration_time: Option<f64>,
    no_cache: bool,
) -> Result<Value> {
    let started = Instant::now();
    log::set_bid(bid);
    let deadline = initial_deadline(started, fmt, convert);
    let mut rec = SummaryRecord {
        bid: bid.to_string(),
        fmt: fmt.map(str::to_string),
        convert: convert.map(str::to_string),
        time_points: (!time_points.is_empty())
            .then(|| time_points.iter().map(format_time_point).collect()),
        duration_time,
        no_cache,
        ..SummaryRecord::default()
    };
    let result = match generate_preview_inner(
        bid,
        fmt,
        convert,
        mods,
        time_points,
        duration_time,
        no_cache,
        started,
        deadline,
        &mut rec,
    ) {
        Ok(value) => {
            rec.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
            if rec.status.is_empty() {
                rec.status = "success".to_string();
            }
            log::write_summary(&rec);
            Ok(value)
        }
        Err(error) => {
            rec.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
            rec.status = "error".to_string();
            rec.error = Some(error.to_string());
            rec.error_kind = Some(format!("{:?}", error.kind()).to_lowercase());
            log::event("render", "error", Some(bid), &error.to_string());
            log::write_summary(&rec);
            Err(error)
        }
    };
    result
}

fn generate_preview_inner(
    bid: &str,
    fmt: Option<&str>,
    convert: Option<&str>,
    mods: Option<ModSettings>,
    time_points: Vec<TimePoint>,
    duration_time: Option<f64>,
    no_cache: bool,
    request_started: Instant,
    mut deadline: RequestDeadline,
    rec: &mut SummaryRecord,
) -> Result<Value> {
    deadline.check()?;
    let runtime_config = crate::config::current();
    let cache_root = crate::config::resolve_path(runtime_config.paths.CACHE_DIR.as_str());
    let output_root = crate::config::output_directory().map_err(PreviewError::new)?;
    // ── .osu 下载与解析 ──
    let t0 = Instant::now();
    let beatmap_path = crate::pipeline::downloader::download_beatmap_file(
        bid,
        &cache_root.join("osu-download-cache"),
        no_cache,
        &deadline,
    )?;
    deadline.check()?;
    rec.download_osu_ms = Some(t0.elapsed().as_secs_f64() * 1000.0);
    rec.osu_bytes = beatmap_path.metadata().ok().map(|meta| meta.len());

    let t1 = Instant::now();
    let mut beatmap = crate::parser::parse_beatmap(&beatmap_path)?;
    rec.parse_ms = Some(t1.elapsed().as_secs_f64() * 1000.0);

    if fmt == Some("mp4") && beatmap.beatmap_set_id().is_none() {
        let set_id = crate::pipeline::downloader::resolve_beatmap_set_id(bid, &deadline)?;
        beatmap.metadata.insert("BeatmapSetID", set_id.to_string());
    }
    fill_beatmap_info(&beatmap, rec);

    let mut target_mode = beatmap.mode();
    let mut convert_used: Option<&str> = None;
    if let Some(convert_name) = convert {
        let mode = resolve_convert_target(&beatmap, convert_name)?;
        if mode != beatmap.mode() {
            target_mode = mode;
            convert_used = Some(convert_name);
            rec.convert = Some(convert_name.to_string());
        }
    }
    rec.target_mode = Some(target_mode);

    let fmt: String = match fmt {
        Some(f) => f.to_string(),
        None => {
            if target_mode == 0 {
                "gif".to_string()
            } else {
                "png".to_string()
            }
        }
    };
    deadline = deadline_for_format(request_started, &fmt);
    deadline.check()?;
    rec.fmt = Some(fmt.clone());

    let ctx = ValidateContext {
        bid,
        fmt: &fmt,
        target_mode,
    };
    let mods = validate::validate_with_context(&ctx, &time_points, duration_time, mods)?;
    deadline.check()?;
    rec.mods = mods.as_ref().map(|m| m.tokens.join(","));

    let mode_name = match target_mode {
        0 => "standard",
        1 => "taiko",
        2 => "catch",
        3 => "mania",
        _ => "unknown",
    };

    let mut parts: Vec<String> = vec![mode_name.to_string(), bid.to_string()];
    if convert_used.is_some() {
        parts.push("convert".to_string());
    }
    if let Some(m) = &mods {
        if m.has_any_mod() {
            parts.push(cache::format_mod_suffix(m));
        }
    }
    if fmt == "mp4" {
        if has_explicit_video_time_options(&time_points, duration_time) {
            parts.push(cache::format_video_time_suffix(
                time_points.first().copied(),
                duration_time,
            ));
        }
    } else {
        if !time_points.is_empty() {
            parts.push(cache::format_time_points_suffix(&time_points));
        }
        if fmt == "gif" {
            if let Some(duration) = duration_time {
                parts.push(cache::format_duration_suffix(duration));
            }
        }
    }
    let output_scale = crate::render::geometry::output_scale(
        match target_mode {
            0 => crate::render::geometry::GameMode::Standard,
            1 => crate::render::geometry::GameMode::Taiko,
            2 => crate::render::geometry::GameMode::Catch,
            3 => crate::render::geometry::GameMode::Mania,
            _ => unreachable!("target mode was validated above"),
        },
        match fmt.as_str() {
            "png" => crate::render::geometry::OutputFormat::Png,
            "gif" => crate::render::geometry::OutputFormat::Gif,
            "mp4" => crate::render::geometry::OutputFormat::Mp4,
            _ => unreachable!("format was validated above"),
        },
    );
    let scale_suffix = cache::format_scale_suffix(output_scale).unwrap_or_default();
    // 命令行倍率只改变本次渲染内容，不改变输出文件位置；带倍率的请求跳过旧缓存，
    // 输出文件名后缀用于区分不同倍率，但不会改变输出目录（尤其不会产生新的配置哈希目录）。
    let output_path: PathBuf =
        output_root.join(format!("{}{}.{}", parts.join("_"), scale_suffix, fmt));

    // ── 图像缓存检查 ──
    let cached = cache::output_cache_hit(&output_path, &beatmap_path, &fmt, target_mode, no_cache);
    if let Some(cached_path) = cached {
        deadline.check()?;
        rec.status = "cache-hit".to_string();
        log::record_cache(CacheKind::Output, "hit");
        if let Ok(meta) = cached_path.metadata() {
            log::record_output_bytes(meta.len());
        }
        log::event(
            "output-cache-hit",
            "done",
            Some(bid),
            &format!("serving cached output: {}", cached_path.display()),
        );
        let abs = cached_path.canonicalize().unwrap_or(cached_path.clone());
        let abs_str = cache::clean_windows_path(&abs.to_string_lossy());
        return Ok(json!({
            "status": "success",
            "msg": format!("preview generated successfully for bid {bid}"),
            "preview-img": abs_str,
            "beatmap-info": {
                "meta-data": cache::format_section_keys(&beatmap.metadata),
                "difficulty": cache::format_section_keys(&beatmap.difficulty),
            },
        }));
    }
    log::record_cache(CacheKind::Output, "miss");

    let renderer: &dyn ModeRenderer = match target_mode {
        0 => &StandardRenderer,
        1 => &TaikoRenderer,
        2 => &CatchRenderer,
        3 => &ManiaRenderer,
        _ => {
            return Err(PreviewError::new(format!(
                "unsupported beatmap mode: {target_mode}"
            )))
        }
    };

    let audio_job = if fmt == "mp4" {
        Some(AudioSourceJob::start(
            bid,
            beatmap.clone(),
            cache_root.join("osz-download-cache"),
            no_cache,
            deadline.clone(),
            match target_mode {
                0 => crate::render::geometry::GameMode::Standard,
                1 => crate::render::geometry::GameMode::Taiko,
                2 => crate::render::geometry::GameMode::Catch,
                3 => crate::render::geometry::GameMode::Mania,
                _ => unreachable!("target mode was validated above"),
            },
        )?)
    } else {
        None
    };

    let t_render = Instant::now();
    log::event(
        "render",
        "start",
        Some(bid),
        &format!(
            "fmt={fmt} target={mode_name} output={}",
            output_path.display()
        ),
    );
    // 原子写入保证失败的渲染不会触碰最终路径，因此已有的有效缓存仍会保留；
    // 临时文件（若存在）由写入器负责清理。
    let preview_path = render_preview_for_mode(
        renderer,
        beatmap.clone(),
        &output_path,
        &fmt,
        target_mode,
        mods,
        time_points,
        duration_time,
        audio_job,
        bid,
        &deadline,
    )?;
    deadline.check()?;
    rec.render_ms = Some(t_render.elapsed().as_secs_f64() * 1000.0);
    if let Ok(meta) = preview_path.metadata() {
        log::record_output_bytes(meta.len());
    }
    log::event(
        "render",
        "done",
        Some(bid),
        &format!(
            "fmt={fmt} finished in {:.1}s",
            rec.render_ms.unwrap_or(0.0) / 1000.0
        ),
    );

    let abs = preview_path.canonicalize().unwrap_or(preview_path.clone());
    let abs_str = cache::clean_windows_path(&abs.to_string_lossy());

    Ok(json!({
        "status": "success",
        "msg": format!("preview generated successfully for bid {bid}"),
        "preview-img": abs_str,
        "beatmap-info": {
            "meta-data": cache::format_section_keys(&beatmap.metadata),
            "difficulty": cache::format_section_keys(&beatmap.difficulty),
        },
    }))
}

/// 把解析出的谱面信息填充进汇总记录。
fn fill_beatmap_info(beatmap: &Beatmap, rec: &mut SummaryRecord) {
    rec.mode = Some(beatmap.mode());
    rec.format_version = Some(beatmap.format_version());
    rec.set_id = beatmap.beatmap_set_id();
    rec.title = beatmap.metadata.get("Title").map(str::to_string);
    rec.artist = beatmap.metadata.get("Artist").map(str::to_string);
    rec.version = beatmap.metadata.get("Version").map(str::to_string);
    rec.hit_object_count = Some(beatmap.hit_objects.len());
    rec.chart_duration_ms =
        object_time_bounds(&beatmap.hit_objects).map(|(first, last)| (last - first).max(0));
    rec.bpm = main_bpm(beatmap);
    rec.ar = beatmap.difficulty.get_f64("ApproachRate");
    rec.cs = beatmap.difficulty.get_f64("CircleSize");
    rec.hp = beatmap.difficulty.get_f64("HPDrainRate");
    rec.od = beatmap.difficulty.get_f64("OverallDifficulty");
}

/// 谱面首尾音符的 (开始时间, 结束时间)，用于计算谱面时长。
fn object_time_bounds(hit_objects: &HitObjects) -> Option<(i64, i64)> {
    let mut first = i64::MAX;
    let mut last = i64::MIN;
    let mut any = false;
    match hit_objects {
        HitObjects::Standard(objects) => {
            for o in objects {
                any = true;
                first = first.min(o.start_time);
                last = last.max(o.end_time);
            }
        }
        HitObjects::Taiko(objects) => {
            for o in objects {
                any = true;
                first = first.min(o.start_time);
                last = last.max(o.end_time);
            }
        }
        HitObjects::Catch(objects) => {
            for o in objects {
                any = true;
                first = first.min(o.start_time);
                last = last.max(o.end_time);
            }
        }
        HitObjects::Mania(objects) => {
            for o in objects {
                any = true;
                first = first.min(o.start_time);
                last = last.max(o.end_time);
            }
        }
    }
    any.then_some((first, last))
}

fn object_time_axis(hit_objects: &HitObjects) -> Option<(TimeAxis, i64, i64)> {
    object_time_bounds(hit_objects).map(|(first, last)| (TimeAxis::new(first), first, last))
}

/// 谱面主 BPM（第一个未继承 timing point），保留两位小数。
fn main_bpm(beatmap: &Beatmap) -> Option<f64> {
    beatmap
        .timing_points
        .iter()
        .find(|t| t.uninherited && t.beat_length > 0.0)
        .map(|t| (60000.0 / t.beat_length * 100.0).round() / 100.0)
}

// ── 模式渲染器 trait ──

trait ModeRenderer {
    /// 将 GIF 动画渲染到 `output_path`，并返回输出路径。
    fn render_gif(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        options: GifRenderOptions,
        _time_axis: TimeAxis,
        output_path: &Path,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf>;

    /// 将静态 PNG 渲染到 `output_path`，并返回输出路径。
    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        _time_axis: TimeAxis,
        _times_ms: Option<Vec<i64>>,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf>;

    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        time_point: Option<TimePoint>,
        duration_time: Option<f64>,
        output_path: &Path,
        background: Option<Img>,
        audio_job: AudioSourceJob,
        _time_axis: TimeAxis,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf>;

    /// 在渲染前按需转换谱面。默认行为是克隆谱面（不转换）。
    fn convert(
        &self,
        beatmap: &Beatmap,
        _target_mode: i32,
        _mods: Option<&ModSettings>,
    ) -> Result<Beatmap> {
        Ok(beatmap.clone())
    }

    /// 校验谱面是否包含音符对象。默认实现接受任意谱面。
    fn validate(&self, _beatmap: &Beatmap) -> Result<()> {
        Ok(())
    }
}

// ── 模式实现 ──

struct StandardRenderer;
impl ModeRenderer for StandardRenderer {
    fn validate(&self, beatmap: &Beatmap) -> Result<()> {
        if !matches!(&beatmap.hit_objects, HitObjects::Standard(v) if !v.is_empty()) {
            return Err(PreviewError::render("standard beatmap has no hit objects"));
        }
        Ok(())
    }

    fn render_gif(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        options: GifRenderOptions,
        _time_axis: TimeAxis,
        output_path: &Path,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf> {
        crate::render::standard::render_standard_gif(
            beatmap,
            mods,
            options,
            output_path,
            deadline,
        )?;
        Ok(output_path.to_path_buf())
    }

    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        time_axis: TimeAxis,
        times_ms: Option<Vec<i64>>,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf> {
        let image = crate::render::standard::render_standard_png(
            beatmap, mods, time_axis, times_ms, deadline,
        )?;
        crate::render::composer::save_png(&image, output_path, deadline)?;
        Ok(output_path.to_path_buf())
    }

    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        time_point: Option<TimePoint>,
        duration_time: Option<f64>,
        output_path: &Path,
        background: Option<Img>,
        audio_job: AudioSourceJob,
        time_axis: TimeAxis,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf> {
        crate::render::standard::render_standard_video(
            beatmap,
            mods,
            time_point,
            duration_time,
            output_path,
            background,
            audio_job,
            time_axis,
            deadline,
        )?;
        Ok(output_path.to_path_buf())
    }
}

struct TaikoRenderer;
impl ModeRenderer for TaikoRenderer {
    fn convert(
        &self,
        beatmap: &Beatmap,
        target_mode: i32,
        mods: Option<&ModSettings>,
    ) -> Result<Beatmap> {
        convert_if_needed(beatmap, 1, target_mode, mods)
    }

    fn render_gif(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        options: GifRenderOptions,
        _time_axis: TimeAxis,
        output_path: &Path,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf> {
        crate::render::taiko::render_taiko_gif(beatmap, mods, options, output_path, deadline)?;
        Ok(output_path.to_path_buf())
    }

    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        time_axis: TimeAxis,
        _times_ms: Option<Vec<i64>>,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf> {
        crate::render::taiko::render_taiko_grid(beatmap, output_path, mods, time_axis, deadline)
    }

    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        time_point: Option<TimePoint>,
        duration_time: Option<f64>,
        output_path: &Path,
        background: Option<Img>,
        audio_job: AudioSourceJob,
        time_axis: TimeAxis,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf> {
        crate::render::taiko::render_taiko_video(
            beatmap,
            mods,
            time_point,
            duration_time,
            output_path,
            background,
            audio_job,
            time_axis,
            deadline,
        )?;
        Ok(output_path.to_path_buf())
    }
}

struct CatchRenderer;
impl ModeRenderer for CatchRenderer {
    fn convert(
        &self,
        beatmap: &Beatmap,
        target_mode: i32,
        mods: Option<&ModSettings>,
    ) -> Result<Beatmap> {
        convert_if_needed(beatmap, 2, target_mode, mods)
    }

    fn render_gif(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        options: GifRenderOptions,
        _time_axis: TimeAxis,
        output_path: &Path,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf> {
        crate::render::catch::render_catch_gif(beatmap, mods, options, output_path, deadline)?;
        Ok(output_path.to_path_buf())
    }

    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        time_axis: TimeAxis,
        _times_ms: Option<Vec<i64>>,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf> {
        crate::render::catch::render_catch_grid(beatmap, output_path, mods, time_axis, deadline)
    }

    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        time_point: Option<TimePoint>,
        duration_time: Option<f64>,
        output_path: &Path,
        background: Option<Img>,
        audio_job: AudioSourceJob,
        time_axis: TimeAxis,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf> {
        crate::render::catch::render_catch_video(
            beatmap,
            mods,
            time_point,
            duration_time,
            output_path,
            background,
            audio_job,
            time_axis,
            deadline,
        )?;
        Ok(output_path.to_path_buf())
    }
}

struct ManiaRenderer;
impl ModeRenderer for ManiaRenderer {
    fn convert(
        &self,
        beatmap: &Beatmap,
        target_mode: i32,
        mods: Option<&ModSettings>,
    ) -> Result<Beatmap> {
        convert_if_needed(beatmap, 3, target_mode, mods)
    }

    fn render_gif(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        options: GifRenderOptions,
        _time_axis: TimeAxis,
        output_path: &Path,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf> {
        crate::render::mania::render_mania_gif(beatmap, mods, options, output_path, deadline)?;
        Ok(output_path.to_path_buf())
    }

    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        time_axis: TimeAxis,
        _times_ms: Option<Vec<i64>>,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf> {
        crate::render::mania::render_mania_grid(beatmap, output_path, mods, time_axis, deadline)
    }

    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        time_point: Option<TimePoint>,
        duration_time: Option<f64>,
        output_path: &Path,
        background: Option<Img>,
        audio_job: AudioSourceJob,
        time_axis: TimeAxis,
        deadline: &RequestDeadline,
    ) -> Result<PathBuf> {
        crate::render::mania::render_mania_video(
            beatmap,
            mods,
            time_point,
            duration_time,
            output_path,
            background,
            audio_job,
            time_axis,
            deadline,
        )?;
        Ok(output_path.to_path_buf())
    }
}

// ── 转换辅助函数 ──

fn resolve_convert_target(beatmap: &Beatmap, name: &str) -> Result<i32> {
    let key = name.to_lowercase();
    let key = key.trim();
    let target = match key {
        "taiko" => 1,
        "ctb" | "catch" => 2,
        "mania" => 3,
        "standard" | "std" => 0,
        _ => {
            return Err(PreviewError::new(format!(
                "unknown convert target: '{name}', expected one of ['catch', 'ctb', 'mania', 'taiko', 'standard']"
            )))
        }
    };

    if target == beatmap.mode() {
        return Ok(target);
    }

    if beatmap.mode() != 0 {
        return Err(PreviewError::new(format!(
            "mode conversion (--convert) is only supported for osu!standard beatmaps, \
             current mode is {}",
            beatmap.mode()
        )));
    }

    Ok(target)
}

type ConvertFn = fn(&Beatmap, i32, Option<&ModSettings>) -> Result<Beatmap>;

static CONVERTERS: &[(i32, ConvertFn)] = &[
    (1, crate::render::taiko::conv::taiko_convert),
    (2, crate::render::catch::conv::catch_convert),
    (3, crate::render::mania::conv::mania_convert),
];

fn convert_beatmap(
    beatmap: &Beatmap,
    target_mode: i32,
    mods: Option<&ModSettings>,
) -> Result<Beatmap> {
    if beatmap.mode() != 0 {
        return Err(PreviewError::new(
            "source beatmap must be osu!standard (mode=0)",
        ));
    }

    CONVERTERS
        .iter()
        .find(|(m, _)| *m == target_mode)
        .map(|(_, f)| f(beatmap, target_mode, mods))
        .unwrap_or_else(|| {
            Err(PreviewError::new(format!(
                "conversion to mode {target_mode} is not yet implemented"
            )))
        })
}

/// 仅当谱面原生模式与目标模式不同时才转换谱面。
fn convert_if_needed(
    beatmap: &Beatmap,
    native_mode: i32,
    target_mode: i32,
    mods: Option<&ModSettings>,
) -> Result<Beatmap> {
    if beatmap.mode() != native_mode {
        convert_beatmap(beatmap, target_mode, mods)
    } else {
        Ok(beatmap.clone())
    }
}

/// 通过 `ModeRenderer` trait 统一分派渲染。
fn render_preview_for_mode(
    renderer: &dyn ModeRenderer,
    beatmap: Beatmap,
    output_path: &Path,
    fmt: &str,
    target_mode: i32,
    mods: Option<ModSettings>,
    time_points: Vec<TimePoint>,
    duration_time: Option<f64>,
    audio_job: Option<AudioSourceJob>,
    bid: &str,
    deadline: &RequestDeadline,
) -> Result<PathBuf> {
    deadline.check()?;
    let mods_ref = mods.as_ref();

    renderer.validate(&beatmap)?;

    let converting = beatmap.mode() != target_mode;
    if converting {
        log::event(
            "convert",
            "start",
            Some(bid),
            &format!("{} -> {}", beatmap.mode(), target_mode),
        );
    }
    let t_convert = Instant::now();
    let beatmap = renderer.convert(&beatmap, target_mode, mods_ref)?;
    deadline.check()?;
    if converting {
        let ms = t_convert.elapsed().as_secs_f64() * 1000.0;
        log::event(
            "convert",
            "done",
            Some(bid),
            &format!("objects={} in {ms:.1} ms", beatmap.hit_objects.len()),
        );
        log::record_stage("convert_ms", ms);
    }

    let (time_axis, _, _) = object_time_axis(&beatmap.hit_objects)
        .ok_or_else(|| PreviewError::render("beatmap has no hit objects"))?;
    let absolute_time_points = resolve_time_points(&beatmap, time_axis, &time_points)?;
    if fmt == "gif" {
        let gif_options = GifRenderOptions::Segments {
            times_ms: absolute_time_points.clone(),
            duration_seconds: duration_time,
            time_axis,
        };
        renderer.render_gif(
            &beatmap,
            mods_ref,
            gif_options,
            time_axis,
            output_path,
            deadline,
        )
    } else if fmt == "mp4" {
        let mut audio_job =
            audio_job.ok_or_else(|| PreviewError::render("MP4 audio job was not started"))?;
        let background = audio_job.take_background();
        renderer.render_video(
            &beatmap,
            mods_ref,
            time_points.first().copied(),
            duration_time,
            output_path,
            background,
            audio_job,
            time_axis,
            deadline,
        )
    } else {
        renderer.render_png(
            &beatmap,
            mods_ref,
            output_path,
            time_axis,
            absolute_time_points,
            deadline,
        )
    }
}

fn timeout_for_format(format: &str) -> Duration {
    let timeouts = &crate::config::current().timeouts.render;
    match format {
        "png" => timeouts.PNG_TIMEOUT,
        "gif" => timeouts.GIF_TIMEOUT,
        "mp4" => timeouts.MP4_TIMEOUT,
        _ => timeouts.PNG_TIMEOUT.max(timeouts.GIF_TIMEOUT),
    }
}

fn deadline_for_format(started: Instant, format: &str) -> RequestDeadline {
    RequestDeadline::new(started, format, timeout_for_format(format))
}

fn initial_deadline(
    started: Instant,
    format: Option<&str>,
    convert: Option<&str>,
) -> RequestDeadline {
    if let Some(format) = format {
        return deadline_for_format(started, format);
    }
    if let Some(convert) = convert {
        let format = if matches!(
            convert.trim().to_ascii_lowercase().as_str(),
            "standard" | "std"
        ) {
            "gif"
        } else {
            "png"
        };
        return deadline_for_format(started, format);
    }
    let timeout = crate::config::current()
        .timeouts
        .render
        .PNG_TIMEOUT
        .max(crate::config::current().timeouts.render.GIF_TIMEOUT);
    RequestDeadline::new(started, "PNG/GIF", timeout)
}

fn format_time_point(point: &TimePoint) -> String {
    match point {
        TimePoint::Preview => "preview".to_string(),
        TimePoint::Seconds(value) => value.to_string(),
    }
}

fn resolve_time_points(
    beatmap: &Beatmap,
    time_axis: TimeAxis,
    points: &[TimePoint],
) -> Result<Option<Vec<i64>>> {
    if points.is_empty() {
        return Ok(None);
    }
    let first_object = object_time_bounds(&beatmap.hit_objects)
        .map(|(first, _)| first)
        .ok_or_else(|| PreviewError::render("beatmap has no hit objects"))?;
    let preview_time = beatmap
        .general
        .get("PreviewTime")
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|time| *time >= 0)
        .unwrap_or(first_object);
    let mut result = Vec::with_capacity(points.len());
    for point in points {
        let absolute = match point {
            TimePoint::Preview => preview_time,
            TimePoint::Seconds(seconds) => {
                if !seconds.is_finite() {
                    return Err(PreviewError::new("time point must be finite"));
                }
                let milliseconds_f64 = seconds * 1000.0;
                if !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0)
                    .contains(&milliseconds_f64)
                {
                    return Err(PreviewError::new(
                        "time point is outside the supported range",
                    ));
                }
                let milliseconds = crate::parser::round_half_even(milliseconds_f64);
                time_axis.to_absolute(milliseconds)?
            }
        };
        result.push(absolute);
    }
    Ok(Some(result))
}

fn has_explicit_video_time_options(time_points: &[TimePoint], duration_time: Option<f64>) -> bool {
    !time_points.is_empty() || duration_time.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::ManiaHitObject;

    #[test]
    fn time_axis_uses_first_object_from_target_mode_objects() {
        let converted_objects = HitObjects::Mania(vec![
            ManiaHitObject {
                lane: 1,
                start_time: 25_000,
                end_time: 26_000,
                is_long_note: true,
            },
            ManiaHitObject {
                lane: 0,
                start_time: 12_500,
                end_time: 12_500,
                is_long_note: false,
            },
        ]);

        let (axis, first, last) = object_time_axis(&converted_objects).unwrap();

        assert_eq!((first, last), (12_500, 26_000));
        assert_eq!(axis.to_display(first), 0);
    }

    #[test]
    fn output_formats_select_their_own_timeout() {
        assert_eq!(timeout_for_format("png").as_secs(), 300);
        assert_eq!(timeout_for_format("gif").as_secs(), 300);
        assert_eq!(timeout_for_format("mp4").as_secs(), 300);
    }

    #[test]
    fn implicit_format_uses_png_gif_provisional_deadline() {
        let deadline = initial_deadline(Instant::now(), None, None);
        assert_eq!(deadline.format(), "PNG/GIF");
        assert_eq!(
            deadline.configured_timeout(),
            timeout_for_format("png").max(timeout_for_format("gif"))
        );
    }

    #[test]
    fn conversion_selects_existing_default_output_format() {
        let standard = initial_deadline(Instant::now(), None, Some("standard"));
        let mania = initial_deadline(Instant::now(), None, Some("mania"));
        assert_eq!(standard.format(), "GIF");
        assert_eq!(mania.format(), "PNG");
    }

    #[test]
    fn implicit_video_defaults_use_the_short_output_name() {
        assert!(!has_explicit_video_time_options(&[], None));
        assert!(has_explicit_video_time_options(
            &[TimePoint::Seconds(0.0)],
            None
        ));
        assert!(has_explicit_video_time_options(&[], Some(600.0)));
        assert!(has_explicit_video_time_options(
            &[TimePoint::Seconds(0.0)],
            Some(600.0)
        ));
    }
}
