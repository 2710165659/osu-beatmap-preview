//! Taiko 回放共享绘制计划；GIF、CPU 视频与 WebGPU 复用真实输入和判定。
use super::feedback;
use crate::render::shared::replay::{
    feedback::FeedbackVisuals,
    font::ArgonFont,
    hud::{self, HudLayout, HudVisuals},
    overlay::OverlayCanvas,
};
use crate::render::{
    canvas::Img,
    cpu::modes::taiko::{
        animation_render::*, autoplay::Autoplay, constants::*, notes::RenderCache, timing::*,
    },
    geometry::{taiko_playfield_top, OutputFormat, PixelRect},
    scene::FrameSceneBuilder,
    visibility::VisibilityTimeline,
};
use crate::{
    gameplay::timeline::GameplayTimeline,
    replay::{
        compatibility::ReplayPolicy,
        preparation::taiko::{PreparedTaiko, TaikoObjectKind},
    },
    Result,
};
use std::{cell::RefCell, sync::Arc};

thread_local! { static CACHE: RefCell<RenderCache> = RefCell::new(RenderCache::default()); }

impl<'a> From<&'a crate::gameplay::state::ObjectState>
    for TaikoReplayDrawState<'a, crate::gameplay::events::HitResult>
{
    fn from(state: &'a crate::gameplay::state::ObjectState) -> Self {
        Self {
            head: state.head,
            end: state.end,
            nodes: &state.nodes,
        }
    }
}

pub struct ReplayRenderPlan {
    pub layout: AnimationLayout,
    prepared: Arc<PreparedTaiko>,
    gameplay: Arc<GameplayTimeline>,
    objects: Vec<PreparedTaikoHitObject>,
    lines: Vec<PreparedAnimationPoint>,
    track: Arc<Img>,
    autoplay: Autoplay,
    flashlight: Option<VisibilityTimeline>,
    hud: HudVisuals,
    feedback: FeedbackVisuals,
}

impl ReplayRenderPlan {
    pub fn new(
        prepared: Arc<PreparedTaiko>,
        gameplay: Arc<GameplayTimeline>,
        font: Option<ArgonFont>,
        dimensions: Option<(u32, u32)>,
    ) -> Result<Self> {
        Self::with_format(prepared, gameplay, font, dimensions, OutputFormat::Mp4)
    }

    pub fn with_format(
        prepared: Arc<PreparedTaiko>,
        gameplay: Arc<GameplayTimeline>,
        font: Option<ArgonFont>,
        dimensions: Option<(u32, u32)>,
        format: OutputFormat,
    ) -> Result<Self> {
        let beatmap = &prepared.beatmap;
        let mods = &prepared.mods.settings;
        let points = effective_timing_points(beatmap, Some(mods));
        let lookup = MultiplierLookup {
            points: build_multiplier_points(
                &points,
                effective_slider_multiplier(beatmap, Some(mods))?,
            ),
        };
        let sources: Vec<_> = prepared.objects.iter().map(|o| o.source.clone()).collect();
        let mut objects = prepare_hit_objects_with_mods(
            &sources,
            &lookup,
            &points,
            beatmap.difficulty.get_f64_or("SliderTickRate", 1.0),
            prepared.od,
            Some(mods),
        );
        for (object, logical) in objects.iter_mut().zip(&prepared.objects) {
            if let TaikoObjectKind::Swell { required } = logical.kind {
                object.swell_required_hits = required;
            }
        }
        let config = crate::config::current();
        let show_measure_lines = match format {
            OutputFormat::Gif => config.render.taiko.gif.style.SHOW_MEASURE_LINES,
            _ => config.render.taiko.mp4.style.SHOW_MEASURE_LINES,
        };
        let lines = prepare_measure_lines(&sources, &points, &lookup, show_measure_lines);
        let mut layout = build_video_animation_layout(compute_time_range(), format);
        if let Some((width, height)) = dimensions {
            let dx = (i64::from(width) - layout.image_width) / 2;
            let dy = taiko_playfield_top(height) - layout.content.y;
            layout.playfield_left += dx;
            layout.first_row_top += dy;
            layout.content.x += dx;
            layout.content.y += dy;
            layout.image_width = width.into();
            layout.image_height = height.into();
        }
        // 静态轨道只上传内容带；不把整帧 CPU 位图作为实时渲染结果。
        let mut track_layout = layout.clone();
        track_layout.first_row_top = 0;
        let mut track = Img::new(layout.image_width as u32, layout.row_height as u32, [0; 4]);
        draw_row_background(&mut track, &track_layout, 0);
        let classic = matches!(gameplay.policy, ReplayPolicy::Taiko(p) if p.classic);
        let autoplay = Autoplay::from_replay(&objects, &gameplay, &layout, classic);
        let flashlight = mods
            .flashlight
            .then(|| VisibilityTimeline::from_replay(&gameplay, &beatmap.break_periods));
        let windows = crate::domain::rulesets::taiko::gameplay::hit_windows(prepared.od);
        let feedback =
            FeedbackVisuals::new_taiko(layout.row_height as f64 / TAIKO_BASE_HEIGHT, font.as_ref());
        let hud = HudVisuals::new(&gameplay, font, layout.render_scale, windows);
        Ok(Self {
            layout,
            prepared,
            gameplay,
            objects,
            lines,
            track: Arc::new(track),
            autoplay,
            flashlight,
            hud,
            feedback,
        })
    }

