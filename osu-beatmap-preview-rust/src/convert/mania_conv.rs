//! standard → mania conversion (mode 3).
//! Ported 1:1 from the Python beatmap_preview mania converter, which itself
//! ports osu!lazer's ManiaBeatmapConverter.
//! RNG call order and float32 round-trip points must match Python exactly.

use crate::errors::{PreviewError, Result};
use crate::legacy_random::LegacyRandom;
use crate::models::{Beatmap, HitObjects, KvSection, ManiaHitObject, StandardHitObject, TimingPoint};
use crate::mods::ModSettings;
use crate::parser::round_half_even;

use super::{kiai_at, std_objects};

const SOURCE_MODE_KEY: &str = "PreviewSourceMode";

// pattern type bit flags
const P_FORCE_NOT_STACK: u32 = 1 << 0;
const P_KEEP_SINGLE: u32 = 1 << 1;
const P_MIRROR: u32 = 1 << 2;
const P_GATHERED: u32 = 1 << 3;
const P_STAIR: u32 = 1 << 4;
const P_REVERSE: u32 = 1 << 5;
const P_CYCLE: u32 = 1 << 6;
const P_LOW_PROBABILITY: u32 = 1 << 7;
const P_FORCE_STACK: u32 = 1 << 8;
// extra bit for stair direction tracking (not part of pattern logic)
const P_REVERSE_STAIR: u32 = 1 << 9;

const HIT_WHISTLE: i32 = 1 << 1;
const HIT_FINISH: i32 = 1 << 2;
const HIT_CLAP: i32 = 1 << 3;

// ── pattern ──

#[derive(Clone)]
struct Pattern {
    columns: Vec<i32>,
    objects: Vec<ManiaHitObject>,
}

impl Pattern {
    fn new() -> Self {
        Pattern {
            columns: Vec::new(),
            objects: Vec::new(),
        }
    }

    fn has_column(&self, c: i32) -> bool {
        self.columns.contains(&c)
    }

    fn add(&mut self, col: i32, start_time: i64, end_time: i64) {
        if !self.columns.contains(&col) {
            self.columns.push(col);
        }
        self.objects.push(ManiaHitObject {
            lane: col,
            start_time,
            end_time,
            is_long_note: end_time > start_time,
        });
    }

    fn column_count(&self) -> i32 {
        self.columns.len() as i32
    }

    // Only meaningful when column_count == 1 (matching Python's set iteration use).
    fn any_column(&self) -> i32 {
        self.columns.first().copied().unwrap_or(0)
    }
}

// ── conversion state ──

struct ConversionState<'a> {
    rng: LegacyRandom,
    total_columns: i32,
    conv_diff: f64,
    timing_points: &'a [TimingPoint],
    slider_multiplier: f64,
    random_start: i32,
    stair_type: u32,
    prev_pattern: Pattern,
    prev_note_times: Vec<i64>,
    last_time: i64,
    last_x: i32,
    last_y: i32,
}

impl ConversionState<'_> {
    fn compute_density(&mut self, time: i64) {
        self.prev_note_times.push(time);
        if self.prev_note_times.len() > 7 {
            self.prev_note_times.remove(0);
        }
    }

    fn record_note(&mut self, time: i64, x: i32, y: i32) {
        self.last_time = time;
        self.last_x = x;
        self.last_y = y;
    }

    fn density(&self) -> f64 {
        if self.prev_note_times.len() < 2 {
            return 2147483647.0;
        }
        let first = self.prev_note_times[0];
        let last = self.prev_note_times[self.prev_note_times.len() - 1];
        (last - first) as f64 / self.prev_note_times.len() as f64
    }

    fn slider_segment_duration(&self, ho: &StandardHitObject) -> i64 {
        let span_count = i32::max(1, ho.slider_repeats) as i64;
        let (beat_length, slider_velocity) =
            mania_resolve_slider_timing(ho.start_time, self.timing_points);
        let adjusted_beat_length =
            beat_length * f64::max(10.0, f64::min(10000.0, 100.0 / slider_velocity)) / 100.0;
        let duration = (ho.start_time as f64
            + ho.slider_pixel_length * adjusted_beat_length * span_count as f64 * 0.01
                / self.slider_multiplier)
            .floor() as i64
            - ho.start_time;
        if span_count > 0 {
            duration.div_euclid(span_count)
        } else {
            duration
        }
    }

    fn get_column(&self, x: i32, allow_special: bool) -> i32 {
        if allow_special && self.total_columns == 8 {
            return i32::min(6, (x * 7).div_euclid(512)) + 1;
        }
        i32::min(self.total_columns - 1, (x * self.total_columns).div_euclid(512))
    }

    fn get_random_column(&mut self, lo: Option<i32>, hi: Option<i32>) -> i32 {
        self.rng.next_range(
            lo.unwrap_or(self.random_start),
            hi.unwrap_or(self.total_columns),
        )
    }

    fn find_available_column(
        &mut self,
        start: i32,
        patterns: &[&Pattern],
        lo: Option<i32>,
        hi: Option<i32>,
        gathered: bool,
        not_equal: Option<i32>,
    ) -> Result<i32> {
        let lo = lo.unwrap_or(self.random_start);
        let hi = hi.unwrap_or(self.total_columns);

        let ok = |c: i32| -> bool {
            if let Some(ne) = not_equal {
                if c == ne {
                    return false;
                }
            }
            patterns.iter().all(|p| !p.has_column(c))
        };

        if lo <= start && start < hi && ok(start) {
            return Ok(start);
        }
        if !(lo..hi).any(&ok) {
            return Err(PreviewError::new(
                "not enough columns to complete mania conversion",
            ));
        }

        let mut col = start;
        loop {
            col = if gathered {
                let mut c = col + 1;
                if c == self.total_columns {
                    c = self.random_start;
                }
                c
            } else {
                self.get_random_column(Some(lo), Some(hi))
            };
            if ok(col) {
                return Ok(col);
            }
        }
    }

    // Inverse cumulative probability: val >= 1-pN → N (highest match wins).
    fn get_random_note_count(&mut self, p2: f64, p3: f64, p4: f64, p5: f64) -> i32 {
        let val = self.rng.next_double();
        if p5 != 0.0 && val >= 1.0 - p5 {
            return 5;
        }
        if p4 != 0.0 && val >= 1.0 - p4 {
            return 4;
        }
        if p3 != 0.0 && val >= 1.0 - p3 {
            return 3;
        }
        if p2 != 0.0 && val >= 1.0 - p2 {
            return 2;
        }
        1
    }
}

