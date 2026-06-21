use crate::errors::{PreviewError, Result};
use crate::models::{Beatmap, BreakPeriod, TimingPoint};

pub const BREAK_GAP_MS: i64 = 2200;

#[derive(Debug, Clone)]
pub struct PreviewSegmentTiming {
    pub start_time: i64,
    pub is_preview: bool,
    pub break_periods: Vec<BreakPeriod>,
}

pub fn times_to_milliseconds(times: Option<&[f64]>) -> Option<Vec<i64>> {
    times.map(|ts| {
        ts.iter()
            .map(|t| crate::parser::round_half_even(t * 1000.0))
            .collect()
    })
}

/// Simple xorshift seeded from system time — replaces Python's unseeded
/// Mersenne Twister (selection was intentionally nondeterministic).
struct SimpleRng(u64);

impl SimpleRng {
    fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15)
            | 1;
        SimpleRng(seed)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn randrange(&mut self, n: i64) -> i64 {
        (self.next_u64() % n.max(1) as u64) as i64
    }
}

pub struct PreviewTimeSelector<'a> {
    beatmap: &'a Beatmap,
    spans: Vec<(i64, i64)>, // (start_time, end_time), sorted
    segment_count: usize,
    segment_duration: i64,
    requested_start_times: Vec<i64>,
}

impl<'a> PreviewTimeSelector<'a> {
    pub fn new(
        beatmap: &'a Beatmap,
        mut spans: Vec<(i64, i64)>,
        segment_count: usize,
        segment_duration: i64,
        requested_start_times: Option<Vec<i64>>,
    ) -> Result<Self> {
        if segment_count == 0 {
            return Err(PreviewError::new("segment count must be positive"));
        }
        if segment_duration < 0 {
            return Err(PreviewError::new("segment duration must be non-negative"));
        }
        if spans.is_empty() {
            return Err(PreviewError::new("beatmap has no hit objects"));
        }
        spans.sort_unstable();
        Ok(PreviewTimeSelector {
            beatmap,
            spans,
            segment_count,
            segment_duration,
            requested_start_times: requested_start_times.unwrap_or_default(),
        })
    }

    pub fn choose(&self) -> Result<Vec<PreviewSegmentTiming>> {
        let valid_intervals = self.build_valid_start_intervals();
        let preview_time = self.preview_time();
        let mut chosen = self.build_forced_times(preview_time)?;

        let mut rng = SimpleRng::new();
        let mut attempts = 0;
        while !valid_intervals.is_empty() && chosen.len() < self.segment_count && attempts < 3000 {
            attempts += 1;
            let candidate = random_start_from_intervals(&valid_intervals, &mut rng);
            if does_not_overlap_existing(candidate, self.segment_duration, &chosen) {
                chosen.push(candidate);
            }
        }

        if !valid_intervals.is_empty() && chosen.len() < self.segment_count {
            for candidate in self.fallback_start_candidates(&valid_intervals) {
                if does_not_overlap_existing(candidate, self.segment_duration, &chosen) {
                    chosen.push(candidate);
                }
                if chosen.len() == self.segment_count {
                    break;
                }
            }
        }

        chosen.sort_unstable();
        Ok(chosen
            .into_iter()
            .map(|start_time| PreviewSegmentTiming {
                start_time,
                is_preview: start_time == preview_time,
                break_periods: break_periods_overlapping_segment(
                    &self.beatmap.break_periods,
                    start_time,
                    self.segment_duration,
                ),
            })
            .collect())
    }

    fn build_forced_times(&self, preview_time: i64) -> Result<Vec<i64>> {
        let mut chosen: Vec<i64> = Vec::new();
        for &start_time in &self.requested_start_times {
            if start_time < 0 {
                return Err(PreviewError::new(format!(
                    "requested time must be non-negative, got {start_time}"
                )));
            }
            if !chosen.contains(&start_time) {
                chosen.push(start_time);
            }
        }
        if chosen.len() > self.segment_count {
            return Err(PreviewError::new(format!(
                "--times accepts at most {} time point{}",
                self.segment_count,
                if self.segment_count == 1 { "" } else { "s" }
            )));
        }
        if chosen.len() < self.segment_count && !chosen.contains(&preview_time) {
            chosen.push(preview_time);
        }
        Ok(chosen)
    }

    fn preview_time(&self) -> i64 {
        let preview_time: i64 = self
            .beatmap
            .general
            .get("PreviewTime")
            .and_then(|v| v.parse().ok())
            .unwrap_or(-1);
        if preview_time < 0 {
            self.spans[0].0
        } else {
            preview_time
        }
    }

