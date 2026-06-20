use crate::errors::{PreviewError, Result};
use crate::models::*;
use std::collections::HashMap;
use std::path::Path;

pub fn parse_beatmap(path: &Path) -> Result<Beatmap> {
    let bytes = std::fs::read(path).map_err(|_| PreviewError::new("Failed to parse beatmap."))?;
    let content = String::from_utf8_lossy(&bytes);
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    parse_beatmap_str(content).ok_or_else(|| PreviewError::new("Failed to parse beatmap."))
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
        0 => HitObjects::Standard(parse_standard(hit_lines, &difficulty, &timing_points)?),
        1 => HitObjects::Taiko(parse_taiko(hit_lines, &difficulty, &timing_points)?),
        2 => HitObjects::Catch(parse_catch(hit_lines, &difficulty, &timing_points)?),
        3 => HitObjects::Mania(parse_mania(hit_lines, &difficulty)?),
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

/// Parse Combo1..ComboN from the [Colours] section, in numeric order.
fn parse_combo_colors(lines: Option<&Vec<&str>>) -> Vec<[u8; 3]> {
    let Some(lines) = lines else {
        return Vec::new();
    };
    let mut entries: Vec<(u32, [u8; 3])> = Vec::new();
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let Some(num) = key.strip_prefix("Combo") else {
            continue;
        };
        let Ok(index) = num.parse::<u32>() else {
            continue;
        };
        let parts: Vec<&str> = value.split(',').map(|p| p.trim()).collect();
        if parts.len() < 3 {
            continue;
        }
        let (Ok(r), Ok(g), Ok(b)) = (
            parts[0].parse::<u8>(),
            parts[1].parse::<u8>(),
            parts[2].parse::<u8>(),
        ) else {
            continue;
        };
        entries.push((index, [r, g, b]));
    }
    entries.sort_by_key(|e| e.0);
    entries.into_iter().map(|e| e.1).collect()
}

fn split_sections(content: &str) -> HashMap<String, Vec<&str>> {
    let mut sections: HashMap<String, Vec<&str>> = HashMap::new();
    let mut current: Option<String> = None;

    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line[1..line.len() - 1].to_string();
            sections.entry(name.clone()).or_default();
            current = Some(name);
            continue;
        }
        if let Some(name) = &current {
            sections.get_mut(name).unwrap().push(line);
        }
    }
    sections
}

fn parse_format_version(content: &str) -> i32 {
    let first = content.lines().next().unwrap_or("").trim();
    if let Some(rest) = first.strip_prefix("osu file format v") {
        if let Ok(v) = rest.parse() {
            return v;
        }
    }
    14
}

fn parse_key_value(lines: &[&str]) -> KvSection {
    let mut kv = KvSection::default();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            kv.insert(key.trim(), value.trim().to_string());
        }
    }
    kv
}

fn default_metadata() -> KvSection {
    let mut kv = KvSection::default();
    kv.insert("Title", "Unknown".into());
    kv.insert("TitleUnicode", "".into());
    kv.insert("Artist", "Unknown".into());
    kv.insert("ArtistUnicode", "".into());
    kv.insert("Creator", "Unknown".into());
    kv.insert("Version", "Unknown".into());
    kv
}

fn parse_timing_points(lines: &[&str]) -> Option<Vec<TimingPoint>> {
    let mut points: Vec<TimingPoint> = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 2 {
            continue;
        }
        let mut meter = if parts.len() > 2 && !parts[2].is_empty() {
            parts[2].parse::<i32>().ok()?
        } else {
            4
        };
        if meter <= 0 {
            meter = 4;
        }
        let uninherited = parts.len() < 7 || parts[6] == "1";
        let effects = if parts.len() > 7 && !parts[7].is_empty() {
            parts[7].parse::<i32>().ok()?
        } else {
            0
        };
        points.push(TimingPoint {
            time: parts[0].parse().ok()?,
            beat_length: parts[1].parse().ok()?,
            meter,
            uninherited,
            kiai_mode: effects & 1 != 0,
        });
    }
    // Stable sort keeps file order for equal times (red/green at same time).
    points.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
    if points.is_empty() {
        return None;
    }
    Some(points)
}

