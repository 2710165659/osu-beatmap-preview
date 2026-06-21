//! .osu beatmap file parser.
//!
//! Sub-modules:
//! - `sections`: section splitting, key-value parsing, combo colours
//! - `timing`: timing points and break periods
//! - `hit_objects`: mode-specific hit-object parsing, slider timing, rounding

mod hit_objects;
mod sections;
mod timing;

pub use hit_objects::round_half_even;
pub use sections::{default_metadata, parse_combo_colors, parse_format_version, parse_key_value, split_sections};
pub use timing::{parse_break_periods, parse_timing_points};

use crate::errors::{PreviewError, Result};
use crate::models::*;
use std::path::Path;

/// Parse a .osu file from disk.
pub fn parse_beatmap(path: &Path) -> Result<Beatmap> {
    let bytes = std::fs::read(path)
        .map_err(|e| PreviewError::parse(format!("Failed to read beatmap file: {e}")))?;
    let content = String::from_utf8_lossy(&bytes);
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    parse_beatmap_str(content).ok_or_else(|| PreviewError::parse("Failed to parse beatmap."))
}

fn parse_beatmap_str(content: &str) -> Option<Beatmap> {
    let sections = split_sections(content);

    let metadata = match sections.get("Metadata") {
        Some(lines) => parse_key_value(lines),
        None => default_metadata(),
    };
    let difficulty = parse_key_value(sections.get("Difficulty")?);
    let mut general = match sections.get("General") {
        Some(lines) => parse_key_value(lines),
        None => {
            let mut kv = KvSection::default();
            kv.insert("Mode", "0".to_string());
            kv
        }
    };
    general.insert("FormatVersion", parse_format_version(content).to_string());
    let timing_points = parse_timing_points(sections.get("TimingPoints")?)?;
    let break_periods = parse_break_periods(sections.get("Events"));
    let mode: i32 = general.get("Mode").unwrap_or("0").parse().ok()?;

    let combo_colors = parse_combo_colors(sections.get("Colours"));

    let editor = sections
        .get("Editor")
        .map(|lines| parse_key_value(lines))
        .unwrap_or_default();
    let beat_divisor: i32 = editor.get("BeatDivisor").and_then(|v| v.parse().ok()).unwrap_or(0);

    let hit_lines = sections.get("HitObjects")?;
    let hit_objects = match mode {
        0 => HitObjects::Standard(hit_objects::parse_standard(hit_lines, &difficulty, &timing_points)?),
        1 => HitObjects::Taiko(hit_objects::parse_taiko(hit_lines, &difficulty, &timing_points)?),
        2 => HitObjects::Catch(hit_objects::parse_catch(hit_lines, &difficulty, &timing_points)?),
        3 => HitObjects::Mania(hit_objects::parse_mania(hit_lines, &difficulty)?),
        _ => return None,
    };

    Some(Beatmap {
        metadata,
        difficulty,
        general,
        timing_points,
        hit_objects,
        break_periods,
        combo_colors,
        beat_divisor,
    })
}
