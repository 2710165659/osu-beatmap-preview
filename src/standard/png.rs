//! osu!standard PNG grid renderer: 5×8 gameplay snapshots.

use crate::canvas::Img;
use crate::errors::Result;
use crate::models::Beatmap;
use crate::mods::ModSettings;
use crate::text::format_mmssmmm;

use super::constants::*;
use super::context::*;
use super::objects::render_frame;
use super::draw_time_label;

pub(crate) fn render_standard_png(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
    times_ms: Option<Vec<i64>>,
) -> Result<Img> {
    let hit_objects = standard_objects(beatmap)?;
    let hit_objects = apply_standard_object_mods(hit_objects, mods);
    let context = build_render_context(beatmap, hit_objects, mods);
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
            let empty_breaks: Vec<crate::models::BreakPeriod> = Vec::new();
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
            let note = if image_index == 0 && row_timing.is_preview {
                Some("Preview Time")
            } else {
                None
            };
            let is_preview_label = row_timing.is_preview;
            draw_time_label(
                &mut canvas,
                &format_mmssmmm(snapshot_time),
                x,
                y + IMAGE_HEIGHT + TIME_LABEL_TOP_GAP,
                note,
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


