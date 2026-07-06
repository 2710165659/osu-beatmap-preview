use crate::cache;
use crate::errors::{PreviewError, Result};
use crate::models::{Beatmap, HitObjects};
use crate::mods::ModSettings;
use crate::validate::{self, ValidateContext};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub fn generate_preview(
    bid: &str,
    fmt: Option<&str>,
    convert: Option<&str>,
    mods: Option<ModSettings>,
    times: Option<Vec<f64>>,
    gap: Option<f64>,
    no_cache: bool,
) -> Result<Value> {
    let temp_root = std::env::temp_dir().join("osu-beatmap-preview");
    let beatmap_path =
        crate::downloader::download_beatmap_file(bid, &temp_root.join("osu-download-cache"), no_cache)?;
    let beatmap = crate::parser::parse_beatmap(&beatmap_path)?;

    let mut target_mode = beatmap.mode();
    let mut convert_used: Option<&str> = None;
    if let Some(convert_name) = convert {
        let mode = resolve_convert_target(&beatmap, convert_name)?;
        if mode != beatmap.mode() {
            target_mode = mode;
            convert_used = Some(convert_name);
        }
    }

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

    let ctx = ValidateContext {
        bid,
        fmt: &fmt,
        target_mode,
    };
    let mods = validate::validate_with_context(
        &ctx,
        times.as_deref(),
        gap,
        mods,
    )?;

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
    if let Some(b) = gap {
        parts.push(format!("bpm{}", b));
    }
    let output_path: PathBuf = temp_root
        .join("outputs")
        .join(format!("{}.{}", parts.join("_"), fmt));

    // ── image cache check ──
    let cached = cache::output_cache_hit(&output_path, &beatmap_path, &times, &fmt, target_mode, no_cache);
    if let Some(cached_path) = cached {
        let abs = cached_path
            .canonicalize()
            .unwrap_or(cached_path.clone());
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

    let renderer: &dyn ModeRenderer = match target_mode {
        0 => &StandardRenderer,
        1 => &TaikoRenderer,
        2 => &CatchRenderer,
        3 => &ManiaRenderer,
        _ => return Err(PreviewError::new(format!(
            "unsupported beatmap mode: {target_mode}"
        ))),
    };

    let preview_path = render_preview_for_mode(
        renderer, beatmap.clone(), &output_path, &fmt, target_mode, mods, times, gap,
    )?;

    let abs = preview_path
        .canonicalize()
        .unwrap_or(preview_path.clone());
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
    /// (explicit range); other lengths are rejected by validation. Returns the
    /// output path.
    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        times_ms: Option<Vec<i64>>,
        output_path: &Path,
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
        crate::standard::render_standard_gif(beatmap, mods, times_ms, output_path)?;
        Ok(output_path.to_path_buf())
    }

    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        _gap: Option<f64>,
    ) -> Result<PathBuf> {
        let image = crate::standard::render_standard_png(beatmap, mods, None)?;
        crate::composer::save_png(&image, output_path)?;
        Ok(output_path.to_path_buf())
    }

    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        times_ms: Option<Vec<i64>>,
        output_path: &Path,
    ) -> Result<PathBuf> {
        crate::standard::render_standard_video(beatmap, mods, times_ms, output_path)?;
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
        crate::taiko::render_taiko_gif(beatmap, mods, times_ms, output_path)?;
        Ok(output_path.to_path_buf())
    }

    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        gap: Option<f64>,
    ) -> Result<PathBuf> {
        crate::taiko::render_taiko_grid(beatmap, output_path, mods, gap)
    }

    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        times_ms: Option<Vec<i64>>,
        output_path: &Path,
    ) -> Result<PathBuf> {
        crate::taiko::render_taiko_video(beatmap, mods, times_ms, output_path)?;
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
        crate::catch::render_catch_gif(beatmap, mods, times_ms, output_path)?;
        Ok(output_path.to_path_buf())
    }

    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        _gap: Option<f64>,
    ) -> Result<PathBuf> {
        crate::catch::render_catch_grid(beatmap, output_path, mods)
    }

    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        times_ms: Option<Vec<i64>>,
        output_path: &Path,
    ) -> Result<PathBuf> {
        crate::catch::render_catch_video(beatmap, mods, times_ms, output_path)?;
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
        crate::mania::render_mania_gif(beatmap, mods, times_ms, output_path)?;
        Ok(output_path.to_path_buf())
    }

    fn render_png(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        output_path: &Path,
        _gap: Option<f64>,
    ) -> Result<PathBuf> {
        crate::mania::render_mania_grid(beatmap, output_path, mods)
    }

    fn render_video(
        &self,
        beatmap: &Beatmap,
        mods: Option<&ModSettings>,
        times_ms: Option<Vec<i64>>,
        output_path: &Path,
    ) -> Result<PathBuf> {
        crate::mania::render_mania_video(beatmap, mods, times_ms, output_path)?;
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
    (1, crate::taiko::conv::taiko_convert),
    (2, crate::catch::conv::catch_convert),
    (3, crate::mania::conv::mania_convert),
];

fn convert_beatmap(
    beatmap: &Beatmap,
    target_mode: i32,
    mods: Option<&ModSettings>,
) -> Result<Beatmap> {
    if beatmap.mode() != 0 {
        return Err(PreviewError::new("source beatmap must be osu!standard (mode=0)"));
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
    gap: Option<f64>,
) -> Result<PathBuf> {
    let times_ms = crate::common::time_selection::times_to_milliseconds(times.as_deref());
    let mods_ref = mods.as_ref();

    renderer.validate(&beatmap)?;
    let beatmap = renderer.convert(&beatmap, target_mode, mods_ref)?;

    if fmt == "gif" {
        renderer.render_gif(&beatmap, mods_ref, times_ms, output_path)
    } else if fmt == "mp4" {
        renderer.render_video(&beatmap, mods_ref, times_ms, output_path)
    } else {
        renderer.render_png(&beatmap, mods_ref, output_path, gap)
    }
}
