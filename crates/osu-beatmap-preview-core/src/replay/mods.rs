//! 回放 Mod 解码与当前 osu! Standard 计分倍率；lazer 扩展覆盖旧位掩码。
use super::{model::ReplayMod, ReplayDocument};
use crate::domain::mods::{parse_mods, validate_mods_for_level, ModSettings, ModSupportLevel};
use crate::model::Beatmap;
use crate::support::error::{PreviewError, Result};

#[derive(Debug, Clone)]
pub struct ReplayMods {
    pub settings: ModSettings,
    pub hidden_only_approach: bool,
    pub flashlight_size: f64,
    pub flashlight_combo: bool,
    pub flashlight_delay: f64,
    pub easy_retries: u64,
    pub adjust_pitch: bool,
}

impl Default for ReplayMods {
    fn default() -> Self {
        Self {
            settings: ModSettings::new(),
            hidden_only_approach: false,
            flashlight_size: 1.0,
            flashlight_combo: true,
            flashlight_delay: 120.0,
            easy_retries: 2,
            adjust_pitch: false,
        }
    }
}

pub fn decode(document: &ReplayDocument) -> Result<ReplayMods> {
    if document.header.ruleset_id == 1 {
        return super::preparation::taiko::decode_mods(document);
    }
    let mut result = ReplayMods::default();
    let mut tokens = Vec::new();
    // 与 LegacyScoreDecoder 一致：存在扩展时只使用扩展里的完整 Mod 声明。
    let mods = if let Some(metadata) = &document.metadata {
        metadata.mods.clone()
    } else {
        let bits = document.header.mods_bits;
        let supported = (1 << 1)
            | (1 << 3)
            | (1 << 4)
            | (1 << 6)
            | (1 << 7)
            | (1 << 8)
            | (1 << 9)
            | (1 << 10)
            | (1 << 12)
            | (1 << 13)
            | (1 << 29);
        if bits & !supported != 0 {
            return Err(unsupported(format!(
                "legacy Mods 0x{:08x}",
                bits & !supported
            )));
        }
        [
            (1, "EZ"),
            (3, "HD"),
            (4, "HR"),
            (6, "DT"),
            (7, "RX"),
            (8, "HT"),
            (9, "NC"),
            (10, "FL"),
            (12, "SO"),
            (13, "AP"),
        ]
        .into_iter()
        // NC 的旧编码同时包含 DT；它们只代表一个速度 Mod。
        .filter(|&(bit, _)| bits & (1 << bit) != 0 && !(bit == 6 && bits & (1 << 9) != 0))
        .map(|(_, acronym)| ReplayMod {
            acronym: acronym.into(),
            settings: Default::default(),
            extra: Default::default(),
        })
        .collect()
    };
    let mut seen = std::collections::HashSet::new();
    for modifier in mods {
        let code = modifier.acronym.as_str();
        if !seen.insert(modifier.acronym.clone()) {
            return Err(unsupported(format!("duplicate Mod {code}")));
        }
        if !matches!(
            code,
            "EZ" | "HR"
                | "HD"
                | "FL"
                | "DA"
                | "TC"
                | "CL"
                | "DT"
                | "HT"
                | "NC"
                | "DC"
                | "RX"
                | "SO"
                | "AP"
                | "AL"
                | "SG"
                | "MR"
        ) {
            return Err(unsupported(format!("Mod {code}")));
        }
        let mut rate = None;
        let mut da = String::from("DA");
        for (name, value) in &modifier.settings {
            // null 对应游戏中未覆盖的可空 DA 字段，不应变为 0。
            if code == "DA"
                && value.is_null()
                && matches!(
                    name.as_str(),
                    "circle_size" | "approach_rate" | "overall_difficulty" | "drain_rate"
                )
            {
                continue;
            }
            let invalid = || unsupported(format!("{code} setting {name}"));
            match (code, name.as_str()) {
                ("CL", _) => (), // Classic 的布尔设置由 compatibility 统一校验。
                ("MR", "reflection") => {
                    // osu! 的枚举通常序列化为名称，历史容器也可能使用数值。
                    result.settings.mirror_reflection = match value.as_str() {
                        Some("Horizontal") => 0,
                        Some("Vertical") => 1,
                        Some("Both") => 2,
                        _ => value.as_u64().filter(|v| *v <= 2).ok_or_else(invalid)? as u8,
                    };
                }
                ("DT" | "HT" | "NC" | "DC", "speed_change") => {
                    rate = Some(
                        value
                            .as_f64()
                            .filter(|v| v.is_finite())
                            .ok_or_else(invalid)?,
                    )
                }
                ("DT" | "HT", "adjust_pitch") => {
                    result.adjust_pitch = value.as_bool().ok_or_else(invalid)?
                }
                ("HD", "only_fade_approach_circles") => {
                    result.hidden_only_approach = value.as_bool().ok_or_else(invalid)?
                }
                ("FL", "size_multiplier") => {
                    result.flashlight_size =
                        f64::from(number(value, 0.5, 2.0).ok_or_else(invalid)? as f32)
                }
                ("FL", "combo_based_size") => {
                    result.flashlight_combo = value.as_bool().ok_or_else(invalid)?
                }
                ("FL", "follow_delay") => {
                    result.flashlight_delay = number(value, 120.0, 1200.0).ok_or_else(invalid)?
                }
                ("EZ", "retries") => {
                    result.easy_retries = value.as_u64().filter(|v| *v <= 10).ok_or_else(invalid)?
                }
                ("DA", "circle_size" | "approach_rate" | "overall_difficulty" | "drain_rate") => {
                    let field = match name.as_str() {
                        "circle_size" => "CS",
                        "approach_rate" => "AR",
                        "overall_difficulty" => "OD",
                        _ => "HP",
                    };
                    let value = value
                        .as_f64()
                        .filter(|v| v.is_finite())
                        .ok_or_else(invalid)?;
                    da.push_str(&format!("{field}{value}"));
                }
                ("DA", "extended_limits") => {
                    value.as_bool().ok_or_else(invalid)?;
                }
                _ => return Err(invalid()),
            }
        }
        if code == "DA" {
            // 空 DA 仍保留声明，但没有数值需要交给普通预览解析器。
            if da != "DA" {
                tokens.push(da);
            }
        } else {
            tokens.push(rate.map_or_else(|| code.to_owned(), |rate| format!("{code}{rate}")));
        }
    }
    let reflection = result.settings.mirror_reflection;
    result.settings = parse_mods(&tokens).map_err(|e| unsupported(e.to_string()))?;
    result.settings.mirror_reflection = reflection;
    let errors = validate_mods_for_level(&result.settings, 0, ModSupportLevel::Realtime);
    if !errors.is_empty() {
        return Err(unsupported(errors.join("; ")));
    }
    Ok(result)
}