    pub fn draw_frame(&self, image: &mut Img, time: i64) {
        self.draw_to(image, time);
    }
    pub fn draw_scene(&self, scene: &mut FrameSceneBuilder, time: i64) {
        self.draw_to(scene, time);
    }
    pub fn draw_overlay(&self, image: &mut Img, time: i64) {
        self.overlay_to(image, time);
    }
    pub fn draw_scene_overlay(&self, scene: &mut FrameSceneBuilder, time: i64) {
        self.overlay_to(scene, time);
    }

    fn overlay_to(&self, target: &mut impl OverlayCanvas, time: i64) {
        for event in self
            .gameplay
            .events_between(time as f64 - 1000.0, time as f64)
        {
            if event.kind != crate::gameplay::events::EventKind::Judgement
                || event.node_id.is_some()
                || event.object_id >= self.objects.len()
            {
                continue;
            }
            // Taiko 判定固定在打击目标，不随早晚击误差沿轨道偏移。
            let position = [
                judgement_line_x(&self.layout) as f64,
                gif_row_center_y(0, &self.layout) as f64,
            ];
            feedback::draw_judgement(
                target,
                &self.feedback,
                event.result,
                event.id,
                position,
                self.layout.row_height as f64 / TAIKO_BASE_HEIGHT,
                self.layout.row_height,
                time as f64 - event.time,
            );
        }
        if let Some(timeline) = &self.flashlight {
            target.flashlight(
                &taiko_flashlight(timeline, time, &self.layout, 0),
                PixelRect {
                    x: 0,
                    y: 0,
                    width: self.layout.image_width,
                    height: self.layout.image_height,
                },
            );
        }
        hud::draw(
            target,
            HudLayout {
                width: self.layout.image_width,
                height: self.layout.image_height,
                scale: self.layout.render_scale,
                time_axis: crate::processing::timeline::TimeAxis::new(
                    self.prepared.objects[0].source.start_time,
                ),
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

    fn draw_to(&self, target: &mut impl OverlayCanvas, time: i64) {
        let layout = &self.layout;
        let center_y = gif_row_center_y(0, layout);
        let left = judgement_line_x(layout);
        let right = layout.playfield_left + layout.left_panel_width + layout.right_panel_width;
        target.shared_sprite(&self.track, 0, gif_row_top(0, layout));
        for line in &self.lines {
            let alpha =
                crate::render::cpu::modes::taiko::animation::measure_line_alpha(line.time, time);
            let x = object_x(line.time, time as f64, line.multiplier, layout);
            if alpha > 0.0 && (left..right).contains(&x) {
                let mut color = ANIMATION_MEASURE_LINE_COLOR;
                color[3] = (f64::from(color[3]) * alpha).round() as u8;
                let h = (layout.row_height as f64 * MEASURE_LINE_HEIGHT_RATIO).round() as i64;
                target.fill_rect_size(
                    x,
                    center_y - h / 2,
                    crate::render::geometry::scale_stroke_px(
                        MEASURE_LINE_WIDTH as f64,
                        layout.render_scale,
                    ),
                    h,
                    color,
                );
            }
        }
        CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            for (id, object) in self.objects.iter().enumerate().rev() {
                let state = TaikoReplayDrawState::from(&self.gameplay.objects[id]);
                draw_hit_object_state(target, object, layout, 0, time, &mut cache, Some(&state));
            }
            for (sprite, (x, y), alpha) in self.autoplay.sprites(layout, 0, time, &mut cache) {
                target.sprite_opacity(&Arc::new(sprite), x, y, alpha);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::scene::{DrawCommand, FrameSceneBuilder};
    use crate::replay::{ReplayDocument, ReplayFrame, ReplayHeader};

    #[test]
    fn replay_roll_tail_is_round_clipped_and_shared_by_cpu_and_scene() {
        for scale in [0.75, 1.0, 1.5] {
            let mut config = crate::config::CoreConfig::default();
            config.render.taiko.mp4.SCALE = scale;
            crate::config::with_config(Arc::new(config), || {
                for strong in [false, true] {
                    let beatmap = crate::parse_beatmap_bytes(format!(
                        "osu file format v14\n[General]\nMode:1\n[Difficulty]\nOverallDifficulty:5\nHPDrainRate:5\nSliderMultiplier:1\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n0,0,1000,2,{},L|800:0,1,800\n",
                        if strong { HIT_SOUNDS_STRONG } else { 0 },
                    ).as_bytes()).unwrap();
                    let document = Arc::new(ReplayDocument {
                        header: ReplayHeader {
                            ruleset_id: 1,
                            version: 20200101,
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
                        frames: [0, 10000]
                            .into_iter()
                            .enumerate()
                            .map(|(sequence, time_ms)| ReplayFrame {
                                delta_ms: time_ms,
                                time_ms,
                                cursor: [0.0; 2],
                                keys: 0,
                                sequence,
                            })
                            .collect(),
                        seed: None,
                        diagnostics: Vec::new(),
                    });
                    let prepared = Arc::new(
                        crate::replay::preparation::taiko::prepare_replay(&beatmap, &document)
                            .unwrap(),
                    );
                    let gameplay = Arc::new(
                        crate::gameplay::engine::compile_taiko(&prepared, document).unwrap(),
                    );
                    let plan = ReplayRenderPlan::new(prepared, gameplay, None, None).unwrap();
                    let layout = &plan.layout;
                    let object = &plan.objects[0];
                    let left = judgement_line_x(layout);
                    let right =
                        layout.playfield_left + layout.left_panel_width + layout.right_panel_width;
                    let center_y = gif_row_center_y(0, layout);
                    let diameter = if strong {
                        layout.big_note_diameter
                    } else {
                        layout.normal_note_diameter
                    };
                    let tail_height =
                        (diameter as f64 * SPAN_BODY_HEIGHT_RATIO).round().max(1.0) as i64;
                    let tail = crate::render::cpu::modes::taiko::notes::build_roll_tail_sprite(
                        ROLL_COLOR,
                        tail_height,
                    );
                    let speed =
                        object.end_multiplier * layout.segment_width as f64 / layout.time_range;
                    let clipped_time = (object.hit_object.end_time as f64
                        - (right - left - i64::from(tail.w) / 2) as f64 / speed)
                        .round() as i64;
                    for time in [3500, clipped_time, 3500] {
                        let end_x = object_x(
                            object.hit_object.end_time as f64,
                            time as f64,
                            object.end_multiplier,
                            layout,
                        );
                        assert!(end_x > left + layout.big_note_diameter);
                        let visible_width = (right - end_x).min(i64::from(tail.w));
                        assert!(visible_width > 0);
                        let mut image = Img::new(
                            layout.image_width as u32,
                            layout.image_height as u32,
                            [0; 4],
                        );
                        plan.draw_frame(&mut image, time);
                        let row = tail.h / 4;
                        let y = center_y - i64::from(tail.h) / 2 + i64::from(row);
                        let mut builder = FrameSceneBuilder::new(image.w, image.h, time);
                        plan.draw_scene(&mut builder, time);
                        let scene = builder.finish();
                        let (resource, destination, alpha) = scene
                            .commands
                            .iter()
                            .find_map(|command| match command {
                                DrawCommand::Sprite {
                                    resource,
                                    destination,
                                    alpha,
                                } if destination.x <= end_x as f32
                                    && destination.x + destination.width
                                        >= (end_x + visible_width) as f32
                                    && destination.y <= y as f32
                                    && destination.y + destination.height > y as f32
                                    && scene.resources[resource].h < layout.row_height as u32 =>
                                {
                                    Some((resource, destination, alpha))
                                }
                                _ => None,
                            })
                            .expect("WebGPU 必须绘制滚奏圆尾");
                        assert_eq!(*alpha, 1.0);
                        let source = &scene.resources[resource];
                        let mut expected = Img::new(image.w, image.h, [0; 4]);
                        expected.alpha_composite(&plan.track, 0, gif_row_top(0, layout));
                        expected.alpha_composite_scaled(
                            source,
                            destination.x as i64,
                            destination.y as i64,
                            f64::from(*alpha),
                        );
                        for x in 0..visible_width as u32 {
                            assert_eq!(
                                image.get((end_x + i64::from(x)) as u32, y as u32),
                                expected.get((end_x + i64::from(x)) as u32, y as u32),
                                "CPU/WGPU 圆尾像素：{scale}/{strong}/{time}/{x}"
                            );
                        }
                        if time == clipped_time {
                            assert!(visible_width < i64::from(tail.w), "覆盖轨道右边界裁剪");
                        }
                    }
                }
            });
        }
    }

    #[test]
    fn replay_judgement_is_drawn_at_the_taiko_target_on_cpu_and_scene() {
        use crate::gameplay::events::{EventKind, HitResult};
        for (hit_time, keys, expected, label_index, dimensions) in [
            (980, 1, HitResult::Great, 0, None),
            (1040, 1, HitResult::Ok, 1, Some((1280, 720))),
            (1000, 0, HitResult::Miss, 3, Some((1280, 960))),
        ] {
            let beatmap = crate::parse_beatmap_bytes(
            b"osu file format v14\n[General]\nMode:1\n[Difficulty]\nOverallDifficulty:5\nHPDrainRate:5\nSliderMultiplier:1\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n0,0,1000,1,0\n",
        )
        .unwrap();
            let document = Arc::new(ReplayDocument {
                header: ReplayHeader {
                    ruleset_id: 1,
                    version: 20200101,
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
                frames: [(0, 0), (hit_time, keys), (1200, 0)]
                    .into_iter()
                    .enumerate()
                    .map(|(sequence, (time_ms, keys))| ReplayFrame {
                        delta_ms: 0,
                        time_ms,
                        cursor: [0.0; 2],
                        keys,
                        sequence,
                    })
                    .collect(),
                seed: None,
                diagnostics: Vec::new(),
            });
            let prepared = Arc::new(
                crate::replay::preparation::taiko::prepare_replay(&beatmap, &document).unwrap(),
            );
            let gameplay =
                Arc::new(crate::gameplay::engine::compile_taiko(&prepared, document).unwrap());
            let event = gameplay
                .events
                .iter()
                .find(|event| event.kind == EventKind::Judgement && event.node_id.is_none())
                .unwrap();
            assert_eq!(event.result, expected);
            let age = if expected == HitResult::Miss {
                400
            } else {
                100
            };
            let time = event.time.ceil() as i64 + age;
            let plan = ReplayRenderPlan::new(prepared, gameplay, None, dimensions).unwrap();
            let expected_top = plan.layout.image_height as f64 * 135.0 / 480.0;
            assert!((plan.layout.first_row_top as f64 - expected_top).abs() <= 0.5);
            assert_eq!(plan.layout.content.y, plan.layout.first_row_top);
            if let Some((width, height)) = dimensions {
                assert_eq!(plan.layout.image_width, i64::from(width));
                assert_eq!(plan.layout.image_height, i64::from(height));
            }
            let position = [
                judgement_line_x(&plan.layout) as f32,
                gif_row_center_y(0, &plan.layout) as f32,
            ];
            let mut scene = FrameSceneBuilder::new(
                plan.layout.image_width as u32,
                plan.layout.image_height as u32,
                time,
            );
            plan.draw_scene_overlay(&mut scene, time);
            let scene = scene.finish();
            if expected == HitResult::Great {
                assert!(
                    scene.resources.values().all(|resource| {
                        !Arc::ptr_eq(resource, &plan.feedback.labels[0])
                            && plan.feedback.rings[0]
                                .iter()
                                .all(|ring| !Arc::ptr_eq(resource, ring))
                    }),
                    "Argon PRO 的 Great 不显示文字或判定粒子"
                );
                continue;
            }
            let label = &plan.feedback.labels[label_index];
            let (text_position, text_size) = scene
                .commands
                .iter()
                .find_map(|command| {
                    if let DrawCommand::TransformedSprite {
                        resource,
                        position,
                        size,
                        ..
                    } = command
                    {
                        Arc::ptr_eq(&scene.resources[resource], label).then_some((*position, *size))
                    } else {
                        None
                    }
                })
                .expect("WGPU 必须包含实际回放判定标签");
            assert_eq!(text_position[0], position[0], "早晚击判定不得沿轨道偏移");
            if expected == HitResult::Miss {
                assert!(text_position[1] > position[1], "Miss 向下坠落");
            } else {
                assert!(
                    text_position[1] < position[1] - plan.layout.row_height as f32 * 0.6,
                    "命中标签位于判定目标上方"
                );
            }
            let mut image = Img::new(
                plan.layout.image_width as u32,
                plan.layout.image_height as u32,
                [0; 4],
            );
            plan.draw_overlay(&mut image, time);
            let left = (text_position[0] - text_size[0] / 2.0 - 2.0).max(0.0) as u32;
            let right = (text_position[0] + text_size[0] / 2.0 + 2.0).min(image.w as f32) as u32;
            let top = (text_position[1] - text_size[1] / 2.0 - 2.0).max(0.0) as u32;
            let bottom = (text_position[1] + text_size[1] / 2.0 + 2.0).min(image.h as f32) as u32;
            assert!((top..bottom).any(|y| (left..right).any(|x| image.get(x, y)[3] > 0)));
        }
    }
}
