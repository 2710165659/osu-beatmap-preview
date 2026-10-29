//! Catch 回放 GIF：每格独立采样真实回放，复用普通 GIF 的布局、选段和调色板。
use crate::{
    application::{legacy::resolve_time_points, output_plan::RenderPlan},
    export::replay::{font, time_plan::checked_gif_frame_count},
    media::image::save_animated_gif_streamed_with_colors,
};
use osu_beatmap_preview_core::{
    gameplay::timeline::GameplayTimeline,
    processing::timeline::{PreviewTimeSelection, TimeAxis},
    render::{
        canvas::Img,
        cpu::modes::catch::{animation::*, constants},
        geometry::OutputFormat,
        shared::catch::ReplayRenderPlan,
    },
    replay::preparation::catch::PreparedCatch,
    support::timeout::RequestDeadline,
    PreviewError, Result,
};
use std::{path::Path, sync::Arc};

pub(crate) fn render_gif(
    prepared: Arc<PreparedCatch>,
    gameplay: Arc<GameplayTimeline>,
    plan: &RenderPlan,
    path: &Path,
    deadline: &RequestDeadline,
) -> Result<()> {
    render_with_config(
        prepared,
        gameplay,
        plan,
        path,
        deadline,
        crate::config::current(),
    )
}

fn render_with_config(
    prepared: Arc<PreparedCatch>,
    gameplay: Arc<GameplayTimeline>,
    plan: &RenderPlan,
    path: &Path,
    deadline: &RequestDeadline,
    runtime: &crate::config::RuntimeConfig,
) -> Result<()> {
    deadline.check()?;
    let gif = &runtime.render.catch.gif;
    let cells = gif
        .structure
        .ROW_COUNT
        .checked_mul(gif.structure.IMAGES_PER_ROW)
        .filter(|&cells| cells > 0)
        .ok_or_else(|| PreviewError::render("回放 GIF 网格行列数必须为正数且不能溢出"))?;
    let speed = prepared.mods.settings.speed_multiplier;
    let duration_ms = plan
        .duration_seconds
        .map_or(gif.style.DURATION_MS, |seconds| seconds * 1000.0);
    let fps = plan.fps.map_or(gif.style.FPS, f64::from);
    let frame_count = checked_gif_frame_count(duration_ms, fps, speed, cells)?;
    let duration = osu_beatmap_preview_core::domain::parser::round_half_even(duration_ms * speed);
    let axis = TimeAxis::new(prepared.first);
    let requested = match &plan.selected_times {
        Some(times) => PreviewTimeSelection::Resolved(times.clone()),
        None => PreviewTimeSelection::Requested(resolve_time_points(
            &prepared.beatmap,
            axis,
            &plan.time_points,
        )?),
    };
    let spans = prepared
        .objects
        .iter()
        .map(|object| (object.start_time, object.start_time))
        .collect();
    let segments = requested.choose(&prepared.beatmap, spans, cells, duration)?;
    let renderer = ReplayRenderPlan::with_format(
        Arc::clone(&prepared),
        gameplay,
        font::load()?,
        None,
        OutputFormat::Gif,
    )?;
    let layout = &renderer.layout;
    let (width, height) = (layout.canvas_width, layout.canvas_height);
    if !(1..=i64::from(u16::MAX)).contains(&width) || !(1..=i64::from(u16::MAX)).contains(&height) {
        return Err(PreviewError::render("回放 GIF 画布尺寸超出格式范围"));
    }
    let origins: Vec<_> = (0..segments.len())
        .map(|index| frame_origin(index, layout))
        .collect();
    let blank = Img::new(
        layout.frame_width as u32,
        layout.frame_height as u32,
        constants::gif::style::PLAYFIELD_BACKGROUND,
    );
    let config = Arc::new(osu_beatmap_preview_core::config::CoreConfig {
        render: runtime.render.clone(),
        skin: runtime.skin.clone(),
    });
    // 编码器并行绘帧时显式安装调用线程的配置，避免字号、样式和缩放回退。
    let draw = |index: usize| {
        osu_beatmap_preview_core::config::with_config(Arc::clone(&config), || {
            let mut canvas = Img::new(
                width as u32,
                height as u32,
                constants::gif::style::PLAYFIELD_BACKGROUND,
            );
            let offset = osu_beatmap_preview_core::domain::parser::round_half_even(
                index as f64 * 1000.0 * speed / fps,
            );
            for (cell, segment) in segments.iter().enumerate() {
                let time = segment.start_time.saturating_add(offset);
                let mut image = blank.clone();
                renderer.draw_frame(&mut image, time);
                renderer.draw_overlay(&mut image, time);
                let origin = origins[cell];
                canvas.alpha_composite(&image, origin.0, origin.1);
                if constants::gif::style::SHOW_TIME_LABEL {
                    draw_gif_time_label(
                        &mut canvas,
                        segment.start_time,
                        duration,
                        origin.0,
                        origin.1,
                        segment.is_preview,
                        axis,
                        layout,
                    );
                }
            }
            canvas
        })
    };
    save_animated_gif_streamed_with_colors(
        frame_count,
        draw,
        path,
        fps,
        deadline,
        &crate::export::gif_time_label_colors(
            constants::gif::style::SHOW_TIME_LABEL,
            constants::gif::style::PLAYFIELD_BACKGROUND,
            [
                constants::gif::style::PREVIEW_TIME_LABEL_COLOR,
                constants::gif::style::TIME_LABEL_COLOR,
                constants::gif::style::TIME_LABEL_NOTE_COLOR,
            ],
        ),
        prepared.mods.settings.hidden || prepared.mods.settings.flashlight,
    )
}
