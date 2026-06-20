//! standard → taiko/catch/mania conversion, ported 1:1 from the Python
//! beatmap_preview convert modules (which themselves port osu!lazer converters).
//! RNG call order and float32 round-trip points must match Python exactly.

mod catch_conv;
mod mania_conv;
mod taiko_conv;

use crate::errors::{PreviewError, Result};
use crate::models::{Beatmap, HitObjects, StandardHitObject, TimingPoint};
use crate::mods::ModSettings;

pub(crate) use catch_conv::catch_convert;
pub(crate) use mania_conv::mania_convert;
pub(crate) use taiko_conv::taiko_convert;

// ── timing-point cursor ──────────────────────────────────────────────────────
///
/// Monotonically advances through sorted timing points as objects are processed
/// in time order, avoiding O(n×m) repeated linear scans.  Each converter creates
/// one cursor and calls `advance_to(time)` before querying the cached fields.
pub(crate) struct TimingCursor<'a> {
    points: &'a [TimingPoint],
    index: usize,
    pub beat_length: f64,
    pub slider_velocity: f64,
    pub meter: i32,
    pub kiai: bool,
}

impl<'a> TimingCursor<'a> {
    pub fn new(points: &'a [TimingPoint]) -> Self {
        let mut c = Self {
            points,
            index: 0,
            beat_length: 500.0,
            slider_velocity: 1.0,
            meter: 4,
            kiai: false,
        };
        // Seed with the state at time 0 so the first advance_to(0) is a no-op.
        while c.index < c.points.len() && c.points[c.index].time <= 0.0 {
            c.apply(c.points[c.index]);
            c.index += 1;
        }
        c
    }

    /// Advance the cursor to cover all timing points ≤ `time_ms`.
    /// Call this once per hit object before reading the cached fields.
    pub fn advance_to(&mut self, time_ms: i64) {
        let t = time_ms as f64;
        while self.index < self.points.len() && self.points[self.index].time <= t {
            self.apply(self.points[self.index]);
            self.index += 1;
        }
    }

    #[inline]
    fn apply(&mut self, p: TimingPoint) {
        if p.uninherited {
            self.beat_length = p.beat_length;
            self.slider_velocity = 1.0;
            self.meter = p.meter;
        } else if p.beat_length < 0.0 {
            self.slider_velocity = -100.0 / p.beat_length;
        }
        self.kiai = p.kiai_mode;
    }
}

// ── dispatch ─────────────────────────────────────────────────────────────────

pub fn resolve_convert_target(beatmap: &Beatmap, name: &str) -> Result<i32> {
    let lowered = name.to_lowercase();
    let key = lowered.trim();
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

pub fn convert_beatmap(
    beatmap: &Beatmap,
    target_mode: i32,
    mods: Option<&ModSettings>,
) -> Result<Beatmap> {
    if beatmap.mode() != 0 {
        return Err(PreviewError::new("source beatmap must be osu!standard (mode=0)"));
    }

    match target_mode {
        3 => mania_convert(beatmap, target_mode, mods),
        1 => taiko_convert(beatmap, target_mode, mods),
        2 => catch_convert(beatmap, target_mode, mods),
        _ => Err(PreviewError::new(format!(
            "conversion to mode {target_mode} is not yet implemented"
        ))),
    }
}

pub(crate) fn std_objects(beatmap: &Beatmap) -> &[StandardHitObject] {
    match &beatmap.hit_objects {
        HitObjects::Standard(v) => v,
        _ => &[],
    }
}

// math.isclose(a, b, abs_tol=1e-7) keeps the default rel_tol=1e-9.
pub(crate) fn almost_equals(a: f64, b: f64) -> bool {
    (a - b).abs() <= f64::max(1e-9 * f64::max(a.abs(), b.abs()), 1e-7)
}
