use crate::core::errors::{PreviewError, Result};
use crate::core::models::{Beatmap, HitObjects};
use crate::core::mods::ModSettings;
use crate::core::validate::{self, ValidateContext};
use crate::log::{self, CacheKind, SummaryRecord};
use crate::pipeline::cache;
use crate::render::video::audio::AudioSourceJob;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub fn generate_preview(
    bid: &str,
    fmt: Option<&str>,
    convert: Option<&str>,
    mods: Option<ModSettings>,
    times: Option<Vec<f64>>,
    preview_30s: bool,
    gap: Option<f64>,
    no_cache: bool,
) -> Result<Value> {
    log::set_bid(bid);
    let started = Instant::now();
    let mut rec = SummaryRecord {
        bid: bid.to_string(),
        fmt: fmt.map(str::to_string),
        convert: convert.map(str::to_string),
        times: times.clone(),
        preview_30s,
        gap,
        no_cache,
        ..SummaryRecord::default()
    };
    match generate_preview_inner(
        bid,
        fmt,
        convert,
        mods,
        times,
        preview_30s,
        gap,
        no_cache,
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
    }
}

fn generate_preview_inner(
    bid: &str,
    fmt: Option<&str>,
    convert: Option<&str>,
    mods: Option<ModSettings>,
    times: Option<Vec<f64>>,
    preview_30s: bool,
    gap: Option<f64>,
    no_cache: bool,
    rec: &mut SummaryRecord,
) -> Result<Value> {
    let temp_root = std::env::temp_dir().join("osu-beatmap-preview");

    // ── .osu 下载与解析 ──
    let t0 = Instant::now();
    let beatmap_path = crate::pipeline::downloader::download_beatmap_file(
        bid,
        &temp_root.join("osu-download-cache"),
        no_cache,
    )?;
    rec.download_osu_ms = Some(t0.elapsed().as_secs_f64() * 1000.0);
    rec.osu_bytes = beatmap_path.metadata().ok().map(|meta| meta.len());

    let t1 = Instant::now();
    let beatmap = crate::parser::parse_beatmap(&beatmap_path)?;
    rec.parse_ms = Some(t1.elapsed().as_secs_f64() * 1000.0);
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
    rec.fmt = Some(fmt.clone());

    let ctx = ValidateContext {
        bid,
        fmt: &fmt,
        target_mode,
    };
    let mods = validate::validate_with_context(&ctx, times.as_deref(), preview_30s, gap, mods)?;
    rec.mods = mods.as_ref().map(|m| m.tokens.join("+"));

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
    if let Some(t) = &times {
        if !t.is_empty() {
            parts.push(cache::format_time_suffix(t));
        }
    }
    if preview_30s {
        parts.push(cache::format_preview_30s_suffix().to_string());
    }
    if let Some(b) = gap {
        parts.push(format!("bpm{}", b));
    }
    let output_path: PathBuf =
        temp_root
            .join("outputs")
            .join(format!("{}.{}", parts.join("_"), fmt));

    // ── image cache check ──
    let cached = cache::output_cache_hit(
        &output_path,
        &beatmap_path,
        &times,
        &fmt,
        target_mode,
        no_cache,
    );
    if let Some(cached_path) = cached {
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
            temp_root.join("osz-download-cache"),
            no_cache,
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
    let preview_path = match render_preview_for_mode(
        renderer,
        beatmap.clone(),
        &output_path,
        &fmt,
        target_mode,
        mods,
        times,
        preview_30s,
        gap,
        audio_job,
        bid,
    ) {
        Ok(path) => path,
        // Atomic writes mean a failed render never touches the final path, so
        // a previously good cached output is preserved. The temp file (if
        // any) was already cleaned up by the writer.
        Err(error) => return Err(error),
    };
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
    rec.bpm = main_bpm(&beatmap);
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

/// 谱面主 BPM（第一个未继承 timing point），保留两位小数。
fn main_bpm(beatmap: &Beatmap) -> Option<f64> {
    beatmap
        .timing_points
        .iter()
        .find(|t| t.uninherited && t.beat_length > 0.0)
        .map(|t| (60000.0 / t.beat_length * 100.0).round() / 100.0)
}

// ── ModeRenderer trait ──

trait ModeRenderer {
    /// Render a GIF animation to `output_path`. Returns the output path.
    fn render_gif(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        times_ms: Option<Vec<i64>>,
        output_path: &Path,
    ) -> Result<PathBuf>;

    /// Render a static PNG to `output_path`. Returns the output path.
    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        gap: Option<f64>,
    ) -> Result<PathBuf>;

    /// Render an MP4 (H.264) video of the full chart to `output_path`.
    /// `times_ms` is either `None` (full chart, ±2s padding) or `Some([t1, t2])`
    /// (explicit range). `preview_30s` selects a PreviewTime-based 30s actual
    /// duration clip. Invalid combinations are rejected by validation.
    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        times_ms: Option<Vec<i64>>,
        preview_30s: bool,
        output_path: &Path,
        audio_job: AudioSourceJob,
    ) -> Result<PathBuf>;

    /// Optionally convert the beatmap before rendering. Default: clone (no conversion).
    fn convert(
        &self,
        beatmap: &Beatmap,
        _target_mode: i32,
        _mods: Option<&ModSettings>,
    ) -> Result<Beatmap> {
        Ok(beatmap.clone())
    }

    /// Validate that the beatmap has hit objects. Default: accept anything.
    fn validate(&self, _beatmap: &Beatmap) -> Result<()> {
        Ok(())
    }
}

