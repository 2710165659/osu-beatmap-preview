//! Mania 回放共享绘制计划：CPU 导出与实时场景消费相同的输入、长条状态和 HUD。
use crate::{
    gameplay::{events::EventKind, timeline::GameplayTimeline},
    processing::timeline::TimeAxis,
    render::{
        canvas::{Img, Rgba},
        cpu::modes::mania::{
            animation::*, build_sv_changes, darken, format_sv_label, is_native_mania, lane_palette,
            skin::load_mania_skin_config,
        },
        geometry::{scale_px, OutputFormat, PixelRect, MANIA_PLAYFIELD_HEIGHT},
        scene::FrameSceneBuilder,
        shared::replay::{
            font::ArgonFont,
            hud::{self, HudLayout, HudVisuals},
            overlay::OverlayCanvas,
        },
        visibility::{ManiaHidden, VisibilityTimeline},
    },
    replay::preparation::mania::PreparedMania,
    Result,
};
use std::sync::Arc;

pub struct ReplayRenderPlan {
    pub layout: AnimationLayout,
    prepared: Arc<PreparedMania>,
    gameplay: Arc<GameplayTimeline>,
    scroll: ScrollMap,
    starts: Vec<f64>,
    ends: Vec<f64>,
    max_hold: f64,
    pixels_per_unit: f64,
    track: Arc<Img>,
    palette: Vec<Rgba>,
    visibility: VisibilityTimeline,
    hud: HudVisuals,
    judgement_center: [i64; 2],
    sv_labels: Vec<(f64, Arc<Img>)>,
    holds: Vec<HoldVisual>,
    hit_effects: Option<super::hit_effects::HitEffects>,
}
impl ReplayRenderPlan {
    pub fn new(
        prepared: Arc<PreparedMania>,
        gameplay: Arc<GameplayTimeline>,
        font: Option<ArgonFont>,
        dimensions: Option<(u32, u32)>,
    ) -> Result<Self> {
        Self::with_format(prepared, gameplay, font, dimensions, OutputFormat::Mp4)
    }
    pub fn with_format(
        prepared: Arc<PreparedMania>,
        gameplay: Arc<GameplayTimeline>,
        font: Option<ArgonFont>,
        dimensions: Option<(u32, u32)>,
        format: OutputFormat,
    ) -> Result<Self> {
        let skin = load_mania_skin_config(prepared.keys, format);
        let mut layout = if format == OutputFormat::Gif {
            build_layout(&skin, format)
        } else {
            build_video_layout(&skin, format)
        };
        reserve_hud_space(&mut layout);
        if let Some((width, height)) = dimensions {
            let dx = (i64::from(width) - layout.image_width) / 2;
            let dy = (i64::from(height) - layout.image_height) / 2;
            layout.playfield_left += dx;
            layout.playfield_top += dy;
            layout.content.x += dx;
            layout.content.y += dy;
            layout.image_width = width.into();
            layout.image_height = height.into();
        }
        let mut track = Img::new(
            layout.segment_width as u32,
            layout.playfield_height as u32,
            [0; 4],
        );
        let mut track_layout = build_layout(&skin, format);
        reserve_hud_space(&mut track_layout);
        draw_segment_background(&mut track, 0, &track_layout);
        let scroll = build_scroll_map(
            &prepared.beatmap,
            &prepared.original_objects,
            prepared.mods.settings.cs_override,
            is_native_mania(&prepared.beatmap),
        );
        let starts: Vec<_> = prepared
            .objects
            .iter()
            .map(|o| scroll.position_at(o.start_time as f64))
            .collect();
        let ends: Vec<_> = prepared
            .objects
            .iter()
            .map(|o| scroll.position_at(o.end_time as f64))
            .collect();
        let max_hold = starts
            .iter()
            .zip(&ends)
            .map(|(a, b)| (b - a).max(0.0))
            .fold(0.0, f64::max);
        let config = crate::config::current();
        let scroll_speed = if format == OutputFormat::Gif {
            config.render.mania.gif.style.SCROLL_SPEED
        } else {
            crate::render::cpu::modes::mania::constants::DEFAULT_SCROLL_SPEED
        };
        let time_range = compute_time_range(
            prepared.mods.settings.speed_multiplier,
            skin.hit_position,
            scroll_speed,
        );
        let center = layout.playfield_left + layout.segment_width / 2;
        let skin_y = |position: i64| {
            layout.playfield_top
                + scale_px(
                    position as f64 / 480.0 * MANIA_PLAYFIELD_HEIGHT,
                    layout.render_scale,
                )
        };
        let combo_center = [center, skin_y(skin.combo_position)];
        let judgement_center = [
            center,
            skin_y(skin.score_position).max(
                combo_center[1]
                    + scale_px(
                        (38.0 / 2.0 + super::feedback::MAX_HEIGHT / 2.0 + 12.0).max(53.0),
                        layout.render_scale,
                    ),
            ),
        ];
        let mut hud = HudVisuals::new(
            &gameplay,
            font,
            layout.render_scale,
            [
                prepared.windows[1],
                prepared.windows[3],
                prepared.windows[4],
            ],
        );
        hud.combo_center = Some(combo_center);
        hud.hit_error_bottom = 8.0;
        let visibility =
            VisibilityTimeline::from_replay(&gameplay, &prepared.beatmap.break_periods);
        let palette = lane_palette(prepared.keys);
        let holds = prepared
            .objects
            .iter()
            .zip(&gameplay.objects)
            .map(|(source, state)| {
                let drop = state.tracking.iter().find_map(|&(t, holding)| {
                    (t >= source.start_time as f64 && !holding).then_some(t)
                });
                let missing = state
                    .head
                    .filter(|p| !p.1.is_hit())
                    .map(|p| p.0)
                    .into_iter()
                    .chain(
                        state
                            .tracking
                            .iter()
                            .find_map(|&(t, holding)| (!holding).then_some(t)),
                    )
                    .chain(state.end.filter(|p| !p.1.is_hit()).map(|p| p.0))
                    .min_by(f64::total_cmp);
                HoldVisual {
                    drop: drop.map(|t| (t, scroll.position_at(t))),
                    missing,
                }
            })
            .collect();
        let show_hit_flash = match format {
            OutputFormat::Gif => config.render.mania.gif.style.SHOW_HIT_FLASH,
            _ => config.render.mania.mp4.style.SHOW_HIT_FLASH,
        };
        let hit_effects = show_hit_flash
            .then(|| super::hit_effects::HitEffects::new(&prepared, &gameplay, &palette, &layout));
        let pixels_per_unit = layout.scroll_length as f64 / time_range;
        let show_sv = match format {
            OutputFormat::Gif => config.render.mania.gif.style.SHOW_SV_LABEL,
            _ => config.render.mania.mp4.style.SHOW_SV_LABEL,
        };
        let sv_labels =
            if show_sv && is_native_mania(&prepared.beatmap) && !prepared.mods.settings.cs_override
            {
                build_sv_changes(
                    &prepared.beatmap.timing_points,
                    prepared.last_end.ceil() as i64,
                )
                .into_iter()
                .map(|(time, sv)| {
                    (
                        scroll.position_at(time as f64),
                        crate::render::text::shared_text_sprite(
                            &format_sv_label(sv),
                            layout.sv_font_size,
                            layout.sv_text_color,
                        ),
                    )
                })
                .collect()
            } else {
                Vec::new()
            };
        Ok(Self {
            layout,
            prepared,
            gameplay,
            scroll,
            starts,
            ends,
            max_hold,
            pixels_per_unit,
            track: Arc::new(track),
            palette,
            visibility,
            hud,
            judgement_center,
            sv_labels,
            holds,
            hit_effects,
        })
    }
    pub fn draw_frame(&self, target: &mut Img, time: i64) {
        self.draw_to(target, time);
    }
    pub fn draw_scene(&self, target: &mut FrameSceneBuilder, time: i64) {
        self.draw_to(target, time);
    }
    pub fn draw_overlay(&self, target: &mut Img, time: i64) {
        self.overlay_to(target, time);
    }
    pub fn draw_scene_overlay(&self, target: &mut FrameSceneBuilder, time: i64) {
        self.overlay_to(target, time);
    }