fn number(value: &serde_json::Value, min: f64, max: f64) -> Option<f64> {
    value
        .as_f64()
        .filter(|v| v.is_finite() && (min..=max).contains(v))
}

fn unsupported(reason: impl std::fmt::Display) -> PreviewError {
    PreviewError::new(format!("UnsupportedReplayVariant: {reason}"))
}

impl ReplayMods {
    pub fn music_pitch(&self) -> f64 {
        if self.adjust_pitch {
            self.settings.speed_multiplier
        } else {
            self.settings.music_pitch()
        }
    }

    /// 对照 OsuScoreMultiplierCalculatorV2；DA 的差值必须使用未应用 Mod 的原始难度。
    pub fn score_multiplier(&self, beatmap: &Beatmap) -> f64 {
        let m = &self.settings;
        let mut multiplier = 1.0;
        if m.relax {
            multiplier *= 0.1;
        }
        if m.autopilot {
            multiplier *= 0.1;
        }
        if m.spun_out {
            multiplier *= 0.95;
        }
        if m.easy {
            multiplier *= (0.8 - (0.1 * (self.easy_retries as f64 - 2.0)).max(0.0)).max(0.4);
        }
        if m.hard_rock {
            multiplier *= 1.09;
        }
        if m.hidden {
            multiplier *= if self.hidden_only_approach {
                1.02
            } else {
                1.04
            };
        }
        if m.traceable {
            multiplier *= 1.02;
        }
        if m.flashlight {
            let value = (1.2 - 0.2 * (self.flashlight_size - 1.0)).clamp(1.02, 1.2);
            multiplier *= if self.flashlight_combo {
                value
            } else {
                1.0 + (value - 1.0) / 5.0
            };
        }
        if m.double_time || m.nightcore {
            let value = (m.speed_multiplier * 10.0) as i32 as f64 / 10.0;
            multiplier *= (value - 1.0) * 0.46 + 1.0
                - if value != 1.5 && value != 1.0 {
                    0.01
                } else {
                    0.0
                };
        }
        if m.half_time || m.daycore {
            multiplier *= (m.speed_multiplier * 20.0) as i32 as f64 / 20.0 * 1.4 - 0.5;
        }
        let od = beatmap.difficulty.get_f64_or("OverallDifficulty", 5.0);
        let mut da = 1.0;
        for (value, original) in [
            (m.da_cs, beatmap.difficulty.get_f64_or("CircleSize", 5.0)),
            (m.da_ar, beatmap.difficulty.get_f64_or("ApproachRate", od)),
            (m.da_od, od),
            (m.da_hp, beatmap.difficulty.get_f64_or("HPDrainRate", 5.0)),
        ] {
            if let Some(value) = value {
                // 官方先从 float 难度提升到 double，再计算差值，保留相同的取整边界。
                let difference = (f64::from(value as f32) - f64::from(original as f32)).abs();
                da *= (1.0 - difference * 0.5).max(0.1);
            }
        }
        multiplier * da.max(0.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_every_standard_preview_mod_and_validates_settings_and_combinations() {
        for code in crate::domain::mods::supported_mod_tokens(0) {
            assert!(
                decode(&document(json!([{"acronym": code}]))).is_ok(),
                "{code}"
            );
        }
        for mods in [
            json!([{"acronym":"AT"}]),
            json!([{"acronym":"HR"},{"acronym":"EZ"}]),
            json!([{"acronym":"HD"},{"acronym":"TC"}]),
            json!([{"acronym":"DT"},{"acronym":"NC"}]),
            json!([{"acronym":"HR"},{"acronym":"HR"}]),
            json!([{"acronym":"AL"},{"acronym":"SG"}]),
            json!([{"acronym":"DT","settings":{"speed_change":0.5}}]),
            json!([{"acronym":"HR","settings":{"unknown":true}}]),
            json!([{"acronym":"FL","settings":{"size_multiplier":0}}]),
        ] {
            assert!(decode(&document(mods)).is_err());
        }
    }

    #[test]
    fn score_multipliers_match_current_official_v2_defaults_and_custom_settings() {
        let beatmap = crate::parse_beatmap_bytes(b"osu file format v14\n[General]\nMode:0\n[Difficulty]\nCircleSize:5\nOverallDifficulty:5\nHPDrainRate:5\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n100,100,1000,1,0\n").unwrap();
        for (mods, expected) in [
            (json!([{"acronym":"EZ"}]), 0.8),
            (json!([{"acronym":"EZ","settings":{"retries":10}}]), 0.4),
            (json!([{"acronym":"HR"}]), 1.09),
            (json!([{"acronym":"HD"}]), 1.04),
            (
                json!([{"acronym":"HD","settings":{"only_fade_approach_circles":true}}]),
                1.02,
            ),
            (json!([{"acronym":"FL"}]), 1.2),
            (
                json!([{"acronym":"FL","settings":{"size_multiplier":2,"combo_based_size":false}}]),
                1.004,
            ),
            (json!([{"acronym":"TC"}]), 1.02),
            (json!([{"acronym":"RX"}]), 0.1),
            (json!([{"acronym":"AP"}]), 0.1),
            (json!([{"acronym":"SO"}]), 0.95),
            (json!([{"acronym":"AL"}]), 1.0),
            (json!([{"acronym":"SG"}]), 1.0),
            (json!([{"acronym":"MR"}]), 1.0),
            (json!([{"acronym":"DT"}]), 1.23),
            (
                json!([{"acronym":"NC","settings":{"speed_change":1.39}}]),
                1.128,
            ),
            (json!([{"acronym":"HT"}]), 0.55),
            (
                json!([{"acronym":"DC","settings":{"speed_change":0.59}}]),
                0.27,
            ),
            (
                json!([{"acronym":"DA","settings":{"circle_size":6,"approach_rate":4,"overall_difficulty":null}}]),
                0.25,
            ),
            (
                json!([{"acronym":"DA","settings":{"circle_size":11,"drain_rate":11}}]),
                0.1,
            ),
            (
                json!([{"acronym":"HR"},{"acronym":"HD"},{"acronym":"DT"},{"acronym":"CL"}]),
                1.09 * 1.04 * 1.23 * 0.985,
            ),
        ] {
            let replay = document(mods);
            let calculated = decode(&replay).unwrap().score_multiplier(&beatmap)
                * super::super::compatibility::select(&replay)
                    .unwrap()
                    .score_multiplier();
            assert!(
                (calculated - expected).abs() < 1e-10,
                "{calculated} != {expected}"
            );
        }
    }

    #[test]
    fn legacy_nightcore_is_one_rate_mod_and_extension_overrides_legacy_bits() {
        let mut replay = document(json!([]));
        replay.metadata = None;
        replay.header.mods_bits = (1 << 4) | (1 << 6) | (1 << 9) | (1 << 29);
        let mods = decode(&replay).unwrap();
        assert!(mods.settings.hard_rock && mods.settings.nightcore);
        assert!(!mods.settings.double_time);
        assert_eq!(mods.settings.speed_multiplier, 1.5);
        replay.metadata = document(json!([{"acronym":"HT"}])).metadata;
        assert!(decode(&replay).unwrap().settings.half_time);
        replay.metadata = None;
        for unsupported_bit in [1, 1 << 11] {
            replay.header.mods_bits = unsupported_bit;
            assert!(decode(&replay).is_err());
        }
    }

    fn document(mods: serde_json::Value) -> ReplayDocument {
        let mut replay = crate::replay::parse_osr(include_bytes!(
            "../../../osu-beatmap-preview-web/test/fixtures/replay-standard.osr"
        ))
        .unwrap();
        replay.header.version = 30_000_016;
        replay.metadata = Some(serde_json::from_value(json!({"mods":mods})).unwrap());
        replay
    }
}
