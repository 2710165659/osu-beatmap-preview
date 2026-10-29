//! Taiko 回放准备：先转换谱面，再应用 Mod，生成与输出尺寸无关的节点。
use crate::domain::models::{Beatmap, TaikoHitObject};
use crate::domain::mods::ModSettings;
use crate::domain::rulesets::taiko::gameplay::{self, DrumRollTick};
use crate::replay::{compatibility, mods::ReplayMods, ReplayDocument};
use crate::support::error::{PreviewError, Result};

#[derive(Debug, Clone)]
pub enum TaikoObjectKind {
    Hit,
    DrumRoll {
        ticks: Vec<DrumRollTick>,
        window: f64,
    },
    Swell {
        required: u32,
    },
}

#[derive(Debug, Clone)]
pub struct PreparedObject {
    pub source: TaikoHitObject,
    pub kind: TaikoObjectKind,
}
impl PreparedObject {
    pub fn rim(&self) -> bool {
        self.source.hitsound & 10 != 0
    }
    pub fn strong(&self) -> bool {
        self.source.hitsound & 4 != 0
    }
    pub fn node_count(&self) -> usize {
        match &self.kind {
            TaikoObjectKind::Hit => usize::from(self.strong()),
            TaikoObjectKind::DrumRoll { ticks, .. } => ticks.len(),
            TaikoObjectKind::Swell { required } => *required as usize,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PreparedTaiko {
    pub beatmap: Beatmap,
    pub objects: Vec<PreparedObject>,
    pub od: f64,
    pub hp: f64,
    pub last_end: f64,
    pub mods: ReplayMods,
    pub score_multiplier: f64,
}

pub fn prepare_replay(beatmap: &Beatmap, document: &ReplayDocument) -> Result<PreparedTaiko> {
    if !matches!(
        compatibility::select_replay(document)?,
        compatibility::ReplayPolicy::Taiko(_)
    ) {
        return Err(unsupported("Taiko preparation requires ruleset 1"));
    }
    compatibility::validate_replay_beatmap(beatmap, document)?;
    let mods = crate::replay::mods::decode(document)?;
    let beatmap = if beatmap.mode() == 0 {
        crate::domain::rulesets::taiko::taiko_convert(beatmap, 1, None)?
    } else {
        beatmap.clone()
    };
    for key in [
        "OverallDifficulty",
        "HPDrainRate",
        "SliderMultiplier",
        "SliderTickRate",
    ] {
        if !beatmap.difficulty.get_f64_or(key, 5.0).is_finite() {
            return Err(PreviewError::parse(format!(
                "InvalidReplay: non-finite {key}"
            )));
        }
    }
    if beatmap.timing_points.iter().any(|p| {
        !p.time.is_finite()
            || (p.uninherited && (!p.beat_length.is_finite() || p.beat_length <= 0.0))
    }) {
        return Err(PreviewError::parse("InvalidReplay: invalid Taiko timing"));
    }
    let effective = |key| {
        let mut value = beatmap.difficulty.get_f64_or(key, 5.0) as f32;
        if mods.settings.easy {
            value *= 0.5;
        }
        if mods.settings.hard_rock {
            value = (value * 1.4).min(10.0);
        }
        f64::from(value)
    };
    let od = effective("OverallDifficulty");
    let hp = effective("HPDrainRate");
    let sources = beatmap
        .hit_objects
        .as_taiko()
        .ok_or_else(|| unsupported("invalid Taiko objects"))?;
    if sources.is_empty() || sources.len() > 200_000 {
        return Err(PreviewError::parse(
            "InvalidReplay: unsupported Taiko object count",
        ));
    }
    let rate = beatmap.difficulty.get_f64_or("SliderTickRate", 1.0);
    let mut objects = Vec::with_capacity(sources.len());
    let mut node_count = 0;
    for source in sources {
        if source.end_time < source.start_time {
            return Err(PreviewError::parse("InvalidReplay: backwards Taiko object"));
        }
        let mut source = source.clone();
        let kind = if source.hit_type & 2 != 0 {
            TaikoObjectKind::DrumRoll {
                ticks: gameplay::generate_drum_roll_ticks(&source, &beatmap.timing_points, rate),
                window: gameplay::drum_roll_tick_spacing(&source, &beatmap.timing_points, rate)
                    / 2.0,
            }
        } else if source.hit_type & 8 != 0 {
            TaikoObjectKind::Swell {
                required: gameplay::required_swell_hits(source.end_time - source.start_time, od),
            }
        } else {
            if mods.settings.swap {
                source.hitsound = if source.hitsound & 10 != 0 {
                    source.hitsound & !10
                } else {
                    source.hitsound | 8
                };
            }
            TaikoObjectKind::Hit
        };
        let object = PreparedObject { source, kind };
        node_count += object.node_count();
        if node_count > 1_000_000 {
            return Err(PreviewError::parse("InvalidReplay: too many Taiko nodes"));
        }
        objects.push(object);
    }
    let last_end = objects
        .iter()
        .map(|o| o.source.end_time as f64)
        .fold(f64::NEG_INFINITY, f64::max);
    let score_multiplier = score_multiplier(&mods.settings);
    Ok(PreparedTaiko {
        beatmap,
        objects,
        od,
        hp,
        last_end,
        mods,
        score_multiplier,
    })
}

fn score_multiplier(s: &ModSettings) -> f64 {
    let mut value = 1.0;
    if s.easy {
        value *= 0.5;
    }
    if s.hard_rock {
        value *= 1.06;
    }
    if s.hidden {
        value *= 1.06;
    }
    if s.flashlight {
        value *= 1.12;
    }
    if s.has_rate_mod() {
        let rate = (s.speed_multiplier * 10.0) as i32 as f64 / 10.0 - 1.0;
        value *= if s.speed_multiplier >= 1.0 {
            1.0 + rate / 5.0
        } else {
            0.6 + rate
        };
    }
    if s.cs_override {
        value *= 0.9;
    }
    // 对照当前 TaikoScoreProcessor：CL 不改变判定或重播倍率。
    // 0.96 仅供旧成绩迁移使用，不能按 OSR 录制版本套入当前重播；SW 同样无倍率。
    value
}

fn unsupported(reason: impl std::fmt::Display) -> PreviewError {
    PreviewError::new(format!("UnsupportedReplayVariant: {reason}"))
}
