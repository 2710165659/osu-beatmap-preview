//! Mania WGPU 实时帧源准备。

use std::sync::Arc;

use crate::domain::errors::{PreviewError, Result};
use crate::domain::models::Beatmap;
use crate::domain::mods::ModSettings;
use crate::domain::parser::round_half_even;
use crate::render::canvas::{Img, Rgba};
use crate::render::cpu::modes::mania::animation::{
    build_layout, build_scroll_map, compute_time_range, segment_left, visible_pos_window,
};
use crate::render::cpu::modes::mania::skin::load_mania_skin_config;
use crate::render::cpu::modes::mania::{
    apply_hold_off_mod, apply_inverse_mod, build_sv_changes, darken, is_native_mania,
    mania_objects, resolve_key_count,
};
use crate::render::geometry::{GameMode, OutputFormat};
use crate::render::scene::{FrameSceneBuilder, SceneRect};
use crate::render::text::render_text_sprite;
use crate::render::wgpu::RealtimeFrameSource;

/// 为 Mania 会话预计算 SV 距离轴和物件位置，seek 时只做二分裁剪与绘制。
pub(crate) fn prepare_realtime(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
) -> Result<RealtimeFrameSource> {
    let key_count = resolve_key_count(beatmap)?;
    let palette = crate::render::cpu::modes::mania::lane_palette(key_count);
    let original_objects = mania_objects(beatmap);
    let mut hit_objects = original_objects.clone();
    if mods.is_some_and(|value| value.inverse) {
        hit_objects = apply_inverse_mod(&hit_objects, &beatmap.timing_points);
    }
    if mods.is_some_and(|value| value.hold_off) {
        hit_objects = apply_hold_off_mod(&hit_objects);
    }
    if hit_objects.is_empty() {
        return Err(PreviewError::render("mania beatmap has no hit objects"));
    }
    let speed = mods.map_or(1.0, |value| value.speed_multiplier);
    let skin_config = load_mania_skin_config(key_count, OutputFormat::Mp4);
    let layout = build_layout(&skin_config, 1, false, OutputFormat::Mp4);
    let native_mania = is_native_mania(beatmap);
    let cs_mode = mods.is_some_and(|value| value.cs_override);
    let scroll_map = build_scroll_map(beatmap, &original_objects, cs_mode, native_mania);
    let time_range = compute_time_range(
        speed,
        skin_config.hit_position,
        crate::render::cpu::modes::mania::constants::DEFAULT_SCROLL_SPEED,
    );
    let pixels_per_scroll_unit = layout.scroll_length as f64 / time_range;
    let last = original_objects
        .iter()
        .map(|object| object.end_time)
        .max()
        .unwrap_or(0);
    let sv_changes = if cs_mode
        || !native_mania
        || !crate::infrastructure::config::current()
            .render
            .mania
            .mp4
            .style
            .SHOW_SV_LABEL
    {
        Vec::new()
    } else {
        build_sv_changes(&beatmap.timing_points, last + round_half_even(time_range))
    };
    let sv_positions = sv_changes
        .iter()
        .map(|&(time, _)| scroll_map.position_at(time as f64))
        .collect::<Vec<_>>();
    let hold_colors = palette
        .iter()
        .map(|&color| darken(color, 0.5))
        .collect::<Vec<Rgba>>();
    let pos_start = hit_objects
        .iter()
        .map(|object| scroll_map.position_at(object.start_time as f64))
        .collect::<Vec<_>>();
    let pos_end = hit_objects
        .iter()
        .map(|object| scroll_map.position_at(object.end_time as f64))
        .collect::<Vec<_>>();
    let max_hold_position = pos_start
        .iter()
        .zip(&pos_end)
        .map(|(&start, &end)| (end - start).max(0.0))
        .fold(0.0_f64, f64::max);
    let image_background = crate::infrastructure::config::current()
        .render
        .mania
        .mp4
        .style
        .IMAGE_BACKGROUND;
    let sv_sprites = sv_changes
        .iter()
        .zip(&sv_positions)
        .map(|(&(_, sv), &position)| {
            let label = crate::render::cpu::modes::mania::format_sv_label(sv);
            let image = render_text_sprite(&label, layout.sv_font_size, [255, 255, 255, 255]);
            (position, Arc::new(image))
        })
        .collect::<Vec<(f64, Arc<Img>)>>();
    Ok(RealtimeFrameSource::new(
        GameMode::Mania,
        move |absolute_time_ms| {
            let snapshot_pos = scroll_map.position_at(absolute_time_ms as f64);
            let left = segment_left(0, &layout);
            let mut scene = FrameSceneBuilder::new(
                layout.image_width as u32,
                layout.image_height as u32,
                absolute_time_ms,
            );
            draw_background_scene(&mut scene, left, &layout, image_background);
            draw_sv_scene(
                &mut scene,
                &sv_sprites,
                left,
                snapshot_pos,
                &layout,
                pixels_per_scroll_unit,
            );
            let (lo_pos, hi_pos) = visible_pos_window(
                snapshot_pos,
                &layout,
                pixels_per_scroll_unit,
                max_hold_position,
            );
            let first_visible = pos_start.partition_point(|&position| position < lo_pos);
            for index in first_visible..hit_objects.len() {
                if pos_start[index] > hi_pos {
                    break;
                }
                draw_hit_object_scene(
                    &mut scene,
                    &hit_objects[index],
                    &palette,
                    &hold_colors,
                    left,
                    pos_start[index],
                    pos_end[index],
                    snapshot_pos,
                    &layout,
                    pixels_per_scroll_unit,
                );
            }
            Ok(scene.finish())
        },
    ))
}