    fn build_valid_start_intervals(&self) -> Vec<(i64, i64)> {
        let chart_start = self.spans[0].0;
        let chart_end = self.spans.iter().map(|s| s.1).max().unwrap();
        let mut forbidden = self.beatmap.break_periods.clone();
        forbidden.extend(infer_break_periods(&self.spans));
        let forbidden = merge_periods(forbidden);
        let playable = subtract_periods(chart_start, chart_end, &forbidden);

        playable
            .into_iter()
            .filter_map(|(start, end)| {
                let latest_start = end - self.segment_duration;
                if latest_start >= start {
                    Some((start, latest_start))
                } else {
                    None
                }
            })
            .collect()
    }

    fn fallback_start_candidates(&self, intervals: &[(i64, i64)]) -> Vec<i64> {
        let mut candidates: Vec<i64> = self
            .spans
            .iter()
            .map(|s| nearest_valid_start(s.0, intervals))
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

fn infer_break_periods(spans: &[(i64, i64)]) -> Vec<BreakPeriod> {
    let mut periods = Vec::new();
    let mut previous_end = spans[0].1;
    for span in &spans[1..] {
        if span.0 - previous_end >= BREAK_GAP_MS {
            periods.push(BreakPeriod {
                start_time: previous_end,
                end_time: span.0,
            });
        }
        previous_end = previous_end.max(span.1);
    }
    periods
}

fn merge_periods(mut periods: Vec<BreakPeriod>) -> Vec<BreakPeriod> {
    periods.sort_by_key(|p| (p.start_time, p.end_time));
    let mut merged: Vec<BreakPeriod> = Vec::new();
    for period in periods {
        match merged.last_mut() {
            Some(last) if period.start_time <= last.end_time => {
                last.end_time = last.end_time.max(period.end_time);
            }
            _ => merged.push(period),
        }
    }
    merged
}

fn subtract_periods(start_time: i64, end_time: i64, forbidden: &[BreakPeriod]) -> Vec<(i64, i64)> {
    let mut segments = Vec::new();
    let mut cursor = start_time;
    for period in forbidden {
        if period.end_time <= cursor {
            continue;
        }
        if period.start_time > cursor {
            segments.push((cursor, period.start_time.min(end_time)));
        }
        cursor = cursor.max(period.end_time);
        if cursor >= end_time {
            break;
        }
    }
    if cursor < end_time {
        segments.push((cursor, end_time));
    }
    segments.retain(|(s, e)| e > s);
    segments
}

fn nearest_valid_start(time: i64, intervals: &[(i64, i64)]) -> i64 {
    if intervals.iter().any(|&(s, e)| s <= time && time <= e) {
        return time;
    }
    intervals
        .iter()
        .map(|&(s, e)| if time < s { s } else { e })
        .min_by_key(|c| (c - time).abs())
        .unwrap_or(time)
}

fn random_start_from_intervals(intervals: &[(i64, i64)], rng: &mut SimpleRng) -> i64 {
    let total: i64 = intervals.iter().map(|(s, e)| e - s + 1).sum();
    let mut pick = rng.randrange(total);
    for &(start, end) in intervals {
        let length = end - start + 1;
        if pick < length {
            return start + pick;
        }
        pick -= length;
    }
    intervals.last().unwrap().1
}

fn does_not_overlap_existing(candidate: i64, segment_duration: i64, chosen: &[i64]) -> bool {
    let candidate_end = candidate + segment_duration;
    for &existing in chosen {
        let existing_end = existing + segment_duration;
        if candidate < existing_end && candidate_end > existing {
            return false;
        }
    }
    true
}

fn break_periods_overlapping_segment(
    break_periods: &[BreakPeriod],
    segment_start_time: i64,
    segment_duration: i64,
) -> Vec<BreakPeriod> {
    let segment_end_time = segment_start_time + segment_duration;
    break_periods
        .iter()
        .filter(|p| p.start_time < segment_end_time && p.end_time > segment_start_time)
        .copied()
        .collect()
}

/// Snap `time` backward to the nearest beat-line position on the red-line grid,
/// so that timing lines stay in phase after chart trimming.
pub fn snap_to_beat_grid(time: i64, timing_points: &[TimingPoint]) -> i64 {
    let red = timing_points
        .iter()
        .filter(|p| p.uninherited && p.beat_length > 0.0)
        .rfind(|p| (p.time as i64) <= time);
    let (red_time, beat_length) = match red {
        Some(p) => (p.time as i64, p.beat_length),
        None => return time.max(0),
    };
    if beat_length <= 0.0 {
        return time.max(0);
    }
    let beats_from_red = (time - red_time) as f64 / beat_length;
    (red_time + (beats_from_red.floor() as i64 as f64 * beat_length) as i64).max(0)
}