fn parse_break_periods(lines: Option<&Vec<&str>>) -> Vec<BreakPeriod> {
    let Some(lines) = lines else {
        return Vec::new();
    };
    let mut breaks = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 3 || parts[0] != "2" {
            continue;
        }
        let (Ok(s), Ok(e)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) else {
            continue;
        };
        let (start_time, end_time) = (s as i64, e as i64);
        if end_time > start_time {
            breaks.push(BreakPeriod {
                start_time,
                end_time,
            });
        }
    }
    breaks
}

struct SliderFields {
    slider_type: String,
    points: Vec<(i32, i32)>,
    repeats: i32,
    pixel_length: f64,
    edge_hitsounds: Vec<i32>,
}

fn parse_slider_fields(parts: &[&str]) -> Option<SliderFields> {
    let mut slider_parts = parts[5].split('|');
    let slider_type = slider_parts.next()?.to_string();
    let mut points = Vec::new();
    for p in slider_parts {
        let (x, y) = p.split_once(':')?;
        points.push((x.parse().ok()?, y.parse().ok()?));
    }
    let repeats: i32 = parts.get(6)?.parse().ok()?;
    let pixel_length: f64 = parts.get(7)?.parse().ok()?;
    let mut edge_hitsounds = Vec::new();
    if let Some(eh) = parts.get(8) {
        if !eh.is_empty() {
            for v in eh.split('|') {
                if !v.is_empty() {
                    edge_hitsounds.push(v.parse().ok()?);
                }
            }
        }
    }
    Some(SliderFields {
        slider_type,
        points,
        repeats,
        pixel_length,
        edge_hitsounds,
    })
}

fn parse_standard(
    lines: &[&str],
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<Vec<StandardHitObject>> {
    let mut objects = Vec::with_capacity(lines.len());
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 5 {
            continue;
        }
        let x: i32 = parts[0].parse::<f64>().ok()? as i32;
        let y: i32 = parts[1].parse::<f64>().ok()? as i32;
        let start_time: i64 = parts[2].parse().ok()?;
        let hit_type: i32 = parts[3].parse().ok()?;
        let hitsound: i32 = parts[4].parse().ok()?;
        let end_time = parse_end_time(&parts, start_time, hit_type, difficulty, timing_points)?;

        let mut obj = StandardHitObject {
            x,
            y,
            start_time,
            end_time,
            hit_type,
            hitsound,
            new_combo: hit_type & 4 != 0,
            combo_offset: (hit_type & 112) >> 4,
            ..Default::default()
        };
        if hit_type & 2 != 0 {
            let sf = parse_slider_fields(&parts)?;
            obj.slider_type = Some(sf.slider_type);
            obj.slider_points = sf.points;
            obj.slider_repeats = sf.repeats;
            obj.slider_pixel_length = sf.pixel_length;
            obj.slider_edge_hitsounds = sf.edge_hitsounds;
        }
        objects.push(obj);
    }
    objects.sort_by_key(|o| (o.start_time, o.end_time));
    Some(objects)
}

fn parse_taiko(
    lines: &[&str],
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<Vec<TaikoHitObject>> {
    let mut objects = Vec::with_capacity(lines.len());
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 5 {
            continue;
        }
        let start_time: i64 = parts[2].parse().ok()?;
        let hit_type: i32 = parts[3].parse().ok()?;
        let hitsound: i32 = parts[4].parse().ok()?;
        let end_time = parse_end_time(&parts, start_time, hit_type, difficulty, timing_points)?;
        objects.push(TaikoHitObject {
            start_time,
            end_time,
            hit_type,
            hitsound,
        });
    }
    objects.sort_by_key(|o| (o.start_time, o.end_time));
    Some(objects)
}

