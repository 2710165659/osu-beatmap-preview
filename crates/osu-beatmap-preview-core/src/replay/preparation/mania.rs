//! Mania 回放准备：转换、列映射和判定窗口均独立于输出分辨率。
use crate::{
    domain::{
        models::{Beatmap, ManiaHitObject},
        rulesets::mania::mania_convert,
    },
    gameplay::events::HitResult,
    render::cpu::modes::mania::{
        apply_hold_off_mod, apply_inverse_mod, apply_mirror_mod, resolve_key_count,
    },
    replay::{
        compatibility::{self, ManiaPolicy, ReplayPolicy},
        mods::ReplayMods,
        ReplayDocument,
    },
    PreviewError, Result,
};

pub struct PreparedMania {
    pub beatmap: Beatmap,
    pub objects: Vec<ManiaHitObject>,
    pub original_objects: Vec<ManiaHitObject>,
    pub keys: i32,
    pub hp: f64,
    pub score_multiplier: f64,
    pub windows: [f64; 6],
    pub last_end: f64,
    pub mods: ReplayMods,
    pub policy: ManiaPolicy,
}

pub fn prepare_replay(beatmap: &Beatmap, document: &ReplayDocument) -> Result<PreparedMania> {
    let ReplayPolicy::Mania(policy) = compatibility::select_replay(document)? else {
        return Err(PreviewError::new("Mania preparation requires ruleset 3"));
    };
    compatibility::validate_replay_beatmap(beatmap, document)?;
    let mods = crate::replay::mods::decode(document)?;
    let converted = beatmap.mode() == 0;
    let beatmap = if converted {
        mania_convert(beatmap, 3, Some(&mods.settings))?
    } else {
        beatmap.clone()
    };
    let keys = resolve_key_count(&beatmap)?;
    if !(1..=18).contains(&keys)
        || (!converted
            && (mods.settings.mania_keys.is_some_and(|k| k != keys) || mods.settings.dual_stage))
    {
        return Err(PreviewError::new(
            "UnsupportedReplayVariant: Mania column count",
        ));
    }
    let original_objects = beatmap
        .hit_objects
        .as_mania()
        .ok_or_else(|| PreviewError::parse("InvalidReplay: missing Mania objects"))?
        .to_vec();
    let mut objects = original_objects.clone();
    if mods.settings.inverse {
        objects = apply_inverse_mod(&objects, &beatmap.timing_points);
    }
    if mods.settings.hold_off {
        objects = apply_hold_off_mod(&objects);
    }
    if mods.settings.mirror {
        apply_mirror_mod(&mut objects, keys);
    }
    objects.sort_by_key(|o| (o.start_time, o.lane));
    if objects.is_empty()
        || objects.len() > 200_000
        || objects
            .iter()
            .any(|o| o.end_time < o.start_time || !(0..keys).contains(&o.lane))
    {
        return Err(PreviewError::parse("InvalidReplay: invalid Mania objects"));
    }
    if document
        .frames
        .iter()
        .any(|f| (f.cursor[0] as u32) >> keys != 0)
    {
        return Err(PreviewError::parse(
            "InvalidReplay: Mania input exceeds column count",
        ));
    }
    let od = beatmap.difficulty.get_f64_or("OverallDifficulty", 5.0);
    let hp = beatmap.difficulty.get_f64_or("HPDrainRate", 5.0);
    if !od.is_finite()
        || !hp.is_finite()
        || beatmap.timing_points.iter().any(|p| {
            !p.time.is_finite()
                || !p.beat_length.is_finite()
                || (p.uninherited && p.beat_length <= 0.0)
        })
    {
        return Err(PreviewError::parse(
            "InvalidReplay: invalid Mania difficulty or timing",
        ));
    }
    let difficulty_multiplier = if mods.settings.easy {
        0.5
    } else if mods.settings.hard_rock {
        1.4
    } else {
        1.0
    };
    let windows = hit_windows(
        od,
        mods.settings.speed_multiplier / difficulty_multiplier,
        policy,
        converted,
    );
    let last_end = objects
        .iter()
        .map(|o| o.end_time as f64)
        .fold(f64::NEG_INFINITY, f64::max);
    let score_multiplier = score_multiplier(&mods, document);
    Ok(PreparedMania {
        beatmap,
        objects,
        original_objects,
        keys,
        hp: if mods.settings.easy {
            hp * 0.5
        } else if mods.settings.hard_rock {
            (hp * 1.4).min(10.0)
        } else {
            hp
        },
        score_multiplier,
        windows,
        last_end,
        mods,
        policy,
    })
}