// ── public API ──

pub(crate) fn mania_convert(
    beatmap: &Beatmap,
    target_mode: i32,
    mods: Option<&ModSettings>,
) -> Result<Beatmap> {
    if beatmap.mode() != 0 {
        return Err(PreviewError::new("source beatmap must be osu!standard (mode=0)"));
    }
    if target_mode != 3 {
        return Err(PreviewError::new(
            "only mania (mode=3) conversion is currently supported",
        ));
    }

    let objects = std_objects(beatmap);
    if objects.is_empty() {
        return Err(PreviewError::new("standard beatmap has no hit objects to convert"));
    }

    let diff = &beatmap.difficulty;
    let total_columns = mania_resolve_total_columns(objects, diff, mods);
    let seed = mania_build_seed(diff);
    let rng = LegacyRandom::new(seed as u32);
    let conv_diff = mania_compute_conversion_difficulty(objects, beatmap, diff);

    let mut state = ConversionState {
        rng,
        total_columns,
        conv_diff,
        timing_points: &beatmap.timing_points,
        slider_multiplier: diff.get_f64_or("SliderMultiplier", 1.4),
        random_start: if total_columns == 8 { 1 } else { 0 },
        stair_type: P_STAIR,
        prev_pattern: Pattern::new(),
        prev_note_times: Vec::new(),
        last_time: 0,
        last_x: 0,
        last_y: 0,
    };
    let mania_objects = mania_convert_all(&mut state, objects)?;

    let mut new_diff = diff.clone();
    new_diff.insert("CircleSize", total_columns.to_string());
    let mut new_general = beatmap.general.clone();
    new_general.insert(SOURCE_MODE_KEY, beatmap.mode().to_string());
    new_general.insert("Mode", "3".to_string());

    Ok(Beatmap {
        metadata: beatmap.metadata.clone(),
        difficulty: new_diff,
        general: new_general,
        timing_points: beatmap.timing_points.clone(),
        hit_objects: HitObjects::Mania(mania_objects),
        break_periods: beatmap.break_periods.clone(),
        combo_colors: beatmap.combo_colors.clone(),
    })
}

fn mania_resolve_total_columns(
    hit_objects: &[StandardHitObject],
    difficulty: &KvSection,
    mods: Option<&ModSettings>,
) -> i32 {
    if let Some(m) = mods {
        if let Some(keys) = m.mania_keys {
            let mut cols = keys;
            if m.dual_stage {
                cols = i32::min(18, cols * 2);
            }
            return cols;
        }
    }

    let cs = difficulty.get_f64_or("CircleSize", 4.0);
    let od = difficulty.get_f64_or("OverallDifficulty", 8.0);
    let rounded_cs = round_half_even(cs);
    let rounded_od = round_half_even(od);
    let total = hit_objects.len();
    let end_time_obj = hit_objects
        .iter()
        .filter(|ho| ho.end_time > ho.start_time)
        .count();
    let ratio = if total > 0 {
        end_time_obj as f64 / total as f64
    } else {
        0.0
    };

    let mut cols: i32 = if ratio < 0.2 {
        7
    } else if ratio < 0.3 || rounded_cs >= 5 {
        if rounded_od > 5 {
            7
        } else {
            6
        }
    } else if ratio > 0.6 {
        if rounded_od > 4 {
            5
        } else {
            4
        }
    } else {
        i64::max(4, i64::min(7, rounded_od + 1)) as i32
    };

    if let Some(m) = mods {
        if m.dual_stage {
            cols = i32::min(18, cols * 2);
        }
    }
    cols
}

