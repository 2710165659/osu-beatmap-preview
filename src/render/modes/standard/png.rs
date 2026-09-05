//! osu!standard PNG 网格渲染器：5×8 游戏画面快照。

use crate::domain::errors::Result;
use crate::domain::models::Beatmap;
use crate::domain::mods::ModSettings;
use crate::domain::shared::time_selection::TimeAxis;
use crate::domain::timeout::RequestDeadline;
use crate::render::canvas::Img;
use crate::render::text::format_mmssmmm;

use super::context::*;
use super::draw_time_label;
use super::objects::render_frame;

pub(crate) fn render_standard_png(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    time_axis: TimeAxis,
    times_ms: Option<Vec<i64>>,
    deadline: &RequestDeadline,
) -> Result<Img> {
    deadline.check()?;
    let hit_objects = standard_objects(beatmap)?;
    let hit_objects = apply_standard_object_mods(hit_objects, mods);
    let context = build_render_context(
        beatmap,
        hit_objects,
        mods,
        time_axis,
        crate::render::geometry::OutputFormat::Png,
    );
    let row_timings = choose_row_start_times(
        beatmap,
        &context.hit_objects,
        crate::infrastructure::config::current()
            .render
            .standard
            .png
            .structure
            .ROW_COUNT,
        crate::infrastructure::config::current()
            .render
            .standard
            .png
            .structure
            .IMAGES_PER_ROW,
        crate::infrastructure::config::current()
            .render
            .standard
            .png
            .style
            .MS_PER_IMAGE,
        times_ms,
    )?;

    let (canvas_w, canvas_h) = png_canvas_size();
    let mut canvas = Img::new(
        canvas_w as u32,
        canvas_h as u32,
        crate::infrastructure::config::current()
            .render
            .standard
            .png
            .style
            .CANVAS_BACKGROUND_COLOR,
    );
    let mut cache = RenderCache::default();

    for (row_index, row_timing) in row_timings.iter().enumerate() {
        deadline.check()?;
        let snapshot_times: Vec<i64> = (0..crate::infrastructure::config::current()
            .render
            .standard
            .png
            .structure
            .IMAGES_PER_ROW)
            .map(|i| {
                row_timing.start_time
                    + i as i64
                        * crate::infrastructure::config::current()
                            .render
                            .standard
                            .png
                            .style
                            .MS_PER_IMAGE
            })
            .collect();
        let visible_groups = build_visible_indexes_by_snapshot(
            &context.hit_objects,
            &snapshot_times,
            context.settings.preempt_ms,
        );
        let config = &crate::infrastructure::config::current().render.standard.png;
        let unit_height = context.frame_layout.frame_height
            + config.sizing.INFO_MARGIN_TOP
            + config.sizing.INFO_MARGIN_BOTTOM;
        let y = config.sizing.PAGE_MARGIN_TOP
            + config.sizing.INFO_MARGIN_TOP
            + row_index as i64 * (unit_height + config.sizing.ROW_GAP);
        for image_index in 0..crate::infrastructure::config::current()
            .render
            .standard
            .png
            .structure
            .IMAGES_PER_ROW
        {
            deadline.check()?;
            let snapshot_time = snapshot_times[image_index];
            let unit_width = context.frame_layout.frame_width
                + config.sizing.INFO_MARGIN_LEFT
                + config.sizing.INFO_MARGIN_RIGHT;
            let x = config.sizing.PAGE_MARGIN_LEFT
                + config.sizing.INFO_MARGIN_LEFT
                + image_index as i64 * (unit_width + config.sizing.COLUMN_GAP);
            let empty_breaks: Vec<crate::domain::models::BreakPeriod> = Vec::new();
            let breaks = if row_timing.is_preview {
                &row_timing.break_periods
            } else {
                &empty_breaks
            };
            let frame = render_frame(
                &context,
                &mut cache,
                snapshot_time,
                breaks,
                &visible_groups[image_index],
                None,
            );
            canvas.alpha_composite(&frame, x, y);
            let note = if image_index == 0 && row_timing.is_preview {
                Some("Preview Time")
            } else {
                None
            };
            let is_preview_label = row_timing.is_preview;
            draw_time_label(
                &mut canvas,
                &format_mmssmmm(time_axis.to_display(snapshot_time)),
                x,
                y + context.frame_layout.frame_height + config.sizing.TIME_LABEL_TOP_GAP,
                note,
                context.frame_layout.frame_width,
                config.sizing.TIME_LABEL_FONT_SIZE,
                config.sizing.TIME_LABEL_NOTE_FONT_SIZE,
                config.sizing.TIME_LABEL_NOTE_TOP_GAP,
                if is_preview_label {
                    crate::infrastructure::config::current()
                        .render
                        .standard
                        .png
                        .style
                        .PREVIEW_TIME_LABEL_COLOR
                } else {
                    crate::infrastructure::config::current()
                        .render
                        .standard
                        .png
                        .style
                        .TIME_LABEL_COLOR
                },
                if is_preview_label {
                    crate::infrastructure::config::current()
                        .render
                        .standard
                        .png
                        .style
                        .PREVIEW_TIME_LABEL_COLOR
                } else {
                    crate::infrastructure::config::current()
                        .render
                        .standard
                        .png
                        .style
                        .TIME_LABEL_NOTE_COLOR
                },
            );
        }
    }
    Ok(canvas)
}