fn rect(x: i64, y: i64, width: i64, height: i64) -> SceneRect {
    SceneRect {
        x: x as f32,
        y: y as f32,
        width: width.max(0) as f32,
        height: height.max(0) as f32,
    }
}

fn draw_background_scene(
    scene: &mut FrameSceneBuilder,
    left: i64,
    layout: &crate::render::cpu::modes::mania::animation::AnimationLayout,
    image_background: Rgba,
) {
    scene.rectangle(
        rect(0, 0, layout.image_width, layout.image_height),
        image_background,
    );
    let top = layout.playfield_top;
    let lane_left = left + layout.left_panel_width;
    let lane_right = lane_left + layout.lane_area_width;
    scene.rectangle(
        rect(left, top, layout.segment_width, layout.playfield_height),
        layout.lane_background,
    );
    scene.rectangle(
        rect(left, top, layout.left_panel_width, layout.playfield_height),
        layout.left_panel_background,
    );
    scene.rectangle(
        rect(
            lane_right,
            top,
            layout.left_panel_width,
            layout.playfield_height,
        ),
        layout.left_panel_background,
    );
    for (lane, &width) in layout.column_widths.iter().enumerate() {
        scene.rectangle(
            rect(
                lane_left + layout.column_left_offsets[lane],
                top,
                width,
                layout.playfield_height,
            ),
            layout.column_background,
        );
    }
    let judgement_y = top + layout.hit_position_y;
    scene.line(
        [left as f32, judgement_y as f32],
        [(left + layout.segment_width) as f32, judgement_y as f32],
        (2.0 * layout.render_scale).max(1.0) as f32,
        layout.judgement_line_color,
    );
}

fn draw_sv_scene(
    scene: &mut FrameSceneBuilder,
    sv_sprites: &[(f64, Arc<Img>)],
    left: i64,
    snapshot_pos: f64,
    layout: &crate::render::cpu::modes::mania::animation::AnimationLayout,
    pixels_per_scroll_unit: f64,
) {
    let (lo_pos, hi_pos) = visible_pos_window(snapshot_pos, layout, pixels_per_scroll_unit, 0.0);
    let start = sv_sprites.partition_point(|(position, _)| *position < lo_pos);
    for (position, sprite) in &sv_sprites[start..] {
        if *position > hi_pos {
            break;
        }
        let y = y_at_position(*position, snapshot_pos, layout, pixels_per_scroll_unit);
        if !(layout.playfield_top..=layout.playfield_top + layout.playfield_height).contains(&y) {
            continue;
        }
        let x = (left - layout.sv_info_margin_left + layout.sv_label_inset).max(0);
        let label_y = (y as f64 - sprite.h as f64 / 2.0).floor() as i64;
        scene.glyph(
            Arc::clone(sprite),
            rect(x, label_y, sprite.w as i64, sprite.h as i64),
            layout.sv_text_color,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_hit_object_scene(
    scene: &mut FrameSceneBuilder,
    hit_object: &crate::domain::models::ManiaHitObject,
    palette: &[Rgba],
    hold_colors: &[Rgba],
    left: i64,
    start_pos: f64,
    end_pos: f64,
    snapshot_pos: f64,
    layout: &crate::render::cpu::modes::mania::animation::AnimationLayout,
    pixels_per_scroll_unit: f64,
) {
    let y_start = y_at_position(start_pos, snapshot_pos, layout, pixels_per_scroll_unit);
    let y_end = y_at_position(end_pos, snapshot_pos, layout, pixels_per_scroll_unit);
    let top = layout.playfield_top;
    let bottom = top + layout.playfield_height;
    if y_start.max(y_end) < top - layout.note_head_height
        || y_start.min(y_end) > bottom + layout.note_head_height
    {
        return;
    }
    let lane = (hit_object.lane.max(0) as usize).min(layout.column_widths.len() - 1);
    let lane_left = left
        + layout.left_panel_width
        + layout.column_left_offsets[lane]
        + layout.note_side_padding;
    let lane_width = layout.column_widths[lane] - layout.note_side_padding * 2;
    if hit_object.is_long_note {
        let body_top = top.max(y_end.min(y_start - layout.note_head_height));
        let body_bottom = bottom.min(y_start);
        if body_top < body_bottom {
            scene.rectangle(
                rect(lane_left, body_top, lane_width, body_bottom - body_top),
                hold_colors[lane.min(hold_colors.len() - 1)],
            );
        }
    }
    let head_top = top.max(y_start - layout.note_head_height);
    let head_bottom = bottom.min(y_start);
    if head_top < head_bottom {
        scene.rectangle(
            rect(lane_left, head_top, lane_width, head_bottom - head_top),
            palette[lane.min(palette.len() - 1)],
        );
    }
}

fn y_at_position(
    object_pos: f64,
    snapshot_pos: f64,
    layout: &crate::render::cpu::modes::mania::animation::AnimationLayout,
    pixels_per_scroll_unit: f64,
) -> i64 {
    layout.playfield_top + layout.hit_position_y
        - round_half_even((object_pos - snapshot_pos) * pixels_per_scroll_unit)
}
