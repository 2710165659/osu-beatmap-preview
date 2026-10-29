//! Catch 回放准备：转换谱面并复用水果展开规则，判定不依赖输出尺寸。
use crate::render::cpu::modes::catch::{
    constants::*,
    objects::{build_catch_render_objects, effective_difficulty, RenderObject},
};
use crate::{
    model::Beatmap,
    replay::{compatibility, mods::ReplayMods, ReplayDocument},
    PreviewError, Result,
};

pub struct PreparedCatch {
    pub beatmap: Beatmap,
    pub objects: Vec<RenderObject>,
    pub mods: ReplayMods,
    pub cs: f64,
    pub ar: f64,
    pub hp: f64,
    pub first: i64,
    pub last_end: f64,
    pub half_catch_width: f64,
    pub score_multiplier: f64,
}

pub fn prepare_replay(beatmap: &Beatmap, document: &ReplayDocument) -> Result<PreparedCatch> {
    if !matches!(
        compatibility::select_replay(document)?,
        compatibility::ReplayPolicy::Catch(_)
    ) {
        return Err(PreviewError::new("Catch preparation requires ruleset 2"));
    }
    compatibility::validate_replay_beatmap(beatmap, document)?;
    let mods = crate::replay::mods::decode(document)?;
    let beatmap = if beatmap.mode() == 0 {
        crate::domain::rulesets::catch::catch_convert(beatmap, 2, Some(&mods.settings))?
    } else {
        beatmap.clone()
    };
    for key in [
        "CircleSize",
        "ApproachRate",
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
        return Err(PreviewError::parse("InvalidReplay: invalid Catch timing"));
    }
    let sources = beatmap
        .hit_objects
        .as_catch()
        .ok_or_else(|| PreviewError::parse("InvalidReplay: invalid Catch objects"))?;
    if sources.is_empty()
        || sources.len() > 200_000
        || sources.iter().any(|o| o.end_time < o.start_time)
    {
        return Err(PreviewError::parse(
            "InvalidReplay: invalid Catch object count or duration",
        ));
    }
    let first = sources[0].start_time;
    let last_end = sources
        .iter()
        .map(|o| o.end_time as f64)
        .fold(f64::NEG_INFINITY, f64::max);
    let difficulty = effective_difficulty(&beatmap, Some(&mods.settings));
    let mut objects =
        build_catch_render_objects(&beatmap, sources, Some(&mods.settings), &difficulty, false)?;
    if objects.is_empty()
        || objects.len() > 1_000_000
        || objects
            .iter()
            .any(|o| !o.x.is_finite() || !o.event_time_or_start().is_finite())
    {
        return Err(PreviewError::parse(
            "InvalidReplay: invalid Catch expanded objects",
        ));
    }
    objects.sort_by(|a, b| a.event_time_or_start().total_cmp(&b.event_time_or_start()));
    // 滑条展开保留小数时刻，不能以解析后的整数终点提前结束判定。
    let last_end = last_end.max(objects.last().unwrap().event_time_or_start());
    let mut hp = beatmap.difficulty.get_f64_or("HPDrainRate", 5.0);
    if mods.settings.easy {
        hp *= 0.5;
    }
    if mods.settings.hard_rock {
        hp = (hp * 1.4).min(10.0);
    }
    let mut multiplier = 1.0;
    if mods.settings.easy {
        multiplier *= 0.5;
    }
    if mods.settings.hard_rock {
        multiplier *= 1.12;
    }
    if mods.settings.hidden {
        multiplier *= 1.06;
    }
    if mods.settings.flashlight {
        multiplier *= 1.12;
    }
    if mods.settings.has_rate_mod() {
        let rate = (mods.settings.speed_multiplier * 10.0) as i32 as f64 / 10.0 - 1.0;
        multiplier *= if mods.settings.speed_multiplier >= 1.0 {
            1.0 + rate / 5.0
        } else {
            0.6 + rate
        };
    }
    // 当前 Catch 重播的 CL 不改变判定或倍率；旧成绩迁移的 0.96 不用于重新计分。
    Ok(PreparedCatch {
        cs: difficulty.cs,
        ar: difficulty.ar,
        hp,
        first,
        last_end,
        half_catch_width: CATCHER_BASE_SIZE
            * crate::render::cpu::modes::catch::objects::circle_scale(difficulty.cs).abs()
            * ALLOWED_CATCH_RANGE,
        score_multiplier: multiplier,
        beatmap,
        objects,
        mods,
    })
}