// ── Mode implementations ──

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
        times_ms: Option<Vec<i64>>,
        output_path: &Path,
    ) -> Result<PathBuf> {
        crate::render::standard::render_standard_gif(beatmap, mods, times_ms, output_path)?;
        Ok(output_path.to_path_buf())
    }

    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        _gap: Option<f64>,
    ) -> Result<PathBuf> {
        let image = crate::render::standard::render_standard_png(beatmap, mods, None)?;
        crate::render::composer::save_png(&image, output_path)?;
        Ok(output_path.to_path_buf())
    }

    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        times_ms: Option<Vec<i64>>,
        preview_30s: bool,
        output_path: &Path,
        audio_job: AudioSourceJob,
    ) -> Result<PathBuf> {
        crate::render::standard::render_standard_video(
            beatmap,
            mods,
            times_ms,
            preview_30s,
            output_path,
            audio_job,
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
        times_ms: Option<Vec<i64>>,
        output_path: &Path,
    ) -> Result<PathBuf> {
        crate::render::taiko::render_taiko_gif(beatmap, mods, times_ms, output_path)?;
        Ok(output_path.to_path_buf())
    }

    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        gap: Option<f64>,
    ) -> Result<PathBuf> {
        crate::render::taiko::render_taiko_grid(beatmap, output_path, mods, gap)
    }

    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        times_ms: Option<Vec<i64>>,
        preview_30s: bool,
        output_path: &Path,
        audio_job: AudioSourceJob,
    ) -> Result<PathBuf> {
        crate::render::taiko::render_taiko_video(
            beatmap,
            mods,
            times_ms,
            preview_30s,
            output_path,
            audio_job,
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
        times_ms: Option<Vec<i64>>,
        output_path: &Path,
    ) -> Result<PathBuf> {
        crate::render::catch::render_catch_gif(beatmap, mods, times_ms, output_path)?;
        Ok(output_path.to_path_buf())
    }

    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        _gap: Option<f64>,
    ) -> Result<PathBuf> {
        crate::render::catch::render_catch_grid(beatmap, output_path, mods)
    }

    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        times_ms: Option<Vec<i64>>,
        preview_30s: bool,
        output_path: &Path,
        audio_job: AudioSourceJob,
    ) -> Result<PathBuf> {
        crate::render::catch::render_catch_video(
            beatmap,
            mods,
            times_ms,
            preview_30s,
            output_path,
            audio_job,
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
        times_ms: Option<Vec<i64>>,
        output_path: &Path,
    ) -> Result<PathBuf> {
        crate::render::mania::render_mania_gif(beatmap, mods, times_ms, output_path)?;
        Ok(output_path.to_path_buf())
    }

    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        _gap: Option<f64>,
    ) -> Result<PathBuf> {
        crate::render::mania::render_mania_grid(beatmap, output_path, mods)
    }

    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        times_ms: Option<Vec<i64>>,
        preview_30s: bool,
        output_path: &Path,
        audio_job: AudioSourceJob,
    ) -> Result<PathBuf> {
        crate::render::mania::render_mania_video(
            beatmap,
            mods,
            times_ms,
            preview_30s,
            output_path,
            audio_job,
        )?;
        Ok(output_path.to_path_buf())
    }
}

// ── conversion helpers ──

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

/// Convert the beatmap only if its native mode differs from the target.
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

/// Unified render dispatch through the `ModeRenderer` trait.
fn render_preview_for_mode(
    renderer: &dyn ModeRenderer,
    beatmap: Beatmap,
    output_path: &Path,
    fmt: &str,
    target_mode: i32,
    mods: Option<ModSettings>,
    times: Option<Vec<f64>>,
    preview_30s: bool,
    gap: Option<f64>,
    audio_job: Option<AudioSourceJob>,
    bid: &str,
) -> Result<PathBuf> {
    let times_ms = crate::common::time_selection::times_to_milliseconds(times.as_deref());
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

    if fmt == "gif" {
        renderer.render_gif(&beatmap, mods_ref, times_ms, output_path)
    } else if fmt == "mp4" {
        let audio_job =
            audio_job.ok_or_else(|| PreviewError::render("MP4 audio job was not started"))?;
        renderer.render_video(
            &beatmap,
            mods_ref,
            times_ms,
            preview_30s,
            output_path,
            audio_job,
        )
    } else {
        renderer.render_png(&beatmap, mods_ref, output_path, gap)
    }
}