/// 官方 ManiaHitWindows：Classic 非 V2 固定 16ms 的最高判定，其余窗口随 OD 变化。
pub fn hit_windows(od: f64, multiplier: f64, policy: ManiaPolicy, converted: bool) -> [f64; 6] {
    let raw = if policy.classic && !policy.score_v2 {
        if converted {
            let hard = od.round_ties_even() > 4.0;
            [
                16.0,
                if hard { 34.0 } else { 47.0 },
                if hard { 67.0 } else { 77.0 },
                97.0,
                121.0,
                158.0,
            ]
        } else {
            let inv = (10.0 - od).clamp(0.0, 10.0) * 3.0;
            [
                16.0,
                34.0 + inv,
                67.0 + inv,
                97.0 + inv,
                121.0 + inv,
                158.0 + inv,
            ]
        }
    } else {
        let perfect = if od > 5.0 {
            19.4 - (od - 5.0) * 1.1
        } else {
            22.4 - od * 0.6
        };
        [
            perfect,
            64.0 - 3.0 * od,
            97.0 - 3.0 * od,
            127.0 - 3.0 * od,
            151.0 - 3.0 * od,
            188.0 - 3.0 * od,
        ]
    };
    raw.map(|w| (w * multiplier).floor() + 0.5)
}

pub fn result_for(offset: f64, windows: &[f64; 6]) -> Option<HitResult> {
    [
        HitResult::Perfect,
        HitResult::Great,
        HitResult::Good,
        HitResult::Ok,
        HitResult::Meh,
        HitResult::Miss,
    ]
    .into_iter()
    .zip(windows)
    .find(|(_, w)| offset.abs() <= **w)
    .map(|(result, _)| result)
}

