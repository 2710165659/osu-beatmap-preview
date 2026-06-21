//! Timing/scrolling helpers for osu!taiko: scroll segments, redline sections,
//! kiai sections, timing lines, SV changes, and mod helpers.

use crate::errors::{PreviewError, Result};
use crate::models::{Beatmap, TaikoHitObject, TimingPoint};
use crate::mods::ModSettings;
use crate::parser::round_half_even;
use std::collections::BTreeMap;

use super::constants::*;

// ─── helpers ───

#[inline]
pub(crate) fn pyround(v: f64) -> i64 {
    round_half_even(v)
}

// ─── data structs ───

#[derive(Debug, Clone)]
pub(crate) struct ScrollSegment {
    pub(crate) start_time: i64,
    pub(crate) end_time: i64,
    pub(crate) pixels_per_ms: f64,
    pub(crate) start_position: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct RedlineSection {
    pub(crate) start_time: i64,
    pub(crate) end_time: i64,
    pub(crate) beat_length: f64,
    pub(crate) meter: i32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct KiaiSection {
    pub(crate) start_time: i64,
    pub(crate) end_time: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct TimingLine {
    pub(crate) time: i64,
    pub(crate) position: f64,
    pub(crate) is_measure: bool,
    pub(crate) show_label: bool,
    pub(crate) is_kiai: bool,
    pub(crate) is_kiai_start: bool,
    pub(crate) bpm: Option<f64>,
}

#[derive(Debug, Clone)]
pub(crate) struct SvChange {
    pub(crate) position: f64,
    pub(crate) sv: f64,
}

pub(crate) struct ScrollPositionMapper {
    segments: Vec<ScrollSegment>,
    start_times: Vec<i64>,
}

impl ScrollPositionMapper {
    pub(crate) fn new(segments: Vec<ScrollSegment>) -> Self {
        let start_times = segments.iter().map(|s| s.start_time).collect();
        ScrollPositionMapper {
            segments,
            start_times,
        }
    }

    pub(crate) fn end_position(&self) -> f64 {
        let segment = self.segments.last().unwrap();
        segment.start_position
            + (segment.end_time - segment.start_time) as f64 * segment.pixels_per_ms
    }

    pub(crate) fn position_at(&self, time: f64) -> f64 {
        let last_end = self.segments.last().unwrap().end_time as f64;
        let clamped_time = time.min(last_end).max(0.0);
        let idx = self
            .start_times
            .partition_point(|&s| (s as f64) <= clamped_time);
        let segment = &self.segments[idx.saturating_sub(1)];
        segment.start_position
            + (clamped_time - segment.start_time as f64) * segment.pixels_per_ms
    }
}

// ─── scroll mapper ───

pub(crate) fn build_scroll_mapper(
    timing_points: &[TimingPoint],
    chart_end_time: i64,
    slider_multiplier: f64,
    spacing_bpm: f64,
) -> ScrollPositionMapper {
    ScrollPositionMapper::new(build_scroll_segments(
        timing_points,
        chart_end_time,
        slider_multiplier,
        spacing_bpm,
    ))
}

fn pixels_per_ms(slider_multiplier: f64, scroll_speed: f64, display_beat_length: f64) -> f64 {
    PIXELS_PER_SCROLL_MULTIPLIER_MS * SCROLL_LENGTH_RATIO * slider_multiplier * scroll_speed
        * 1000.0
        / display_beat_length
}

fn apply_timing_state(
    point: &TimingPoint,
    beat_length: f64,
    meter: i32,
    scroll_speed: f64,
) -> (f64, i32, f64) {
    if point.uninherited {
        let bl = if point.beat_length >= 60.0 {
            point.beat_length
        } else {
            60_000.0 / 180.0
        };
        return (bl, point.meter, scroll_speed);
    }
    if point.beat_length.is_nan() {
        return (beat_length, meter, scroll_speed);
    }
    if point.beat_length >= -0.001 {
        return (beat_length, meter, 1.0);
    }
    (beat_length, meter, -100.0 / point.beat_length)
}

fn build_scroll_segments(
    timing_points: &[TimingPoint],
    chart_end_time: i64,
    slider_multiplier: f64,
    spacing_bpm: f64,
) -> Vec<ScrollSegment> {
    let mut beat_length = DEFAULT_BEAT_LENGTH;
    let mut meter = DEFAULT_METER;
    let mut scroll_speed = 1.0;

    for point in timing_points {
        if point.time > 0.0 {
            break;
        }
        let (bl, m, ss) = apply_timing_state(point, beat_length, meter, scroll_speed);
        beat_length = bl;
        meter = m;
        scroll_speed = ss;
    }

    let mut display_beat_length = if spacing_bpm > 0.0 {
        60_000.0 / spacing_bpm
    } else {
        beat_length
    };
    let mut segment_start: i64 = 0;
    let mut segment_position = 0.0f64;
    let mut segments: Vec<ScrollSegment> = Vec::new();

    for point in timing_points {
        let point_time = pyround(point.time);
        if point_time <= 0 || point_time >= chart_end_time {
            continue;
        }

        if point_time > segment_start {
            let ppm = pixels_per_ms(slider_multiplier, scroll_speed, display_beat_length);
            segments.push(ScrollSegment {
                start_time: segment_start,
                end_time: point_time,
                pixels_per_ms: ppm,
                start_position: segment_position,
            });
            segment_position += (point_time - segment_start) as f64 * ppm;
        }

        let (bl, m, ss) = apply_timing_state(point, beat_length, meter, scroll_speed);
        beat_length = bl;
        meter = m;
        scroll_speed = ss;
        display_beat_length = if spacing_bpm > 0.0 {
            60_000.0 / spacing_bpm
        } else {
            beat_length
        };
        segment_start = point_time;
    }

    let ppm = pixels_per_ms(slider_multiplier, scroll_speed, display_beat_length);
    segments.push(ScrollSegment {
        start_time: segment_start,
        end_time: chart_end_time,
        pixels_per_ms: ppm,
        start_position: segment_position,
    });
    segments
}

// ─── redline sections ───

pub(crate) fn build_redline_sections(
    timing_points: &[TimingPoint],
    chart_end_time: i64,
) -> Vec<RedlineSection> {
    let mut beat_length = DEFAULT_BEAT_LENGTH;
    let mut meter = DEFAULT_METER;
    let mut section_start: i64 = 0;
    let mut last_pre_zero_red: Option<f64> = None;

    for point in timing_points {
        if point.time > 0.0 {
            break;
        }
        if point.uninherited {
            beat_length = point.beat_length;
            meter = point.meter;
            last_pre_zero_red = Some(point.time);
        }
    }

    // If the last red line before time 0 has a negative time (happens after
    // chart trimming), start the first section from that negative time so
    // beat phase is preserved.
    if let Some(t) = last_pre_zero_red {
        if t < 0.0 {
            section_start = pyround(t);
        }
    }

    let mut sections: Vec<RedlineSection> = Vec::new();
    for point in timing_points {
        let point_time = pyround(point.time);
        if point_time <= 0 || point_time >= chart_end_time || !point.uninherited {
            continue;
        }
        if point_time > section_start {
            sections.push(RedlineSection {
                start_time: section_start,
                end_time: point_time,
                beat_length,
                meter,
            });
        }
        beat_length = point.beat_length;
        meter = point.meter;
        section_start = point_time;
    }

    sections.push(RedlineSection {
        start_time: section_start,
        end_time: chart_end_time,
        beat_length,
        meter,
    });
    sections
}

// ─── kiai sections ───

pub(crate) fn build_kiai_sections(
    timing_points: &[TimingPoint],
    chart_end_time: i64,
) -> Vec<KiaiSection> {
    let mut kiai_mode = false;
    let mut active_start: Option<i64> = None;

    for point in timing_points {
        if point.time > 0.0 {
            break;
        }
        kiai_mode = point.kiai_mode;
    }

    if kiai_mode {
        active_start = Some(0);
    }

    let mut sections: Vec<KiaiSection> = Vec::new();
    for point in timing_points {
        let point_time = pyround(point.time);
        if point_time <= 0 || point_time >= chart_end_time {
            continue;
        }
        if point.kiai_mode == kiai_mode {
            continue;
        }

        if kiai_mode {
            sections.push(KiaiSection {
                start_time: active_start.unwrap_or(0),
                end_time: point_time,
            });
            active_start = None;
        } else {
            active_start = Some(point_time);
        }

        kiai_mode = point.kiai_mode;
    }

    if kiai_mode {
        sections.push(KiaiSection {
            start_time: active_start.unwrap_or(0),
            end_time: chart_end_time,
        });
    }
    sections
}

// ─── timing lines ───

#[allow(clippy::too_many_arguments)]
fn merge_timing_line(
    line_by_time: &mut BTreeMap<i64, TimingLine>,
    time: i64,
    position: f64,
    is_measure: bool,
    show_label: bool,
    is_kiai: bool,
    is_kiai_start: bool,
    bpm: Option<f64>,
) {
    match line_by_time.get(&time) {
        None => {
            line_by_time.insert(
                time,
                TimingLine {
                    time,
                    position,
                    is_measure,
                    show_label,
                    is_kiai,
                    is_kiai_start,
                    bpm,
                },
            );
        }
        Some(existing) => {
            let merged = TimingLine {
                time,
                position: existing.position,
                is_measure: existing.is_measure || is_measure,
                show_label: existing.show_label || show_label,
                is_kiai: existing.is_kiai || is_kiai,
                is_kiai_start: existing.is_kiai_start || is_kiai_start,
                bpm: if existing.bpm.is_some() {
                    existing.bpm
                } else {
                    bpm
                },
            };
            line_by_time.insert(time, merged);
        }
    }
}

pub(crate) fn build_timing_lines(
    redline_sections: &[RedlineSection],
    mapper: &ScrollPositionMapper,
    min_beat_line_spacing: f64,
    kiai_sections: &[KiaiSection],
    first_note_time: i64,
) -> Vec<TimingLine> {
    let mut line_by_time: BTreeMap<i64, TimingLine> = BTreeMap::new();
    let mut last_bpm: Option<f64> = None;
    let mut deferred_first_bpm: Option<f64> = None;

    for section in redline_sections {
        let bpm = 60_000.0 / section.beat_length;
        let mut show_bpm = match last_bpm {
            None => true,
            Some(last) => (bpm - last).abs() > 0.01,
        };
        last_bpm = Some(bpm);

        if show_bpm && section.start_time == 0 && first_note_time > 0 {
            deferred_first_bpm = Some(bpm);
            show_bpm = false;
        }

        let mut beat_index: i64 = 0;
        let mut current_time = section.start_time as f64;
        while current_time <= section.end_time as f64 + 0.001 {
            let rounded_time = pyround(current_time);
            let next_time = current_time + section.beat_length;
            let beat_spacing = mapper.position_at(next_time.min(section.end_time as f64))
                - mapper.position_at(current_time);
            let is_measure = beat_index % (section.meter.max(1) as i64) == 0;
            let is_first_beat = beat_index == 0;

            if rounded_time >= 0
                && (is_measure || beat_spacing >= min_beat_line_spacing || (show_bpm && is_first_beat))
            {
                merge_timing_line(
                    &mut line_by_time,
                    rounded_time,
                    mapper.position_at(current_time),
                    is_measure,
                    true,
                    false,
                    false,
                    if show_bpm && is_first_beat {
                        Some(pyround(bpm) as f64)
                    } else {
                        None
                    },
                );
                if show_bpm && is_first_beat {
                    show_bpm = false;
                }
            }

            current_time = next_time;
            beat_index += 1;
        }
    }

    for section in kiai_sections {
        merge_timing_line(
            &mut line_by_time,
            section.start_time,
            mapper.position_at(section.start_time as f64),
            false,
            true,
            true,
            true,
            None,
        );
    }

    if let Some(bpm) = deferred_first_bpm {
        if first_note_time > 0 {
            merge_timing_line(
                &mut line_by_time,
                first_note_time,
                mapper.position_at(first_note_time as f64),
                false,
                true,
                false,
                false,
                Some(pyround(bpm) as f64),
            );
        }
    }

    dedupe_display_labels(apply_kiai_flags(&line_by_time, kiai_sections))
}

fn apply_kiai_flags(
    line_by_time: &BTreeMap<i64, TimingLine>,
    kiai_sections: &[KiaiSection],
) -> Vec<TimingLine> {
    let mut lines: Vec<TimingLine> = Vec::with_capacity(line_by_time.len());
    let mut kiai_index = 0usize;

    for (&time, line) in line_by_time {
        while kiai_index < kiai_sections.len() && kiai_sections[kiai_index].end_time <= time {
            kiai_index += 1;
        }

        let mut is_kiai = line.is_kiai;
        if kiai_index < kiai_sections.len() {
            let current = &kiai_sections[kiai_index];
            is_kiai = is_kiai || (current.start_time <= time && time < current.end_time);
        }

        let mut out = line.clone();
        out.is_kiai = is_kiai;
        lines.push(out);
    }

    lines
}

pub(crate) fn time_label_text(time: i64) -> String {
    format!("{:.1}s", time as f64 / 1000.0)
}

fn dedupe_display_labels(lines: Vec<TimingLine>) -> Vec<TimingLine> {
    let mut deduped: Vec<TimingLine> = Vec::with_capacity(lines.len());

    for line in lines {
        if !line.show_label || deduped.is_empty() {
            deduped.push(line);
            continue;
        }

        let previous = deduped.last().unwrap().clone();
        let same_label = time_label_text(previous.time) == time_label_text(line.time);
        if !previous.show_label || !same_label {
            deduped.push(line);
            continue;
        }

        if (line.is_kiai_start && !previous.is_kiai_start)
            || (line.bpm.is_some() && previous.bpm.is_none())
        {
            let last = deduped.last_mut().unwrap();
            last.show_label = false;
            last.bpm = None;
            deduped.push(line);
            continue;
        }

        let mut suppressed = line;
        suppressed.show_label = false;
        suppressed.bpm = None;
        deduped.push(suppressed);
    }

    deduped
}

// ─── SV changes ───

pub(crate) fn build_sv_changes(
    timing_points: &[TimingPoint],
    chart_end_time: i64,
    mapper: &ScrollPositionMapper,
) -> Vec<SvChange> {
    let inherited: Vec<&TimingPoint> = timing_points
        .iter()
        .filter(|tp| {
            !tp.uninherited
                && tp.beat_length < -0.001
                && tp.time >= 0.0
                && tp.time <= chart_end_time as f64
        })
        .collect();
    if inherited.is_empty() {
        return Vec::new();
    }

    let mut changes: Vec<SvChange> = Vec::new();
    let mut prev_sv: Option<f64> = None;
    for tp in inherited {
        let sv = -100.0 / tp.beat_length;
        if let Some(prev) = prev_sv {
            if (sv - prev).abs() <= 0.001 {
                continue;
            }
        }
        prev_sv = Some(sv);
        changes.push(SvChange {
            position: mapper.position_at(tp.time),
            sv,
        });
    }

    changes
}

// ─── shared mod helpers (renderer.py) ───

pub(crate) fn apply_taiko_object_mods(
    hit_objects: Vec<TaikoHitObject>,
    mods: Option<&ModSettings>,
) -> Vec<TaikoHitObject> {
    let swap = mods.map(|m| m.swap).unwrap_or(false);
    if !swap {
        return hit_objects;
    }

    // SW only swaps centre/rim for plain hits; rolls and swells are untouched.
    hit_objects
        .into_iter()
        .map(|hit_object| {
            if hit_object.hit_type & (DRUMROLL_FLAG | SWELL_FLAG) != 0 {
                return hit_object;
            }
            let mut hitsound = hit_object.hitsound;
            let is_rim = hitsound & HIT_SOUNDS_RIM != 0;
            if is_rim {
                hitsound &= !HIT_SOUNDS_RIM;
            } else {
                hitsound |= 8;
            }
            TaikoHitObject {
                start_time: hit_object.start_time,
                end_time: hit_object.end_time,
                hit_type: hit_object.hit_type,
                hitsound,
            }
        })
        .collect()
}

pub(crate) fn effective_slider_multiplier(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
) -> Result<f64> {
    let mut slider_multiplier = beatmap
        .difficulty
        .get_f64("SliderMultiplier")
        .ok_or_else(|| PreviewError::new("beatmap is missing SliderMultiplier"))?;
    if let Some(mods) = mods {
        if mods.easy {
            slider_multiplier *= 0.8;
        }
        if mods.hard_rock {
            slider_multiplier *= 1.4 * 4.0 / 3.0;
        }
    }
    Ok(slider_multiplier)
}

pub(crate) fn effective_timing_points(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
) -> Vec<TimingPoint> {
    if mods.map(|m| m.cs_override).unwrap_or(false) {
        // Constant Speed: keep only red lines, equivalent to disabling SV.
        return beatmap
            .timing_points
            .iter()
            .filter(|p| p.uninherited)
            .copied()
            .collect();
    }
    beatmap.timing_points.clone()
}

pub(crate) fn spacing_timing_points_for_png(timing_points: &[TimingPoint]) -> Vec<TimingPoint> {
    // Static chart spacing follows red-line BPM only; neutralize green-line SV
    // while keeping inherited point times and kiai flags.
    timing_points
        .iter()
        .map(|point| {
            if point.uninherited {
                *point
            } else {
                TimingPoint {
                    time: point.time,
                    beat_length: f64::NAN,
                    meter: point.meter,
                    uninherited: false,
                    kiai_mode: point.kiai_mode,
                }
            }
        })
        .collect()
}

pub(crate) fn taiko_hit_objects(beatmap: &Beatmap) -> Vec<TaikoHitObject> {
    beatmap.hit_objects.as_taiko().map(|v| v.to_vec()).unwrap_or_default()
}