fn parse_catch(
    lines: &[&str],
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<Vec<CatchHitObject>> {
    let mut objects = Vec::with_capacity(lines.len());
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 5 {
            continue;
        }
        let x: i32 = parts[0].parse::<f64>().ok()? as i32;
        let y: i32 = parts[1].parse::<f64>().ok()? as i32;
        let start_time: i64 = parts[2].parse().ok()?;
        let hit_type: i32 = parts[3].parse().ok()?;
        let end_time = parse_end_time(&parts, start_time, hit_type, difficulty, timing_points)?;

        let mut obj = CatchHitObject {
            x,
            y,
            start_time,
            end_time,
            hit_type,
            new_combo: hit_type & 4 != 0,
            combo_offset: (hit_type & 112) >> 4,
            slider_type: None,
            slider_points: Vec::new(),
            slider_repeats: 1,
            slider_pixel_length: 0.0,
        };
        if hit_type & 2 != 0 {
            let sf = parse_slider_fields(&parts)?;
            obj.slider_type = Some(sf.slider_type);
            obj.slider_points = sf.points;
            obj.slider_repeats = sf.repeats;
            obj.slider_pixel_length = sf.pixel_length;
        }
        objects.push(obj);
    }
    objects.sort_by_key(|o| (o.start_time, o.end_time));
    Some(objects)
}

fn parse_mania(lines: &[&str], difficulty: &KvSection) -> Option<Vec<ManiaHitObject>> {
    let key_count = difficulty.get_f64("CircleSize")? as i64;
    let mut objects = Vec::with_capacity(lines.len());
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 5 {
            continue;
        }
        let x: i64 = parts[0].parse::<f64>().ok()? as i64;
        let start_time: i64 = parts[2].parse().ok()?;
        let hit_type: i32 = parts[3].parse().ok()?;
        let lane = (x * key_count).div_euclid(512).clamp(0, key_count - 1) as i32;
        let is_long_note = hit_type & 128 != 0;
        let mut end_time = start_time;
        if is_long_note {
            let head = parts.get(5)?.split(':').next()?;
            end_time = head.parse().ok()?;
        }
        objects.push(ManiaHitObject {
            lane,
            start_time,
            end_time,
            is_long_note,
        });
    }
    objects.sort_by_key(|o| (o.start_time, o.end_time));
    Some(objects)
}

fn parse_end_time(
    parts: &[&str],
    start_time: i64,
    hit_type: i32,
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<i64> {
    if hit_type & 8 != 0 {
        return parts.get(5)?.parse::<f64>().ok().map(|v| v as i64);
    }
    if hit_type & 2 != 0 {
        return parse_slider_end_time(parts, start_time, difficulty, timing_points);
    }
    Some(start_time)
}

fn parse_slider_end_time(
    parts: &[&str],
    start_time: i64,
    difficulty: &KvSection,
    timing_points: &[TimingPoint],
) -> Option<i64> {
    let slides: f64 = parts.get(6)?.parse::<i32>().ok()? as f64;
    let pixel_length: f64 = parts.get(7)?.parse().ok()?;
    let slider_multiplier = difficulty.get_f64("SliderMultiplier")?;
    let (beat_length, slider_velocity) = resolve_slider_timing(start_time, timing_points);
    let duration = pixel_length / (slider_multiplier * 100.0 * slider_velocity) * beat_length * slides;
    Some(start_time + round_half_even(duration))
}

pub fn resolve_slider_timing(start_time: i64, timing_points: &[TimingPoint]) -> (f64, f64) {
    let mut beat_length = timing_points[0].beat_length;
    let mut slider_velocity = 1.0;
    for point in timing_points {
        if point.time > start_time as f64 {
            break;
        }
        if point.uninherited {
            beat_length = point.beat_length;
            slider_velocity = 1.0;
        } else if point.beat_length < 0.0 {
            slider_velocity = -100.0 / point.beat_length;
        }
    }
    (beat_length, slider_velocity)
}

// Python's round() = banker's rounding.
pub fn round_half_even(v: f64) -> i64 {
    let floor = v.floor();
    let diff = v - floor;
    if diff > 0.5 {
        floor as i64 + 1
    } else if diff < 0.5 {
        floor as i64
    } else {
        let f = floor as i64;
        if f % 2 == 0 {
            f
        } else {
            f + 1
        }
    }
}