fn mania_build_seed(difficulty: &KvSection) -> i64 {
    let cs = difficulty.get_f64_or("CircleSize", 4.0);
    let od = difficulty.get_f64_or("OverallDifficulty", 8.0);
    let ar = difficulty.get_f64_or("ApproachRate", 5.0);
    let dr = difficulty.get_f64_or("HPDrainRate", 5.0);
    round_half_even(dr + cs) * 20 + (od * 41.2) as i64 + round_half_even(ar)
}

fn mania_compute_conversion_difficulty(
    hit_objects: &[StandardHitObject],
    beatmap: &Beatmap,
    difficulty: &KvSection,
) -> f64 {
    let total_break: i64 = beatmap
        .break_periods
        .iter()
        .map(|b| b.end_time - b.start_time)
        .sum();
    let first_time = hit_objects[0].start_time;
    let last_time = hit_objects[hit_objects.len() - 1].start_time;
    let drain_time_int = ((last_time - first_time - total_break) as f64 / 1000.0) as i64;
    let drain_time = if drain_time_int == 0 {
        10000.0
    } else {
        drain_time_int as f64
    };

    let dr = difficulty.get_f64_or("HPDrainRate", 5.0);
    let ar = difficulty.get_f64_or("ApproachRate", 5.0);
    let clamped_ar = f64::max(4.0, f64::min(7.0, ar));
    let obj_density = hit_objects.len() as f64 / drain_time;

    let cd = ((dr + clamped_ar) / 1.5 + obj_density * 9.0) / 38.0 * 5.0 / 1.15;
    f64::min(cd, 12.0)
}