// 回放采用当前标准化计分倍率，历史总分仍保留在诊断中。
fn score_multiplier(mods: &ReplayMods, document: &ReplayDocument) -> f64 {
    let m = &mods.settings;
    let declared = |code: &str, bit: u32| {
        document
            .metadata
            .as_ref()
            .map_or(document.header.mods_bits & (1 << bit) != 0, |metadata| {
                metadata
                    .mods
                    .iter()
                    .any(|modifier| modifier.acronym == code)
            })
    };
    let mut value = 1.0;
    if m.easy {
        value *= 0.5;
    }
    if declared("NF", 0) {
        value *= 0.5;
    }
    if m.half_time || m.daycore {
        let rate = (m.speed_multiplier * 10.0).trunc() / 10.0;
        value *= if m.speed_multiplier >= 1.0 {
            1.0 + (rate - 1.0) / 5.0
        } else {
            0.6 + rate - 1.0
        };
    }
    if m.classic && document.header.version < 30_000_017 {
        value *= 0.96;
    }
    if m.cs_override {
        value *= 0.9;
    }
    if m.hold_off {
        value *= 0.9;
    }
    if m.mania_keys.is_some() {
        value *= 0.9;
    }
    value
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::replay::{ReplayFrame, ReplayHeader};
    use std::sync::Arc;

    pub(crate) fn fixture(
        objects: &str,
        frames: &[(i64, u32)],
    ) -> (PreparedMania, Arc<ReplayDocument>) {
        let map = crate::parse_beatmap_bytes(format!("osu file format v14\n[General]\nMode:3\n[Difficulty]\nCircleSize:4\nOverallDifficulty:5\nHPDrainRate:5\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n{objects}\n").as_bytes()).unwrap();
        let document = Arc::new(ReplayDocument {
            header: ReplayHeader {
                ruleset_id: 3,
                version: 30_000_019,
                beatmap_md5: String::new(),
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
            metadata: None,
            seed: None,
            diagnostics: Vec::new(),
            frames: frames
                .iter()
                .enumerate()
                .map(|(sequence, &(time_ms, mask))| ReplayFrame {
                    time_ms,
                    cursor: [mask as f32, 0.0],
                    keys: 0,
                    delta_ms: 0,
                    sequence,
                })
                .collect(),
        });
        (prepare_replay(&map, &document).unwrap(), document)
    }

    #[test]
    fn windows_preserve_classic_v2_rate_and_inclusive_boundaries() {
        for (classic, score_v2, multiplier, converted, expected) in [
            (
                true,
                false,
                1.0,
                false,
                [16.5, 49.5, 82.5, 112.5, 136.5, 173.5],
            ),
            (
                false,
                false,
                1.0,
                false,
                [19.5, 49.5, 82.5, 112.5, 136.5, 173.5],
            ),
            (
                true,
                true,
                1.0,
                false,
                [19.5, 49.5, 82.5, 112.5, 136.5, 173.5],
            ),
            (
                true,
                false,
                1.5,
                true,
                [24.5, 51.5, 100.5, 145.5, 181.5, 237.5],
            ),
        ] {
            let windows = hit_windows(
                5.0,
                multiplier,
                ManiaPolicy { classic, score_v2 },
                converted,
            );
            assert_eq!(windows, expected);
            for (i, result) in [
                HitResult::Perfect,
                HitResult::Great,
                HitResult::Good,
                HitResult::Ok,
                HitResult::Meh,
                HitResult::Miss,
            ]
            .into_iter()
            .enumerate()
            {
                assert_eq!(result_for(windows[i], &windows), Some(result));
                assert_eq!(result_for(-windows[i], &windows), Some(result));
            }
            assert_eq!(result_for(windows[5].next_up(), &windows), None);
        }
    }

    #[test]
    fn classic_replays_use_recorded_policy_for_legacy_lazer_and_score_v2() {
        use crate::gameplay::engine::compile_mania;
        use crate::render::{geometry::OutputFormat, shared::mania::replay::ReplayRenderPlan};
        for (version, mods, expected, classic) in [
            (20250101, None, HitResult::Great, true),
            (
                30_000_019,
                Some(serde_json::json!([])),
                HitResult::Perfect,
                false,
            ),
            (
                30_000_019,
                Some(serde_json::json!([{"acronym":"CL"}])),
                HitResult::Great,
                true,
            ),
            (
                30_000_019,
                Some(serde_json::json!([{"acronym":"CL"},{"acronym":"SV2"}])),
                HitResult::Perfect,
                true,
            ),
        ] {
            let (source, document) = fixture(
                "64,192,1000,1,0",
                &[(0, 0), (1018, 1), (1030, 0), (1400, 0)],
            );
            let mut document = (*document).clone();
            document.header.version = version;
            document.metadata =
                mods.map(|mods| serde_json::from_value(serde_json::json!({"mods":mods})).unwrap());
            let document = Arc::new(document);
            let prepared = Arc::new(prepare_replay(&source.beatmap, &document).unwrap());
            assert_eq!(prepared.mods.settings.classic, classic);
            assert_eq!(prepared.policy.classic, classic);
            let gameplay = Arc::new(compile_mania(&prepared, document).unwrap());
            assert_eq!(gameplay.objects[0].head.unwrap().1, expected);
            for format in [OutputFormat::Gif, OutputFormat::Mp4] {
                let plan = ReplayRenderPlan::with_format(
                    Arc::clone(&prepared),
                    Arc::clone(&gameplay),
                    None,
                    None,
                    format,
                )
                .unwrap();
                let mut image = crate::render::canvas::Img::new(
                    plan.layout.image_width as u32,
                    plan.layout.image_height as u32,
                    [0; 4],
                );
                plan.draw_frame(&mut image, 1060);
                plan.draw_overlay(&mut image, 1060);
                assert!(image.data.chunks_exact(4).any(|p| p[3] > 0));
            }
            let session = crate::api::session::RealtimeSession::from_replay_bundle(
                crate::ResourceBundle::new(prepared.beatmap.clone()),
                crate::RealtimeOptions::default(),
                Arc::clone(&gameplay.document),
                None,
            )
            .unwrap();
            assert_eq!(
                session.replay_timeline().unwrap().objects[0]
                    .head
                    .unwrap()
                    .1,
                expected
            );
        }
    }

    #[test]
    fn column_masks_and_mods_are_validated_before_compilation() {
        let (prepared, document) = fixture("64,192,1000,1,0", &[(0, 0), (1000, 1), (1300, 0)]);
        for invalid in [-1.0, 0.5, 16.0, f32::NAN] {
            let mut replay = (*document).clone();
            replay.frames[1].cursor[0] = invalid;
            assert!(prepare_replay(&prepared.beatmap, &replay).is_err());
        }
        for (mods, valid) in [
            (serde_json::json!([{"acronym":"4K"}]), true),
            (
                serde_json::json!([{"acronym":"4K"},{"acronym":"5K"}]),
                false,
            ),
            (serde_json::json!([{"acronym":"RD"}]), false),
        ] {
            let mut replay = (*document).clone();
            replay.metadata =
                Some(serde_json::from_value(serde_json::json!({"mods":mods})).unwrap());
            assert_eq!(prepare_replay(&prepared.beatmap, &replay).is_ok(), valid);
        }
    }
}
