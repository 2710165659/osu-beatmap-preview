//! Beatmap section parsing: split a .osu file into `[Section]` blocks,
//! parse key-value pairs, format version, and combo colours.

use crate::models::KvSection;
use std::collections::HashMap;

/// Split raw .osu content into named sections (HashMap<section_name, Vec<lines>>).
pub fn split_sections(content: &str) -> HashMap<String, Vec<&str>> {
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

/// Parse the file format version from the first line of the file.
pub fn parse_format_version(content: &str) -> i32 {
    let first = content.lines().next().unwrap_or("").trim();
    if let Some(rest) = first.strip_prefix("osu file format v") {
        if let Ok(v) = rest.parse() {
            return v;
        }
    }
    14
}

/// Parse a key-value section into a `KvSection`.
pub fn parse_key_value(lines: &[&str]) -> KvSection {
    let mut kv = KvSection::default();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            kv.insert(key.trim(), value.trim().to_string());
        }
    }
    kv
}

/// Default metadata when the [Metadata] section is missing.
pub fn default_metadata() -> KvSection {
    let mut kv = KvSection::default();
    kv.insert("Title", "Unknown".into());
    kv.insert("TitleUnicode", "".into());
    kv.insert("Artist", "Unknown".into());
    kv.insert("ArtistUnicode", "".into());
    kv.insert("Creator", "Unknown".into());
    kv.insert("Version", "Unknown".into());
    kv
}

/// Parse Combo1..ComboN from the [Colours] section, in numeric order.
pub fn parse_combo_colors(lines: Option<&Vec<&str>>) -> Vec<[u8; 3]> {
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
