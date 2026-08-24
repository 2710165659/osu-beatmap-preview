//! osu!standard PNG grid renderer: 5×8 gameplay snapshots.

use crate::common::time_selection::TimeAxis;
use crate::core::errors::Result;
use crate::core::models::Beatmap;
use crate::core::mods::ModSettings;
use crate::render::canvas::Img;
use crate::render::text::format_mmssmmm;

use super::constants::*;
use super::context::*;
use super::draw_time_label;
use super::objects::render_frame;

pub(crate) fn render_standard_png(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    times_ms: Option<Vec<i64>>,
    time_axis: TimeAxis,
) -> Result<Img> {
    let hit_objects = standard_objects(beatmap)?;
    let hit_objects = apply_standard_object_mods(hit_objects, mods);
    let context = build_render_context(beatmap, hit_objects, mods, time_axis);
    let row_timings = choose_row_start_times(
        beatmap,
        &context.hit_objects,
        PNG_ROW_COUNT,
        PNG_IMAGES_PER_ROW,
        PNG_MS_PER_IMAGE,
        times_ms,
    )?;

    let (canvas_w, canvas_h) = png_canvas_size();
    let mut canvas = Img::new(canvas_w as u32, canvas_h as u32, CANVAS_BACKGROUND_COLOR);
    let mut cache = RenderCache::default();

    for (row_index, row_timing) in row_timings.iter().enumerate() {
        let snapshot_times: Vec<i64> = (0..PNG_IMAGES_PER_ROW)
            .map(|i| row_timing.start_time + i as i64 * PNG_MS_PER_IMAGE)
            .collect();
        let visible_groups = build_visible_indexes_by_snapshot(
            &context.hit_objects,
            &snapshot_times,
            context.settings.preempt_ms,
        );
        let y = VERTICAL_PAGE_MARGIN
            + row_index as i64
                * (IMAGE_HEIGHT + TIME_LABEL_TOP_GAP + TIME_LABEL_HEIGHT + INTER_ROW_GAP);
        for image_index in 0..PNG_IMAGES_PER_ROW {
            let snapshot_time = snapshot_times[image_index];
            let x =
                HORIZONTAL_PAGE_MARGIN + image_index as i64 * (IMAGE_WIDTH + INTRA_ROW_IMAGE_GAP);
            let empty_breaks: Vec<crate::core::models::BreakPeriod> = Vec::new();
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
            );
            canvas.alpha_composite(&frame, x, y);
            let preview_note =
                (image_index == 0 && row_timing.is_preview).then_some("Preview Time");
            let is_preview_label = row_timing.is_preview;
            let bpm_note = crate::render::timing::bpm_at(&beatmap.timing_points, snapshot_time)
                .map(crate::render::timing::format_bpm);
            let combined_note = match (preview_note, bpm_note.as_deref()) {
                (Some(preview), Some(bpm)) => Some(format!("{preview} | {bpm}")),
                (Some(preview), None) => Some(preview.to_owned()),
                (None, Some(bpm)) => Some(bpm.to_owned()),
                (None, None) => None,
            };
            draw_time_label(
                &mut canvas,
                &format_mmssmmm(time_axis.to_display(snapshot_time)),
                x,
                y + IMAGE_HEIGHT + TIME_LABEL_TOP_GAP,
                combined_note.as_deref(),
                if is_preview_label {
                    PREVIEW_TIME_LABEL_COLOR
                } else {
                    TIME_LABEL_COLOR
                },
                if is_preview_label {
                    PREVIEW_TIME_LABEL_COLOR
                } else {
                    TIME_LABEL_NOTE_COLOR
                },
            );
        }
    }
    Ok(canvas)
}