    fn draw_to(&self, target: &mut impl OverlayCanvas, time: i64) {
        let l = &self.layout;
        target.shared_sprite(&self.track, l.playfield_left, l.playfield_top);
        let pos = self.scroll.position_at(time as f64);
        let (lo, hi) = visible_pos_window(pos, l, self.pixels_per_unit, self.max_hold);
        let hidden = self
            .prepared
            .mods
            .settings
            .hidden
            .then(|| mania_hidden(&self.visibility, time, l));
        let stage_left = l.playfield_left + l.left_panel_width;
        let line = l.playfield_top + l.hit_position_y;
        let y_at = |p: f64| line - ((p - pos) * self.pixels_per_unit).round_ties_even() as i64;
        for id in self.starts.partition_point(|&p| p < lo)..self.starts.len() {
            if self.starts[id] > hi {
                break;
            }
            let source = &self.prepared.objects[id];
            let state = &self.gameplay.objects[id];
            let Some(opacity) = note_opacity(state, time as f64) else {
                continue;
            };
            let lane = source.lane as usize;
            let x = stage_left + l.column_left_offsets[lane] + l.note_side_padding;
            let w = (l.column_widths[lane] - 2 * l.note_side_padding).max(1);
            let mut y = y_at(self.starts[id]);
            let mut color = self.palette[lane];
            color[3] = (255.0 * opacity).round() as u8;
            if source.is_long_note {
                let visual = &self.holds[id];
                y = y_at(visual.head_position(state, self.starts[id], pos, time as f64));
                let end_y = y_at(self.ends[id]);
                color = visual.dim(color, time as f64);
                fill_note(
                    target,
                    x,
                    end_y,
                    w,
                    y - end_y,
                    shade(color),
                    l,
                    hidden.as_ref(),
                );
                // 参考皮肤的长条尾为透明占位，仅保留主体的平直收尾。
            }
            fill_note(
                target,
                x,
                y - l.note_head_height / 2,
                w,
                l.note_head_height,
                color,
                l,
                hidden.as_ref(),
            );
        }
        for (position, sprite) in &self.sv_labels {
            let y = y_at(*position);
            if (l.playfield_top..=l.playfield_top + l.playfield_height).contains(&y) {
                target.shared_sprite(
                    sprite,
                    (l.playfield_left - l.sv_info_margin_left + l.sv_label_inset).max(0),
                    y - i64::from(sprite.h) / 2,
                );
            }
        }
        // 接盘高亮只跟随实际输入，漏击不会触发自动击打动画。
        if let Some(input) = self.gameplay.input_at(time as f64) {
            for lane in 0..self.prepared.keys as usize {
                if input.keys & (1 << lane) == 0 {
                    continue;
                }
                let x = stage_left + l.column_left_offsets[lane];
                let color = self.palette[lane];
                target.fill_rect_size(
                    x,
                    line - scale_px(6.0, l.render_scale),
                    l.column_widths[lane],
                    scale_px(8.0, l.render_scale),
                    [color[0], color[1], color[2], 180],
                );
            }
        }
        if let Some(effects) = &self.hit_effects {
            effects.draw(target, time as f64, l);
        }
    }
    fn overlay_to(&self, target: &mut impl OverlayCanvas, time: i64) {
        let l = &self.layout;
        if self.prepared.mods.settings.flashlight {
            target.flashlight(
                &mania_flashlight(&self.visibility, time, l, l.playfield_left),
                PixelRect {
                    x: 0,
                    y: 0,
                    width: l.image_width,
                    height: l.image_height,
                },
            );
        }
        if let Some(event) = self
            .gameplay
            .events_between(time as f64 - super::feedback::LIFETIME, time as f64)
            .iter()
            .rev()
            .find(|e| {
                e.kind == EventKind::Judgement
                    && e.maximum == crate::gameplay::events::HitResult::Perfect
            })
        {
            super::feedback::draw(
                target,
                event.result,
                self.judgement_center,
                l.render_scale,
                time as f64 - event.time,
            );
        }
        hud::draw(
            target,
            HudLayout {
                width: l.image_width,
                height: l.image_height,
                scale: l.render_scale,
                time_axis: TimeAxis::new(self.prepared.objects[0].start_time),
            },
            &self.gameplay,
            &self.hud,
            time as f64,
            self.prepared
                .last_end
                .max(self.gameplay.settlement_time)
                .ceil() as i64,
        );
    }
}

