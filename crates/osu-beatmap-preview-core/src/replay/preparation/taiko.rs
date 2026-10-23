//! Taiko 回放准备：先转换谱面，再应用 Mod，生成与输出尺寸无关的节点。
use crate::domain::models::{Beatmap, TaikoHitObject};
use crate::domain::mods::{parse_mods, validate_mods, ModSettings};
use crate::domain::rulesets::taiko::gameplay::{self, DrumRollTick};
use crate::replay::{compatibility, model::ReplayMod, mods::ReplayMods, ReplayDocument};
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
    let mods = decode_mods(document)?;
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
    // 当前 TaikoScoreProcessor 的无成绩上下文计算：AT、SW、CL 不改变倍率。
    value
}

pub fn decode_mods(document: &ReplayDocument) -> Result<ReplayMods> {
    let mut result = ReplayMods::default();
    let mut mods = if let Some(metadata) = &document.metadata {
        metadata.mods.clone()
    } else {
        let bits = document.header.mods_bits;
        let supported = (1 << 1)
            | (1 << 3)
            | (1 << 4)
            | (1 << 6)
            | (1 << 8)
            | (1 << 9)
            | (1 << 10)
            | (1 << 11)
            | (1 << 29);
        if bits & !supported != 0 {
            return Err(unsupported(format!("legacy Taiko Mods 0x{bits:08x}")));
        }
        [
            (1, "EZ"),
            (3, "HD"),
            (4, "HR"),
            (6, "DT"),
            (8, "HT"),
            (9, "NC"),
            (10, "FL"),
            (11, "AT"),
        ]
        .into_iter()
        .filter(|&(bit, _)| bits & (1 << bit) != 0 && !(bit == 6 && bits & (1 << 9) != 0))
        .map(|(_, acronym)| ReplayMod {
            acronym: acronym.into(),
            settings: Default::default(),
            extra: Default::default(),
        })
        .collect()
    };
    if document.header.version < 30_000_000 && !mods.iter().any(|m| m.acronym == "CL") {
        mods.push(ReplayMod {
            acronym: "CL".into(),
            settings: Default::default(),
            extra: Default::default(),
        });
    }
    let mut tokens = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for modifier in mods {
        let code = modifier.acronym.as_str();
        if !seen.insert(code.to_owned()) {
            return Err(unsupported(format!("duplicate Mod {code}")));
        }
        if !matches!(
            code,
            "EZ" | "HR" | "HD" | "FL" | "CS" | "SW" | "DT" | "HT" | "NC" | "DC" | "AT" | "CL"
        ) {
            return Err(unsupported(format!("Mod {code}")));
        }
        let mut token = code.to_owned();
        if !modifier.extra.is_empty() {
            return Err(unsupported(format!("{code} extra settings")));
        }
        for (name, value) in &modifier.settings {
            match (code, name.as_str()) {
                ("DT" | "HT" | "NC" | "DC", "speed_change") => {
                    let rate = value
                        .as_f64()
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| unsupported(format!("{code} setting {name}")))?;
                    token = format!("{code}{rate}");
                }
                ("DT" | "HT", "adjust_pitch") => {
                    result.adjust_pitch = value
                        .as_bool()
                        .ok_or_else(|| unsupported(format!("{code} setting {name}")))?
                }
                ("FL", "size_multiplier") if value.as_f64() == Some(1.0) => (),
                ("FL", "combo_based_size") if value.as_bool() == Some(true) => (),
                _ => return Err(unsupported(format!("{code} setting {name}"))),
            }
        }
        tokens.push(token);
    }
    result.settings = parse_mods(&tokens).map_err(|e| unsupported(e.to_string()))?;
    let errors = validate_mods(&result.settings, Some(1), None);
    if !errors.is_empty() {
        return Err(unsupported(errors.join("; ")));
    }
    Ok(result)
}

fn unsupported(reason: impl std::fmt::Display) -> PreviewError {
    PreviewError::new(format!("UnsupportedReplayVariant: {reason}"))
}
