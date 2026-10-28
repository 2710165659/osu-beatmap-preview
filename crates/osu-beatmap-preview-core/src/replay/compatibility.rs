//! 按回放内容选择 Standard/Taiko 规则，不用客户端名称或分数版本建立白名单。

use super::ReplayDocument;
use crate::model::Beatmap;
use crate::support::error::{PreviewError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayPolicy {
    Standard(StandardPolicy),
    Taiko(TaikoPolicy),
}

impl Default for ReplayPolicy {
    fn default() -> Self {
        Self::Standard(StandardPolicy::default())
    }
}

impl From<StandardPolicy> for ReplayPolicy {
    fn from(policy: StandardPolicy) -> Self {
        Self::Standard(policy)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaikoPolicy {
    pub classic: bool,
}

pub fn select_replay(document: &ReplayDocument) -> Result<ReplayPolicy> {
    match document.header.ruleset_id {
        0 => select(document).map(ReplayPolicy::Standard),
        1 => {
            let mods = super::mods::decode(document)?;
            if document.frames.is_empty()
                || document
                    .frames
                    .windows(2)
                    .any(|w| w[1].time_ms < w[0].time_ms)
            {
                return Err(PreviewError::parse(
                    "InvalidReplay: empty or backwards Taiko frames",
                ));
            }
            if document.frames.iter().any(|f| f.keys & !15 != 0) {
                return Err(PreviewError::new(
                    "UnsupportedReplayVariant: unknown Taiko input buttons",
                ));
            }
            Ok(ReplayPolicy::Taiko(TaikoPolicy {
                classic: mods.settings.classic,
            }))
        }
        mode => Err(PreviewError::new(format!(
            "UnsupportedReplayVariant: mode {mode}; only Standard and Taiko replays are supported"
        ))),
    }
}

pub fn validate_replay_beatmap(beatmap: &Beatmap, document: &ReplayDocument) -> Result<()> {
    match document.header.ruleset_id {
        0 => validate_beatmap(beatmap),
        1 if matches!(beatmap.mode(), 0 | 1) => Ok(()),
        _ => Err(PreviewError::new(
            "UnsupportedReplayVariant: beatmap mode does not match replay",
        )),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StandardPolicy {
    pub classic: bool,
    pub classic_sliders: bool,
    pub classic_note_lock: bool,
    pub always_tail_sample: bool,
    pub fade_hit_circle_early: bool,
}

impl StandardPolicy {
    pub(crate) fn score_multiplier(self) -> f64 {
        // 对照当前官方 OsuScoreMultiplierCalculatorV2；历史回放重播也使用当前倍率。
        if !self.classic {
            1.0
        } else if self.classic_note_lock {
            0.985
        } else {
            0.96
        }
    }
}

pub fn select(document: &ReplayDocument) -> Result<StandardPolicy> {
    let header = &document.header;
    if header.ruleset_id != 0 {
        return Err(unsupported(format!(
            "mode {} (only native osu!standard)",
            header.ruleset_id
        )));
    }
    super::mods::decode(document)?;
    // 官方 LegacyScoreDecoder 给 stable 自动附加 CL；历史和未来 lazer 计分版本
    // 都按当前规则重播，原始版本和统计保留作诊断，不据此拒绝播放。
    let legacy = header.version < 30_000_000;
    let mut policy = StandardPolicy {
        classic: legacy,
        classic_sliders: legacy,
        classic_note_lock: legacy,
        always_tail_sample: legacy,
        fade_hit_circle_early: legacy,
    };
    if let Some(metadata) = &document.metadata {
        for modifier in &metadata.mods {
            if modifier.acronym != "CL" {
                continue;
            }
            policy.classic_sliders = true;
            policy.classic = true;
            policy.classic_note_lock = true;
            policy.always_tail_sample = true;
            policy.fade_hit_circle_early = true;
            for (name, value) in &modifier.settings {
                let value = value
                    .as_bool()
                    .ok_or_else(|| unsupported(format!("CL setting {name}")))?;
                match name.as_str() {
                    "no_slider_head_accuracy" => policy.classic_sliders = value,
                    "classic_note_lock" => policy.classic_note_lock = value,
                    "always_play_tail_sample" => policy.always_tail_sample = value,
                    "fade_hit_circle_early" => policy.fade_hit_circle_early = value,
                    // 预览只在输入提前结束时失败，不另外模拟 HP 失败。
                    "classic_health" => (),
                    _ => return Err(unsupported(format!("CL setting {name}"))),
                }
            }
        }
    }
    // Pauses 是录制时的墙钟统计，帧已经使用谱面时间；不能再次累加暂停时长。
    if document.frames.is_empty() {
        return Err(PreviewError::parse("InvalidReplay: empty input frames"));
    }
    if document
        .frames
        .windows(2)
        .any(|w| w[1].time_ms < w[0].time_ms)
    {
        return Err(PreviewError::parse("InvalidReplay: backwards frame time"));
    }
    if document.frames.iter().any(|frame| frame.keys & !31 != 0) {
        return Err(unsupported("unknown Standard input buttons"));
    }
    Ok(policy)
}

pub fn validate_beatmap(beatmap: &Beatmap) -> Result<()> {
    if beatmap.mode() != 0 {
        return Err(unsupported("converted/non-Standard beatmap"));
    }
    for key in [
        "CircleSize",
        "ApproachRate",
        "OverallDifficulty",
        "HPDrainRate",
    ] {
        if !beatmap.difficulty.get_f64_or(key, 5.0).is_finite() {
            return Err(PreviewError::parse(format!(
                "InvalidReplay: non-finite difficulty {key}"
            )));
        }
    }
    Ok(())
}
fn unsupported(reason: impl std::fmt::Display) -> PreviewError {
    PreviewError::new(format!("UnsupportedReplayVariant: {reason}; only native osu!standard with supported replay Mods is available"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::model::ReplayMod;
    use crate::replay::{ReplayFrame, ReplayHeader, ReplayMetadata};

    #[test]
    fn rejects_unsupported_mods_and_invalid_input_instead_of_client_versions() {
        let original = document();
        assert!(select(&original).is_ok());
        for acronym in ["NF", "UNKNOWN"] {
            let mut replay = original.clone();
            replay.metadata.as_mut().unwrap().mods.push(ReplayMod {
                acronym: acronym.into(),
                settings: Default::default(),
                extra: Default::default(),
            });
            assert!(select(&replay).unwrap_err().to_string().contains(acronym));
        }
        for version in [20070916, 20200101, 30_000_000, 30_000_019, 40_000_000] {
            let mut replay = original.clone();
            replay.header.version = version;
            assert!(select(&replay).is_ok());
        }
        for bits in [1, 1 << 30, 1 << 31] {
            let mut replay = original.clone();
            replay.metadata = None;
            replay.header.mods_bits = bits;
            assert!(select(&replay).is_err());
        }
        let mut replay = original.clone();
        replay.metadata.as_mut().unwrap().client_version = "2099.1.0-lazer-windows".into();
        assert!(select(&replay).is_ok());
        let mut replay = original;
        replay.frames.clear();
        assert!(select(&replay)
            .unwrap_err()
            .to_string()
            .contains("InvalidReplay"));
    }

    #[test]
    fn supports_tachyon_pauses_smoke_and_implicit_or_explicit_classic() {
        let mut replay = document();
        let metadata = replay.metadata.as_mut().unwrap();
        metadata.client_version = "2026.821.0-tachyon-windows".into();
        metadata
            .pauses
            .push(serde_json::json!({"time": 100, "duration": 4000}));
        replay.frames[0].keys = 16;
        assert_eq!(select(&replay).unwrap(), StandardPolicy::default());
        replay.header.version = 20200101;
        replay.metadata = None;
        assert!(select(&replay).unwrap().classic_sliders);
        replay.header.version = 30_000_001;
        assert!(!select(&replay).unwrap().classic_sliders);
        replay.metadata = document().metadata;
        replay.metadata.as_mut().unwrap().mods.push(ReplayMod {
            acronym: "CL".into(),
            settings: std::collections::BTreeMap::from([(
                "classic_note_lock".into(),
                false.into(),
            )]),
            extra: Default::default(),
        });
        let policy = select(&replay).unwrap();
        assert!(policy.classic_sliders);
        assert!(!policy.classic_note_lock);
    }

    fn document() -> ReplayDocument {
        ReplayDocument {
            header: ReplayHeader {
                ruleset_id: 0,
                version: 30_000_016,
                beatmap_md5: "0".repeat(32),
                player: String::new(),
                replay_hash: String::new(),
                counts: [0; 6],
                total_score: 0,
                max_combo: 0,
                perfect: false,
                mods_bits: 0,
                life_graph: String::new(),
                timestamp_ticks: 0,
                online_id: 0,
                target_practice_accuracy: None,
            },
            metadata: Some(ReplayMetadata {
                client_version: "2026.119.0-lazer-windows".into(),
                rank: "X".into(),
                mods: vec![],
                statistics: Default::default(),
                maximum_statistics: Default::default(),
                pauses: vec![],
                extra: Default::default(),
            }),
            frames: vec![ReplayFrame {
                delta_ms: 0,
                time_ms: 0,
                cursor: [0.0; 2],
                keys: 0,
                sequence: 0,
            }],
            seed: None,
            diagnostics: vec![],
        }
    }
}
