//! 在产物命名及缓存检查之前确定选段，所有渲染器消费同一份绝对毫秒起点。

use super::{
    legacy::resolve_time_points,
    output_plan::{OutputFormat, RenderPlan},
};
use osu_beatmap_preview_core::{
    processing::{
        parse::round_half_even,
        timeline::{PreviewTimeSelector, TimeAxis},
    },
    render::cpu::modes::mania::{apply_hold_off_mod, apply_inverse_mod},
    Beatmap, HitObjects, PreviewError, Result,
};

pub(super) fn select(beatmap: &Beatmap, plan: &mut RenderPlan) -> Result<()> {
    let runtime = crate::config::current();
    let (count, duration_ms) = match (plan.format, plan.target_mode) {
        (OutputFormat::Gif, 0) => {
            let config = &runtime.render.standard.gif;
            (
                config
                    .structure
                    .ROW_COUNT
                    .checked_mul(config.structure.IMAGES_PER_ROW),
                config.style.DURATION_MS as f64,
            )
        }
        (OutputFormat::Gif, 1) => {
            let config = &runtime.render.taiko.gif;
            (
                config
                    .structure
                    .ROW_COUNT
                    .checked_mul(config.structure.IMAGES_PER_ROW)
                    .and_then(|value| usize::try_from(value).ok()),
                config.style.DURATION_MS,
            )
        }
        (OutputFormat::Gif, 2) => {
            let config = &runtime.render.catch.gif;
            (
                config
                    .structure
                    .ROW_COUNT
                    .checked_mul(config.structure.IMAGES_PER_ROW)
                    .and_then(|value| usize::try_from(value).ok()),
                config.style.DURATION_MS,
            )
        }
        (OutputFormat::Gif, 3) => {
            let config = &runtime.render.mania.gif;
            (
                config
                    .structure
                    .ROW_COUNT
                    .checked_mul(config.structure.IMAGES_PER_ROW),
                config.style.DURATION_MS as f64,
            )
        }
        (OutputFormat::Png, 0) => {
            let config = &runtime.render.standard.png;
            (
                Some(config.structure.ROW_COUNT),
                (config.structure.IMAGES_PER_ROW.saturating_sub(1) as f64)
                    * config.style.MS_PER_IMAGE as f64,
            )
        }
        _ => return Ok(()),
    };
    let count = count
        .filter(|&value| value > 0)
        .ok_or_else(|| PreviewError::render("preview segment capacity must be positive"))?;
    let duration_ms = if plan.format == OutputFormat::Gif {
        plan.duration_seconds
            .map_or(duration_ms, |seconds| seconds * 1000.0)
            * plan.mods.as_ref().map_or(1.0, |mods| mods.speed_multiplier)
    } else {
        duration_ms
    };
    if !duration_ms.is_finite() || !(0.0..9_223_372_036_854_775_808.0).contains(&duration_ms) {
        return Err(PreviewError::render(
            "preview segment duration is outside the supported range",
        ));
    }
    // 与各模式渲染器使用相同的时间跨度；IN/HO 会改变 Mania 的长按尾部。
    let spans = object_spans(beatmap, plan);
    let origin = super::legacy::object_time_bounds(&beatmap.hit_objects)
        .map(|(first, _)| first)
        .ok_or_else(|| PreviewError::render("beatmap has no hit objects"))?;
    let requested = resolve_time_points(beatmap, TimeAxis::new(origin), &plan.time_points)?;
    let selected = PreviewTimeSelector::new(
        beatmap,
        spans,
        count,
        round_half_even(duration_ms),
        requested,
    )?
    .choose()?;
    let times: Vec<i64> = selected
        .into_iter()
        .map(|segment| segment.start_time)
        .collect();
    crate::logging::event(
        "select-time-points",
        "done",
        None,
        &format!("absolute_start_ms={times:?}"),
    );
    plan.selected_times = Some(times);
    Ok(())
}

fn object_spans(beatmap: &Beatmap, plan: &RenderPlan) -> Vec<(i64, i64)> {
    match &beatmap.hit_objects {
        HitObjects::Standard(objects) => {
            objects.iter().map(|o| (o.start_time, o.end_time)).collect()
        }
        HitObjects::Taiko(objects) => objects.iter().map(|o| (o.start_time, o.end_time)).collect(),
        HitObjects::Catch(objects) => objects.iter().map(|o| (o.start_time, o.end_time)).collect(),
        HitObjects::Mania(objects) => {
            let mut objects = objects.clone();
            if plan.mods.as_ref().is_some_and(|mods| mods.inverse) {
                objects = apply_inverse_mod(&objects, &beatmap.timing_points);
            }
            if plan.mods.as_ref().is_some_and(|mods| mods.hold_off) {
                objects = apply_hold_off_mod(&objects);
            }
            objects.iter().map(|o| (o.start_time, o.end_time)).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{artifact::ArtifactName, request::RenderRequest};
    use osu_beatmap_preview_core::processing::validation::TimePoint;

    #[test]
    fn all_gif_modes_name_complete_selections_and_keep_full_requests_stable() {
        let _guard = crate::logging::test_guard();
        for mode in 0..=3 {
            let objects = (0..240)
                .map(|index| format!("256,192,{},1,0,0:0:0:0:\n", 1000 + index * 500))
                .collect::<String>();
            let source = format!("osu file format v14\n[General]\nMode:{mode}\nPreviewTime:30000\n[Difficulty]\nCircleSize:4\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n{objects}");
            let beatmap = osu_beatmap_preview_core::parse_beatmap_bytes(source.as_bytes()).unwrap();
            let mut request = RenderRequest::new("123");
            request.output.format = Some("gif".into());
            request.view.time_points = vec![TimePoint::Seconds(2.0)];
            let mut plan =
                RenderPlan::build(request.clone().validate().unwrap(), mode, false).unwrap();
            select(&beatmap, &mut plan).unwrap();
            let times = plan.selected_times.as_ref().unwrap();
            assert_eq!(times.len(), 4);
            assert!(times.contains(&3000));
            assert!(times.contains(&30_000));
            let name = ArtifactName::from_plan(&plan);
            for time in times {
                assert!(name.as_str().contains(&format!("{time}ms")));
            }

            // 显式填满四格时不补入 Preview Time，去重排序后命名也保持稳定。
            request.view.time_points = [40.0, 0.0, 10.0, 20.0]
                .into_iter()
                .map(TimePoint::Seconds)
                .collect();
            let mut explicit = RenderPlan::build(request.validate().unwrap(), mode, false).unwrap();
            select(&beatmap, &mut explicit).unwrap();
            assert_eq!(
                explicit.selected_times,
                Some(vec![1000, 11_000, 21_000, 41_000])
            );
            let first = ArtifactName::from_plan(&explicit).as_str().to_string();
            select(&beatmap, &mut explicit).unwrap();
            assert_eq!(ArtifactName::from_plan(&explicit).as_str(), first);
        }
    }
}