fn mania_resolve_slider_timing(start_time: i64, timing_points: &[TimingPoint]) -> (f64, f64) {
    let mut beat_length = if !timing_points.is_empty() {
        timing_points[0].beat_length
    } else {
        500.0
    };
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

// ── main conversion loop ──

fn mania_convert_all(
    s: &mut ConversionState,
    hit_objects: &[StandardHitObject],
) -> Result<Vec<ManiaHitObject>> {
    let mut result: Vec<ManiaHitObject> = Vec::new();
    for ho in hit_objects {
        if ho.end_time > ho.start_time && ho.slider_type.is_some() {
            let seg_dur = s.slider_segment_duration(ho);
            for i in 0..(ho.slider_repeats + 1) {
                let time = ho.start_time + seg_dur * i as i64;
                s.record_note(time, ho.x, ho.y);
                s.compute_density(time);
            }
            result.extend(slider_generate(s, ho)?);
        } else if ho.end_time > ho.start_time {
            s.record_note(ho.end_time, 256, 192);
            s.compute_density(ho.end_time);
            result.extend(spinner_generate(s, ho)?);
        } else {
            let time_gap = ho.start_time - s.last_time;
            let pos_gap = ((ho.x - s.last_x) as f64).hypot((ho.y - s.last_y) as f64);
            s.compute_density(ho.start_time);
            result.extend(circle_generate(s, ho, time_gap, pos_gap)?);
            s.record_note(ho.start_time, ho.x, ho.y);
        }
    }
    Ok(result)
}

// ── circle generator ──

fn circle_resolve_convert_type(
    s: &ConversionState,
    ho: &StandardHitObject,
    time_gap: i64,
    pos_gap: f64,
) -> u32 {
    let mut ct: u32 = 0;
    let t_cols = s.total_columns;
    let beat_len = circle_beat_length_at(s.timing_points, ho.start_time);
    let density = s.density();

    if time_gap <= 80 {
        ct |= P_FORCE_NOT_STACK | P_KEEP_SINGLE;
    } else if time_gap <= 95 {
        ct |= P_FORCE_NOT_STACK | P_KEEP_SINGLE | s.stair_type;
    } else if time_gap <= 105 {
        ct |= P_FORCE_NOT_STACK | P_LOW_PROBABILITY;
    } else if time_gap <= 125 {
        ct |= P_FORCE_NOT_STACK;
    } else if time_gap <= 135 && pos_gap < 20.0 {
        ct |= P_CYCLE | P_KEEP_SINGLE;
    } else if time_gap <= 150 && pos_gap < 20.0 {
        ct |= P_FORCE_STACK | P_LOW_PROBABILITY;
    } else if pos_gap < 20.0 && density >= beat_len / 2.5 {
        ct |= P_REVERSE | P_LOW_PROBABILITY;
    } else if density < beat_len / 2.5 || kiai_at(ho.start_time, s.timing_points) {
        // high density, no special flag
    } else {
        ct |= P_LOW_PROBABILITY;
    }

    if ct & P_KEEP_SINGLE == 0 {
        if (ho.hitsound & HIT_FINISH != 0) && t_cols != 8 {
            ct |= P_MIRROR;
        } else if ho.hitsound & HIT_CLAP != 0 {
            ct |= P_GATHERED;
        }
    }

    ct
}

fn circle_beat_length_at(timing_points: &[TimingPoint], time: i64) -> f64 {
    let mut base = 500.0;
    for tp in timing_points {
        if tp.uninherited && tp.time <= time as f64 {
            base = tp.beat_length;
        }
    }
    base
}

fn circle_generate(
    s: &mut ConversionState,
    ho: &StandardHitObject,
    time_gap: i64,
    pos_gap: f64,
) -> Result<Vec<ManiaHitObject>> {
    let ct = circle_resolve_convert_type(s, ho, time_gap, pos_gap);
    let t_cols = s.total_columns;
    let t = ho.start_time;
    let mut pattern = Pattern::new();
    let prev = s.prev_pattern.clone();

    if t_cols <= 1 {
        pattern.add(0, t, t);
        s.prev_pattern = pattern;
        return Ok(s.prev_pattern.objects.clone());
    }

    // Reverse
    if ct & P_REVERSE != 0 && prev.column_count() > 0 {
        for c in s.random_start..t_cols {
            if prev.has_column(c) {
                pattern.add(s.random_start + t_cols - c - 1, t, t);
            }
        }
        s.prev_pattern = pattern;
        return Ok(s.prev_pattern.objects.clone());
    }

    // Cycle
    if ct & P_CYCLE != 0
        && prev.column_count() == 1
        && !(t_cols == 8 && prev.any_column() == 0)
        && !(t_cols % 2 == 1 && prev.any_column() == t_cols / 2)
    {
        let col = s.random_start + t_cols - prev.any_column() - 1;
        pattern.add(col, t, t);
        s.prev_pattern = pattern;
        return Ok(s.prev_pattern.objects.clone());
    }

    // ForceStack
    if ct & P_FORCE_STACK != 0 && prev.column_count() > 0 {
        for c in s.random_start..t_cols {
            if prev.has_column(c) {
                pattern.add(c, t, t);
            }
        }
        s.prev_pattern = pattern;
        return Ok(s.prev_pattern.objects.clone());
    }

    // Stair (from previous)
    if prev.column_count() == 1 {
        let last_col = prev.any_column();
        if ct & P_STAIR != 0 {
            let mut col = last_col + 1;
            if col >= t_cols {
                col = s.random_start;
            }
            pattern.add(col, t, t);
            if col == t_cols - 1 {
                s.stair_type = P_REVERSE_STAIR;
            }
            s.prev_pattern = pattern;
            return Ok(s.prev_pattern.objects.clone());
        }
        if ct & P_REVERSE_STAIR != 0 {
            let mut col = last_col - 1;
            if col < s.random_start {
                col = t_cols - 1;
            }
            pattern.add(col, t, t);
            if col == s.random_start {
                s.stair_type = P_STAIR;
            }
            s.prev_pattern = pattern;
            return Ok(s.prev_pattern.objects.clone());
        }
    }

    // KeepSingle
    if ct & P_KEEP_SINGLE != 0 {
        circle_gen_random_notes(s, &mut pattern, 1, ct, ho, &prev)?;
        s.prev_pattern = pattern;
        return Ok(s.prev_pattern.objects.clone());
    }

    // Mirror
    if ct & P_MIRROR != 0 {
        circle_gen_mirrored(s, &mut pattern, ct, ho, &prev)?;
        s.prev_pattern = pattern;
        return Ok(s.prev_pattern.objects.clone());
    }

    // Random with conversion difficulty
    let cd = s.conv_diff;
    let lp = ct & P_LOW_PROBABILITY != 0;
    let (p2, p3): (f64, f64) = if cd > 6.5 {
        if lp {
            (0.78, 0.42)
        } else {
            (1.0, 0.62)
        }
    } else if cd > 4.0 {
        if lp {
            (0.35, 0.08)
        } else {
            (0.52, 0.15)
        }
    } else if cd > 2.0 {
        if lp {
            (0.18, 0.0)
        } else {
            (0.45, 0.0)
        }
    } else {
        (0.0, 0.0)
    };

    let note_count = circle_get_random_note_count(s, ho, p2, p3, 0.0, 0.0);
    circle_gen_random_notes(s, &mut pattern, note_count, ct, ho, &prev)?;
    circle_add_special_column(s, &mut pattern, ho);
    s.prev_pattern = pattern;
    Ok(s.prev_pattern.objects.clone())
}

fn circle_gen_random_notes(
    s: &mut ConversionState,
    pattern: &mut Pattern,
    count: i32,
    ct: u32,
    ho: &StandardHitObject,
    prev: &Pattern,
) -> Result<()> {
    let allow_stack = ct & P_FORCE_NOT_STACK == 0;
    let mut count = count;

    if !allow_stack {
        let occupied = prev.column_count();
        count = i32::min(count, s.total_columns - s.random_start - occupied);
    }

    let mut col = s.get_column(ho.x, true);
    let gathered = ct & P_GATHERED != 0;

    for _ in 0..count {
        col = if allow_stack {
            s.find_available_column(col, &[&*pattern], None, None, gathered, None)?
        } else {
            s.find_available_column(col, &[&*pattern, prev], None, None, gathered, None)?
        };
        pattern.add(col, ho.start_time, ho.start_time);
    }
    Ok(())
}

fn circle_gen_mirrored(
    s: &mut ConversionState,
    pattern: &mut Pattern,
    ct: u32,
    ho: &StandardHitObject,
    prev: &Pattern,
) -> Result<()> {
    let t_cols = s.total_columns;
    let cd = s.conv_diff;

    if ct & P_FORCE_NOT_STACK != 0 {
        let (p2, p3, p4, p5): (f64, f64, f64, f64) = if cd > 6.5 {
            (0.5 + 0.38 / 2.0, 0.38, (0.38 + 0.12) / 2.0, 0.12)
        } else if cd > 4.0 {
            (0.5 + 0.17 / 2.0, 0.17, 0.17 / 2.0, 0.0)
        } else {
            (0.5, 0.0, 0.0, 0.0)
        };
        let nc = circle_get_random_note_count(s, ho, p2, p3, p4, p5);
        circle_gen_random_notes(s, pattern, nc, ct, ho, prev)?;
        circle_add_special_column(s, pattern, ho);
        return Ok(());
    }

    let mut centre_p: f64 = 0.12;
    let (mut p2, mut p3): (f64, f64) = if cd > 6.5 {
        (0.38, 0.12)
    } else if cd > 4.0 {
        (0.17, 0.0)
    } else {
        (0.0, 0.0)
    };

    // cap mirrored probabilities per key count
    if t_cols == 2 {
        centre_p = 0.0;
        p2 = 0.0;
        p3 = 0.0;
    } else if t_cols == 3 {
        centre_p = f64::min(centre_p, 0.03);
        p2 = 0.0;
        p3 = 0.0;
    } else if t_cols == 4 {
        centre_p = 0.0;
        p2 = 1.0 - f64::max((1.0 - p2) * 2.0, 0.8);
    } else if t_cols == 5 {
        centre_p = f64::min(centre_p, 0.03);
        p3 = 0.0;
    } else if t_cols == 6 {
        centre_p = 0.0;
        p2 = 1.0 - f64::max((1.0 - p2) * 2.0, 0.5);
        p3 = 1.0 - f64::max((1.0 - p3) * 2.0, 0.85);
    }

    p2 = f64::max(0.0, f64::min(1.0, p2));
    p3 = f64::max(0.0, f64::min(1.0, p3));
    let centre_val = s.rng.next_double();
    let note_count = s.get_random_note_count(p2, p3, 0.0, 0.0);
    let add_centre = t_cols % 2 == 1 && note_count != 3 && centre_val > 1.0 - centre_p;

    let half = (if t_cols % 2 == 0 { t_cols } else { t_cols - 1 }) / 2;
    let mut col = s.get_random_column(None, Some(s.random_start + half));
    for _ in 0..note_count {
        col = s.find_available_column(
            col,
            &[&*pattern],
            None,
            Some(s.random_start + half),
            false,
            None,
        )?;
        pattern.add(col, ho.start_time, ho.start_time);
        pattern.add(
            s.random_start + t_cols - col - 1,
            ho.start_time,
            ho.start_time,
        );
    }

    if add_centre {
        pattern.add(t_cols / 2, ho.start_time, ho.start_time);
    }

    circle_add_special_column(s, pattern, ho);
    Ok(())
}

fn circle_add_special_column(s: &ConversionState, pattern: &mut Pattern, ho: &StandardHitObject) {
    if s.random_start > 0
        && (ho.hitsound & HIT_CLAP != 0)
        && (ho.hitsound & HIT_FINISH != 0)
        && !pattern.has_column(0)
    {
        pattern.add(0, ho.start_time, ho.start_time);
    }
}

fn circle_cap_note_counts(t_cols: i32, p2: f64, p3: f64, p4: f64, p5: f64) -> (f64, f64, f64, f64) {
    if t_cols == 2 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    if t_cols == 3 {
        return (f64::min(p2, 0.1), 0.0, 0.0, 0.0);
    }
    if t_cols == 4 {
        return (f64::min(p2, 0.23), f64::min(p3, 0.04), 0.0, 0.0);
    }
    if t_cols == 5 {
        return (p2, f64::min(p3, 0.15), f64::min(p4, 0.03), 0.0);
    }
    (p2, p3, p4, p5)
}

fn circle_get_random_note_count(
    s: &mut ConversionState,
    ho: &StandardHitObject,
    p2: f64,
    p3: f64,
    p4: f64,
    p5: f64,
) -> i32 {
    let (mut p2, p3, p4, p5) = circle_cap_note_counts(s.total_columns, p2, p3, p4, p5);
    if ho.hitsound & HIT_CLAP != 0 {
        p2 = 1.0;
    }
    s.get_random_note_count(p2, p3, p4, p5)
}

// ── slider generator ──

struct SliderCtx {
    start_time: i64,
    spans: i32,
    seg_dur: i64,
    end_time: i64,
    duration: i64,
    convert_type: u32,
}

fn slider_generate(s: &mut ConversionState, ho: &StandardHitObject) -> Result<Vec<ManiaHitObject>> {
    let spans = i32::max(1, ho.slider_repeats);
    let seg_dur = s.slider_segment_duration(ho);
    let end_time = ho.start_time + seg_dur * spans as i64;
    let mut ctx = SliderCtx {
        start_time: ho.start_time,
        spans,
        seg_dur,
        end_time,
        duration: end_time - ho.start_time,
        convert_type: if kiai_at(ho.start_time, s.timing_points) {
            0
        } else {
            P_LOW_PROBABILITY
        },
    };

    let pattern = if ctx.spans > 1 {
        slider_gen_multi_span(s, ho, &mut ctx)?
    } else {
        slider_gen_single_span(s, ho, &mut ctx)?
    };

    Ok(slider_split_patterns(s, &ctx, pattern))
}

fn slider_split_patterns(
    s: &mut ConversionState,
    ctx: &SliderCtx,
    pattern: Pattern,
) -> Vec<ManiaHitObject> {
    if pattern.objects.len() == 1 {
        s.prev_pattern = pattern;
        return s.prev_pattern.objects.clone();
    }

    let mut intermediate = Pattern::new();
    let mut end_pattern = Pattern::new();
    for obj in &pattern.objects {
        if ctx.end_time != obj.end_time {
            intermediate.add(obj.lane, obj.start_time, obj.end_time);
        } else {
            end_pattern.add(obj.lane, obj.start_time, obj.end_time);
        }
    }

    let mut out = intermediate.objects;
    out.extend(end_pattern.objects.iter().copied());
    s.prev_pattern = end_pattern;
    out
}

fn slider_gen_multi_span(
    s: &mut ConversionState,
    ho: &StandardHitObject,
    ctx: &mut SliderCtx,
) -> Result<Pattern> {
    let t_cols = s.total_columns;
    let seg = ctx.seg_dur;

    if seg <= 90 {
        return slider_gen_holds(s, ctx, 1);
    }
    if seg <= 120 {
        ctx.convert_type |= P_FORCE_NOT_STACK;
        return slider_gen_notes_no_stack(s, ho, ctx, ctx.spans + 1);
    }
    if seg <= 160 {
        return slider_gen_stair(s, ho, ctx);
    }
    if seg <= 200 && s.conv_diff > 3.0 {
        return slider_gen_random_multiple(s, ho, ctx);
    }

    if ctx.duration >= 4000 {
        return slider_gen_n_random_notes(s, ho, ctx, 0.23, 0.0, 0.0);
    }

    if seg > 400 && ctx.spans < t_cols - 1 - s.random_start {
        return slider_gen_tiled_holds(s, ho, ctx);
    }

    slider_gen_hold_and_normal(s, ho, ctx)
}

fn slider_gen_single_span(
    s: &mut ConversionState,
    ho: &StandardHitObject,
    ctx: &mut SliderCtx,
) -> Result<Pattern> {
    let t_cols = s.total_columns;
    let cd = s.conv_diff;
    let lp = ctx.convert_type & P_LOW_PROBABILITY != 0;
    let seg = ctx.seg_dur;

    if seg <= 110 {
        if s.prev_pattern.column_count() < t_cols {
            ctx.convert_type |= P_FORCE_NOT_STACK;
        } else {
            ctx.convert_type &= !P_FORCE_NOT_STACK;
        }
        return slider_gen_notes_no_stack(s, ho, ctx, if seg < 80 { 1 } else { 2 });
    }

    // graded by ConversionDifficulty
    let (p2, p3): (f64, f64) = if cd > 6.5 {
        if lp {
            (0.78, 0.3)
        } else {
            (0.85, 0.36)
        }
    } else if cd > 4.0 {
        if lp {
            (0.43, 0.08)
        } else {
            (0.56, 0.18)
        }
    } else if cd > 2.5 {
        if lp {
            (0.3, 0.0)
        } else {
            (0.37, 0.08)
        }
    } else if lp {
        (0.17, 0.0)
    } else {
        (0.27, 0.0)
    };

    let (p2, p3, _p4) = slider_cap_hold_counts(t_cols, p2, p3, 0.0);
    slider_gen_n_random_notes(s, ho, ctx, p2, p3, 0.0)
}

fn slider_gen_holds(s: &mut ConversionState, ctx: &SliderCtx, count: i32) -> Result<Pattern> {
    let t_cols = s.total_columns;
    let prev = s.prev_pattern.clone();
    let usable = t_cols - s.random_start - prev.column_count();
    let count = i32::min(count, t_cols - s.random_start);
    let mut pattern = Pattern::new();
    let mut col = s.get_random_column(None, None);
    let n1 = i32::min(usable, count);
    for _ in 0..n1 {
        col = s.find_available_column(col, &[&pattern, &prev], None, None, false, None)?;
        pattern.add(col, ctx.start_time, ctx.end_time);
    }
    for _ in 0..(count - n1) {
        col = s.find_available_column(col, &[&pattern], None, None, false, None)?;
        pattern.add(col, ctx.start_time, ctx.end_time);
    }
    Ok(pattern)
}

fn slider_gen_notes_no_stack(
    s: &mut ConversionState,
    ho: &StandardHitObject,
    ctx: &SliderCtx,
    count: i32,
) -> Result<Pattern> {
    let t_cols = s.total_columns;
    let prev = s.prev_pattern.clone();
    let mut pattern = Pattern::new();
    let mut col = s.get_column(ho.x, true);
    if ctx.convert_type & P_FORCE_NOT_STACK != 0 && prev.column_count() < t_cols {
        col = s.find_available_column(col, &[&prev], None, None, false, None)?;
    }

    let mut last_col = col;
    for i in 0..count {
        let t = ctx.start_time + i as i64 * ctx.seg_dur;
        pattern.add(col, t, t);
        col = s.find_available_column(col, &[], None, None, false, Some(last_col))?;
        last_col = col;
    }
    Ok(pattern)
}

fn slider_gen_stair(
    s: &mut ConversionState,
    ho: &StandardHitObject,
    ctx: &SliderCtx,
) -> Result<Pattern> {
    let t_cols = s.total_columns;
    let mut col = s.get_column(ho.x, true);
    let mut increasing = s.rng.next_double() > 0.5;
    let mut pattern = Pattern::new();
    for i in 0..(ctx.spans + 1) {
        let t = ctx.start_time + i as i64 * ctx.seg_dur;
        pattern.add(col, t, t);
        if increasing {
            if col >= t_cols - 1 {
                increasing = false;
                col -= 1;
            } else {
                col += 1;
            }
        } else if col <= s.random_start {
            increasing = true;
            col += 1;
        } else {
            col -= 1;
        }
    }
    Ok(pattern)
}

fn slider_gen_random_multiple(
    s: &mut ConversionState,
    ho: &StandardHitObject,
    ctx: &SliderCtx,
) -> Result<Pattern> {
    let t_cols = s.total_columns;
    let legacy = (4..=8).contains(&t_cols);
    let interval = s
        .rng
        .next_range(1, t_cols - if legacy { 1 } else { 0 });
    let mut col = s.get_column(ho.x, true);
    let mut pattern = Pattern::new();
    for i in 0..(ctx.spans + 1) {
        let t = ctx.start_time + i as i64 * ctx.seg_dur;
        pattern.add(col, t, t);
        let mut col2 = col + interval;
        if col2 >= t_cols - s.random_start {
            col2 = col2 - t_cols - s.random_start + if legacy { 1 } else { 0 };
        }
        col2 += s.random_start;
        if t_cols > 2 {
            pattern.add(col2, t, t);
        }
        col = s.get_random_column(None, None);
    }
    Ok(pattern)
}

fn slider_gen_tiled_holds(
    s: &mut ConversionState,
    ho: &StandardHitObject,
    ctx: &SliderCtx,
) -> Result<Pattern> {
    let t_cols = s.total_columns;
    let col_repeat = i32::min(ctx.spans, t_cols);
    let prev = s.prev_pattern.clone();
    let mut col = s.get_column(ho.x, true);
    if ctx.convert_type & P_FORCE_NOT_STACK != 0 && prev.column_count() < t_cols {
        col = s.find_available_column(col, &[&prev], None, None, false, None)?;
    }

    let mut pattern = Pattern::new();
    for i in 0..col_repeat {
        let t = ctx.start_time + i as i64 * ctx.seg_dur;
        col = s.find_available_column(col, &[&pattern], None, None, false, None)?;
        pattern.add(col, t, ctx.end_time);
    }
    Ok(pattern)
}

fn slider_gen_hold_and_normal(
    s: &mut ConversionState,
    ho: &StandardHitObject,
    ctx: &SliderCtx,
) -> Result<Pattern> {
    let t_cols = s.total_columns;
    let cd = s.conv_diff;
    let prev = s.prev_pattern.clone();
    let mut col = s.get_column(ho.x, true);
    if ctx.convert_type & P_FORCE_NOT_STACK != 0 && prev.column_count() < t_cols {
        col = s.find_available_column(col, &[&prev], None, None, false, None)?;
    }

    let mut pattern = Pattern::new();
    pattern.add(col, ctx.start_time, ctx.end_time); // hold

    // accompanying note count
    let mut nc = if cd > 6.5 {
        s.get_random_note_count(0.63, 0.0, 0.0, 0.0)
    } else if cd > 4.0 {
        s.get_random_note_count(if t_cols < 6 { 0.12 } else { 0.45 }, 0.0, 0.0, 0.0)
    } else if cd > 2.5 {
        s.get_random_note_count(if t_cols < 6 { 0.0 } else { 0.24 }, 0.0, 0.0, 0.0)
    } else {
        0
    };
    nc = i32::min(t_cols - 1, nc);

    let mut next_col = s.get_random_column(None, None);
    let mut row = Pattern::new();
    let ignore_head =
        slider_hitsound_at(ho, ctx, ctx.start_time) & (HIT_WHISTLE | HIT_FINISH | HIT_CLAP) == 0;

    for i in 0..(ctx.spans + 1) {
        let t = ctx.start_time + i as i64 * ctx.seg_dur;
        if !(ignore_head && t == ctx.start_time) {
            for _ in 0..nc {
                next_col = s.find_available_column(next_col, &[&row], None, None, false, Some(col))?;
                row.add(next_col, t, t);
            }
        }
        for obj in row.objects.clone() {
            pattern.add(obj.lane, obj.start_time, obj.end_time);
        }
        row = Pattern::new();
    }

    Ok(pattern)
}

fn slider_gen_n_random_notes(
    s: &mut ConversionState,
    ho: &StandardHitObject,
    ctx: &SliderCtx,
    p2: f64,
    p3: f64,
    p4: f64,
) -> Result<Pattern> {
    let mut can_generate_two_notes = ctx.convert_type & P_LOW_PROBABILITY == 0;
    can_generate_two_notes = can_generate_two_notes
        && ((ho.hitsound & (HIT_CLAP | HIT_FINISH) != 0)
            || (slider_hitsound_at(ho, ctx, ctx.start_time) & (HIT_CLAP | HIT_FINISH) != 0));
    let p2 = if can_generate_two_notes { 1.0 } else { p2 };
    let nc = s.get_random_note_count(p2, p3, p4, 0.0);
    slider_gen_holds(s, ctx, nc)
}

fn slider_hitsound_at(ho: &StandardHitObject, ctx: &SliderCtx, time: i64) -> i32 {
    if ho.slider_edge_hitsounds.is_empty() {
        return ho.hitsound;
    }
    let index: i64 = if ctx.seg_dur == 0 {
        0
    } else {
        (time - ctx.start_time).div_euclid(ctx.seg_dur)
    };
    let index = i64::max(0, i64::min(index, ho.slider_edge_hitsounds.len() as i64 - 1));
    ho.slider_edge_hitsounds[index as usize]
}

fn slider_cap_hold_counts(t_cols: i32, p2: f64, p3: f64, p4: f64) -> (f64, f64, f64) {
    if t_cols == 2 {
        return (0.0, 0.0, 0.0);
    }
    if t_cols == 3 {
        return (f64::min(p2, 0.1), 0.0, 0.0);
    }
    if t_cols == 4 {
        return (f64::min(p2, 0.3), f64::min(p3, 0.04), 0.0);
    }
    if t_cols == 5 {
        return (f64::min(p2, 0.34), f64::min(p3, 0.1), f64::min(p4, 0.03));
    }
    (p2, p3, p4)
}

// ── spinner generator ──

fn spinner_generate(s: &mut ConversionState, ho: &StandardHitObject) -> Result<Vec<ManiaHitObject>> {
    let t_cols = s.total_columns;
    let dur = ho.end_time - ho.start_time;
    let is_hold = dur >= 100;
    let prev = s.prev_pattern.clone();
    let force_not = prev.column_count() < t_cols;

    let col = if force_not {
        let start = s.get_random_column(None, None);
        s.find_available_column(start, &[&prev], None, None, false, None)?
    } else {
        s.get_random_column(Some(0), None)
    };

    let end = if is_hold { ho.end_time } else { ho.start_time };
    let mut pattern = Pattern::new();
    pattern.add(col, ho.start_time, end);
    Ok(pattern.objects)
}