struct HoldVisual {
    drop: Option<(f64, f64)>,
    missing: Option<f64>,
}

impl HoldVisual {
    fn head_position(
        &self,
        state: &crate::gameplay::state::ObjectState,
        start: f64,
        current: f64,
        time: f64,
    ) -> f64 {
        if !state
            .head
            .is_some_and(|(t, result)| t <= time && result.is_hit())
        {
            return start;
        }
        // 松键后保留已经吃掉的主体长度，头部从松键位置继续滚动；重接不能把它拉回判定线。
        let consumed = self.drop.filter(|p| p.0 <= time).map_or(current, |p| p.1);
        start.max(consumed)
    }

    fn dim(&self, color: Rgba, time: f64) -> Rgba {
        let Some(at) = self.missing.filter(|&at| at <= time) else {
            return color;
        };
        // LegacyBodyPiece 的 MissingStartTime 一旦设置便不会因重接清空，60ms 内持续变暗。
        let amount = 0.5 * ((time - at) / 60.0).clamp(0.0, 1.0);
        let mut result = darken(color, amount);
        result[3] = color[3];
        result
    }
}

// Legacy HUD 把偏移条固定在屏幕底部；回放舞台为它留出空间，并保留 Miss 穿线的滚动区域。
fn reserve_hud_space(layout: &mut AnimationLayout) {
    let line = layout
        .hit_position_y
        .min(layout.playfield_height - scale_px(44.0, layout.render_scale));
    layout.scroll_length -= layout.hit_position_y - line;
    layout.scroll_length = layout.scroll_length.max(1);
    layout.hit_position_y = line;
    layout.playfield_height -= scale_px(24.0, layout.render_scale);
}

