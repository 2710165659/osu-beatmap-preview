//! 回放 PNG：复用 Standard 快照网格，每格按绝对时间绘制回放与 HUD。
use super::font;
use crate::{
    application::{legacy::resolve_time_points, output_plan::RenderPlan},
    export::standard::png::render_grid,
    media::image::save_png,
};
use osu_beatmap_preview_core::{
    gameplay::timeline::GameplayTimeline,
    processing::timeline::{PreviewTimeSelection, TimeAxis},
    render::{
        cpu::modes::standard::{context::build_replay_context, osr},
        geometry::OutputFormat,
    },
    replay::preparation::standard::PreparedStandard,
    support::timeout::RequestDeadline,
    Beatmap, Result,
};
use std::path::Path;

pub(crate) fn render_png(
    beatmap: &Beatmap,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    plan: &RenderPlan,
    path: &Path,
    deadline: &RequestDeadline,
) -> Result<()> {
    deadline.check()?;
    let axis = TimeAxis::new(prepared.objects[0].source.start_time);
    let context = build_replay_context(beatmap, prepared, axis, OutputFormat::Png);
    let requested = match &plan.selected_times {
        Some(times) => PreviewTimeSelection::Resolved(times.clone()),
        None => {
            PreviewTimeSelection::Requested(resolve_time_points(beatmap, axis, &plan.time_points)?)
        }
    };
    let render_plan = osr::ReplayRenderPlan::new(&context, prepared, gameplay);
    let visuals = osr::ReplayVisuals::new(
        &context,
        gameplay,
        font::load()?,
        &beatmap.break_periods,
        prepared.od,
    );
    let image = render_grid(
        beatmap,
        &context,
        axis,
        requested,
        deadline,
        |cache, time, breaks, _| {
            let mut image = osr::render_frame(
                &context,
                cache,
                prepared,
                gameplay,
                time,
                breaks,
                &render_plan,
                None,
            );
            osr::draw_overlay(&mut image, &context, prepared, gameplay, &visuals, time);
            image
        },
    )?;
    save_png(&image, path, deadline)
}