fn note_opacity(state: &crate::gameplay::state::ObjectState, time: f64) -> Option<f64> {
    match state.end {
        Some((at, result)) if at <= time => {
            if result.is_hit() {
                None
            } else {
                let elapsed = (time - at) / 150.0;
                (elapsed < 1.0).then(|| 1.0 - elapsed.powi(2))
            }
        }
        _ => Some(1.0),
    }
}
fn shade(color: Rgba) -> Rgba {
    let mut result = darken(color, 0.5);
    result[3] = color[3];
    result
}

#[allow(clippy::too_many_arguments)]
fn fill_note(
    target: &mut impl OverlayCanvas,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    color: Rgba,
    layout: &AnimationLayout,
    hidden: Option<&ManiaHidden>,
) {
    let bottom = (y + h).min(layout.playfield_top + layout.playfield_height);
    let top = y.max(layout.playfield_top);
    if bottom <= top {
        return;
    }
    if let Some(hidden) = hidden {
        let mut from = top;
        while from < bottom {
            let alpha = (u16::from(hidden.alpha_at(from)) * u16::from(color[3]) / 255) as u8;
            let mut end = from + 1;
            while end < bottom
                && (u16::from(hidden.alpha_at(end)) * u16::from(color[3]) / 255) as u8 == alpha
            {
                end += 1;
            }
            if alpha > 0 {
                target.fill_rect_size(
                    x,
                    from,
                    w,
                    end - from,
                    [color[0], color[1], color[2], alpha],
                );
            }
            from = end;
        }
    } else {
        target.fill_rect_size(x, top, w, bottom - top, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{with_config, CoreConfig},
        render::scene::DrawCommand,
        replay::preparation::mania::tests::fixture,
    };
    #[test]
    fn hold_drop_preserves_consumed_length_and_regrab_does_not_blink_or_restore_tail_cap() {
        let (prepared, document) = fixture(
            "64,192,1000,128,0,3000:0:0:0:0:",
            &[(0, 0), (980, 1), (1500, 0), (1600, 1), (3000, 0), (3400, 0)],
        );
        let gameplay =
            Arc::new(crate::gameplay::engine::compile_mania(&prepared, document).unwrap());
        let prepared = Arc::new(prepared);
        for (format, scale) in [(OutputFormat::Gif, 0.75), (OutputFormat::Mp4, 1.25)] {
            let mut config = CoreConfig::default();
            config.render.mania.gif.SCALE = scale;
            config.render.mania.mp4.SCALE = scale;
            config.render.mania.gif.style.SHOW_HIT_FLASH = false;
            config.render.mania.mp4.style.SHOW_HIT_FLASH = false;
            with_config(Arc::new(config), || {
                let plan = ReplayRenderPlan::with_format(
                    Arc::clone(&prepared),
                    Arc::clone(&gameplay),
                    None,
                    None,
                    format,
                )
                .unwrap();
                let l = &plan.layout;
                let state = &gameplay.objects[0];
                let head = |time: f64| {
                    let position = plan.scroll.position_at(time);
                    let anchor = plan.holds[0].head_position(state, plan.starts[0], position, time);
                    (position - anchor) * plan.pixels_per_unit
                };
                assert_eq!(head(1400.0), 0.0, "按住期间头部固定在判定线");
                assert!(
                    head(1501.0) >= 0.0 && head(1501.0) < scale,
                    "松键时头部不能突然跳到屏幕下方"
                );
                assert!(head(1700.0) > head(1600.0), "重接后已经落下的头部继续滚动");
                assert_eq!(
                    plan.holds[0].dim(plan.palette[0], 1580.0),
                    plan.holds[0].dim(plan.palette[0], 1700.0),
                    "重接不能把断按主体突然点亮"
                );
                let time = 2900;
                let pos = plan.scroll.position_at(time as f64);
                let tail_y = l.playfield_top + l.hit_position_y
                    - ((plan.ends[0] - pos) * plan.pixels_per_unit).round_ties_even() as i64;
                let x = l.playfield_left
                    + l.left_panel_width
                    + l.column_left_offsets[0]
                    + l.column_widths[0] / 2;
                assert!(tail_y > l.playfield_top + l.note_head_height);
                let mut image = Img::new(l.image_width as u32, l.image_height as u32, [0; 4]);
                plan.draw_frame(&mut image, time);
                let background = plan.track.get(
                    (x - l.playfield_left) as u32,
                    (tail_y - 1 - l.playfield_top) as u32,
                );
                assert_eq!(
                    image.get(x as u32, (tail_y - 1) as u32),
                    background,
                    "透明尾部不应多画一个音符头"
                );
                assert_ne!(
                    image.get(x as u32, tail_y as u32),
                    background,
                    "尾部以下仍有长条主体"
                );
            });
        }
    }

    #[test]
    fn flash_switch_uses_own_format_and_cpu_and_scene_share_actual_hit_timing() {
        let (prepared, document) =
            fixture("64,192,1000,1,0", &[(0, 0), (980, 1), (990, 0), (1400, 0)]);
        let gameplay =
            Arc::new(crate::gameplay::engine::compile_mania(&prepared, document).unwrap());
        let prepared = Arc::new(prepared);
        for format in [OutputFormat::Gif, OutputFormat::Mp4] {
            for enabled in [false, true] {
                let mut config = CoreConfig::default();
                config.render.mania.gif.style.SHOW_HIT_FLASH = if format == OutputFormat::Gif {
                    enabled
                } else {
                    !enabled
                };
                config.render.mania.mp4.style.SHOW_HIT_FLASH = if format == OutputFormat::Mp4 {
                    enabled
                } else {
                    !enabled
                };
                with_config(Arc::new(config), || {
                    let plan = ReplayRenderPlan::with_format(
                        Arc::clone(&prepared),
                        Arc::clone(&gameplay),
                        None,
                        None,
                        format,
                    )
                    .unwrap();
                    let l = &plan.layout;
                    for time in [1060, 970, 1180, 1060] {
                        let mut image =
                            Img::new(l.image_width as u32, l.image_height as u32, [0; 4]);
                        plan.draw_frame(&mut image, time);
                        let mut scene = FrameSceneBuilder::new(image.w, image.h, time);
                        plan.draw_scene(&mut scene, time);
                        let flashing = enabled && time == 1060;
                        let scene = scene.finish();
                        assert_eq!(
                            scene.commands.iter().any(|command| matches!(
                                command,
                                DrawCommand::TransformedSprite { additive: true, .. }
                            )),
                            flashing
                        );
                        let x = l.playfield_left
                            + l.left_panel_width
                            + l.column_left_offsets[0]
                            + l.column_widths[0] / 2;
                        let y = l.playfield_top + l.hit_position_y;
                        if time >= 1060 {
                            let base = plan
                                .track
                                .get((x - l.playfield_left) as u32, (y - l.playfield_top) as u32);
                            assert_eq!(image.get(x as u32, y as u32) != base, flashing);
                        }
                    }
                });
            }
        }
    }
    #[test]
    fn hit_disappears_immediately_and_miss_fades_while_scrolling_past_line() {
        use crate::{gameplay::events::HitResult, gameplay::state::ObjectState};
        let hit = ObjectState {
            end: Some((980.0, HitResult::Perfect)),
            ..Default::default()
        };
        assert_eq!(note_opacity(&hit, 979.0), Some(1.0));
        assert_eq!(note_opacity(&hit, 980.0), None);
        let miss = ObjectState {
            end: Some((1100.0, HitResult::Miss)),
            ..Default::default()
        };
        assert_eq!(note_opacity(&miss, 1100.0), Some(1.0));
        assert_eq!(note_opacity(&miss, 1175.0), Some(0.75));
        assert_eq!(note_opacity(&miss, 1250.0), None);

        let (prepared, document) = fixture("64,192,1000,1,0", &[(0, 0), (2000, 0)]);
        let gameplay =
            Arc::new(crate::gameplay::engine::compile_mania(&prepared, document).unwrap());
        let prepared = Arc::new(prepared);
        for scale in [0.75, 1.25] {
            let mut config = CoreConfig::default();
            config.render.mania.gif.SCALE = scale;
            config.render.mania.gif.style.SCROLL_SPEED = 1.0;
            with_config(Arc::new(config), || {
                let plan = ReplayRenderPlan::with_format(
                    Arc::clone(&prepared),
                    Arc::clone(&gameplay),
                    None,
                    None,
                    OutputFormat::Gif,
                )
                .unwrap();
                let l = &plan.layout;
                assert!(
                    plan.judgement_center[1] - plan.hud.combo_center.unwrap()[1]
                        >= scale_px(53.0, scale)
                );
                assert!(l.hit_position_y < l.playfield_height);
                assert!(
                    l.playfield_top + l.playfield_height
                        < l.image_height - scale_px(8.0 + 4.0, scale)
                );
                let time = gameplay.objects[0].end.unwrap().0.ceil() as i64 + 30;
                let mut scene =
                    FrameSceneBuilder::new(l.image_width as u32, l.image_height as u32, time);
                plan.draw_scene(&mut scene, time);
                assert!(scene.finish().commands.iter().any(|command| matches!(command, DrawCommand::Rectangle { rect, color } if rect.y + rect.height > (l.playfield_top + l.hit_position_y) as f32 && color[3] > 0 && color[3] < 255)), "Miss 穿过判定线后仍然可见并逐渐淡出");
            });
        }
    }
    #[test]
    fn skin_positions_scale_per_format_and_gpu_keeps_judgements_as_geometry() {
        let (prepared, document) = fixture("64,192,1000,1,0", &[(0, 0), (1000, 1), (1400, 0)]);
        let gameplay =
            Arc::new(crate::gameplay::engine::compile_mania(&prepared, document).unwrap());
        let prepared = Arc::new(prepared);
        for (format, scale) in [(OutputFormat::Gif, 0.75), (OutputFormat::Mp4, 1.25)] {
            let mut config = CoreConfig::default();
            config.render.mania.gif.SCALE = if format == OutputFormat::Gif {
                scale
            } else {
                2.0
            };
            config.render.mania.mp4.SCALE = if format == OutputFormat::Mp4 {
                scale
            } else {
                3.0
            };
            config.skin.MANIA.KEYS_4.COMBO_POSITION = 123;
            config.skin.MANIA.KEYS_4.SCORE_POSITION = 205;
            with_config(Arc::new(config), || {
                let plan = ReplayRenderPlan::with_format(
                    Arc::clone(&prepared),
                    Arc::clone(&gameplay),
                    None,
                    None,
                    format,
                )
                .unwrap();
                assert_eq!(plan.layout.render_scale, scale);
                assert_eq!(
                    plan.hud.combo_center.unwrap()[1],
                    plan.layout.playfield_top + scale_px(123.0 / 480.0 * 384.0, scale)
                );
                assert_eq!(
                    plan.judgement_center[1],
                    plan.layout.playfield_top + scale_px(205.0 / 480.0 * 384.0, scale)
                );
                let mut image = Img::new(
                    plan.layout.image_width as u32,
                    plan.layout.image_height as u32,
                    [0; 4],
                );
                let mut scene = FrameSceneBuilder::new(image.w, image.h, 1100);
                super::super::feedback::draw(
                    &mut image,
                    crate::gameplay::events::HitResult::Perfect,
                    plan.judgement_center,
                    scale,
                    100.0,
                );
                super::super::feedback::draw(
                    &mut scene,
                    crate::gameplay::events::HitResult::Perfect,
                    plan.judgement_center,
                    scale,
                    100.0,
                );
                let scene = scene.finish();
                assert!(scene.resources.is_empty(), "判定由代码绘制，不上传皮肤图片");
                for command in scene.commands.iter() {
                    let DrawCommand::Rectangle { rect, color } = command else {
                        panic!("判定应由几何图元组成")
                    };
                    assert!(image.get(rect.x as u32, rect.y as u32)[3] > 0);
                    assert!(color[3] > 0);
                }
                let center = plan.hud.combo_center.unwrap();
                let mut overlay = Img::new(image.w, image.h, [0; 4]);
                plan.draw_overlay(&mut overlay, 1100);
                assert!(
                    (center[1] - 12..center[1] + 12).any(|y| (center[0] - 12..center[0] + 12)
                        .any(|x| overlay.get(x as u32, y as u32)[3] > 0))
                );
            });
        }
    }
}
