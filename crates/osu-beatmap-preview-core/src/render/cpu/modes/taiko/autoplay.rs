//! Taiko 自动击打事件与命中动画；预计算后支持并行、任意顺序取帧。

use super::animation_render::{
    gif_row_center_y, gif_row_top, object_x, AnimationLayout, PreparedTaikoHitObject,
};
use super::constants::*;
use super::notes::{cached_note_disc, RenderCache};
use crate::render::canvas::Img;
use crate::render::scene::{FrameSceneBuilder, SceneRect};

fn hit_flight_offset(age: f64, row_height: i64) -> f64 {
    let travel =
        200.0 * row_height as f64 / crate::render::cpu::modes::taiko::constants::TAIKO_BASE_HEIGHT;
    if age < 300.0 {
        -travel * (1.0 - (1.0 - age / 300.0).powi(3))
    } else {
        -travel + 3.0 * travel * ((age - 300.0) / 600.0).powi(3)
    }
}

#[derive(Clone, Copy)]
struct Hit {
    time: f64,
    source_time: f64,
    drum: bool,
    multiplier: f64,
    rim: bool,
    right: bool,
    strong: bool,
    flying: bool,
}

pub struct Autoplay {
    hits: Vec<Hit>,
    drum_sprites: [Img; 4],
}

impl Autoplay {
    pub fn new(objects: &[PreparedTaikoHitObject], layout: &AnimationLayout) -> Self {
        let mut hits = Vec::new();
        let mut right = false;
        for object in objects {
            let base = &object.hit_object;
            let strong = base.hitsound & HIT_SOUNDS_STRONG != 0;
            let mut add = |time, rim, right, strong, flying, multiplier| {
                hits.push(Hit {
                    time,
                    source_time: time,
                    drum: true,
                    multiplier,
                    rim,
                    right,
                    strong,
                    flying,
                })
            };
            if base.hit_type & SWELL_FLAG != 0 {
                let required = object.swell_required_hits.max(1);
                let spacing =
                    ((base.end_time - base.start_time).max(1) as f64 / required as f64).min(50.0);
                for index in 0..required {
                    let time = base.start_time as f64 + index as f64 * spacing;
                    if time >= base.end_time as f64 {
                        break;
                    }
                    add(
                        time,
                        index % 2 == 1,
                        index % 4 >= 2,
                        false,
                        false,
                        object.start_multiplier,
                    );
                }
            } else if base.hit_type & DRUMROLL_FLAG != 0 {
                for tick in &object.drum_roll_ticks {
                    add(
                        tick.time,
                        false,
                        right,
                        strong,
                        !object.hidden,
                        tick.multiplier,
                    );
                    right = !right;
                }
            } else {
                add(
                    base.start_time as f64,
                    base.hitsound & HIT_SOUNDS_RIM != 0,
                    right,
                    strong,
                    !object.hidden,
                    object.start_multiplier,
                );
            }
            right = !right;
        }
        hits.sort_by(|a, b| a.time.total_cmp(&b.time));
        let diameter = (layout.row_height.min(layout.left_panel_width) as f64 * 0.72)
            .round()
            .max(1.0) as u32;
        let drum_sprites = std::array::from_fn(|index| {
            let rim = index / 2 != 0;
            let right = index % 2 != 0;
            let mut sprite = Img::new(diameter, diameter, [0; 4]);
            let radius = diameter as f64 / 2.0;
            let rgb = if rim {
                RIM_NOTE_COLOR
            } else {
                CENTRE_NOTE_COLOR
            };
            for y in 0..diameter {
                for x in 0..diameter {
                    if (x as f64 + 0.5 >= radius) != right {
                        continue;
                    }
                    let distance = (x as f64 + 0.5 - radius).hypot(y as f64 + 0.5 - radius);
                    let coverage = if rim {
                        (radius - distance).clamp(0.0, 1.0)
                            * (distance - radius * 0.7).clamp(0.0, 1.0)
                    } else {
                        (radius * 0.7 - distance).clamp(0.0, 1.0)
                    };
                    sprite.put(x, y, [rgb[0], rgb[1], rgb[2], (255.0 * coverage) as u8]);
                }
            }
            sprite
        });
        Self { hits, drum_sprites }
    }

    #[cfg(feature = "replay")]
    pub fn from_replay(
        objects: &[PreparedTaikoHitObject],
        gameplay: &crate::gameplay::timeline::GameplayTimeline,
        layout: &AnimationLayout,
        classic: bool,
    ) -> Self {
        use crate::gameplay::events::{EventKind, HitResult};
        let mut result = Self::new(&[], layout);
        result
            .hits
            .extend(gameplay.taiko_inputs.iter().map(|press| Hit {
                time: press.time,
                source_time: press.time,
                drum: true,
                multiplier: 1.0,
                rim: press.key & 10 != 0,
                right: press.key & 12 != 0,
                strong: false,
                flying: false,
            }));
        for event in &gameplay.events {
            if event.kind != EventKind::Judgement
                || !matches!(
                    event.result,
                    HitResult::Great | HitResult::Ok | HitResult::SmallBonus
                )
            {
                continue;
            }
            let object = &objects[event.object_id];
            let multiplier = event
                .node_id
                .and_then(|id| object.drum_roll_ticks.get(id))
                .map_or(object.start_multiplier, |tick| tick.multiplier);
            let source_time = if classic {
                event.time
            } else {
                event.sample_time
            };
            let rim = gameplay
                .taiko_inputs
                .iter()
                .find(|p| p.time == event.time && p.object_id == Some(event.object_id))
                .map_or(object.hit_object.hitsound & HIT_SOUNDS_RIM != 0, |p| {
                    p.key & 10 != 0
                });
            result.hits.push(Hit {
                time: event.time,
                source_time,
                drum: false,
                multiplier,
                rim,
                right: false,
                strong: object.hit_object.hitsound & HIT_SOUNDS_STRONG != 0,
                flying: !object.hidden,
            });
        }
        result.hits.sort_by(|a, b| a.time.total_cmp(&b.time));
        result
    }

    pub fn draw(
        &self,
        image: &mut Img,
        layout: &AnimationLayout,
        row: i64,
        time: i64,
        cache: &mut RenderCache,
    ) {
        for (sprite, destination, alpha) in self.sprites(layout, row, time, cache) {
            image.alpha_composite_scaled(&sprite, destination.0, destination.1, alpha);
        }
    }

    pub fn draw_scene(&self, scene: &mut FrameSceneBuilder, layout: &AnimationLayout, time: i64) {
        AUTOPLAY_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            for (sprite, (x, y), alpha) in self.sprites(layout, 0, time, &mut cache) {
                let destination = SceneRect {
                    x: x as f32,
                    y: y as f32,
                    width: sprite.w as f32,
                    height: sprite.h as f32,
                };
                scene.sprite(std::sync::Arc::new(sprite), destination, alpha as f32);
            }
        });
    }

    pub(crate) fn sprites(
        &self,
        layout: &AnimationLayout,
        row: i64,
        time: i64,
        cache: &mut RenderCache,
    ) -> Vec<(Img, (i64, i64), f64)> {
        let time = time as f64;
        let start = self.hits.partition_point(|hit| hit.time <= time - 1040.0);
        let end = self.hits.partition_point(|hit| hit.time <= time);
        let center_y = gif_row_center_y(row, layout) as f64;
        let drum_x = layout.playfield_left as f64 + layout.left_panel_width as f64 / 2.0;
        let mut drum_alpha = [0.0f64; 4];
        let mut sprites = Vec::new();
        for hit in &self.hits[start..end] {
            // snapshot 已在谱面时间域；游戏默认 RateAdjustedHitAnimation=true，不能再乘 DT/HT。
            let age = time - hit.time;
            let alpha = if age < 40.0 {
                1.0 - (1.0 - age / 40.0).powi(5)
            } else {
                (1.0 - (age - 40.0) / 1000.0).clamp(0.0, 1.0).powi(5)
            };
            let index = usize::from(hit.rim) * 2;
            if hit.drum && hit.strong {
                for value in &mut drum_alpha[index..index + 2] {
                    *value = (*value + alpha).min(1.0);
                }
            } else if hit.drum {
                drum_alpha[index + usize::from(hit.right)] =
                    (drum_alpha[index + usize::from(hit.right)] + alpha).min(1.0);
            }
            if !hit.flying || age >= 800.0 {
                continue;
            }
            let offset = hit_flight_offset(age, layout.row_height);
            let scale = 1.0 - 0.2 * (1.0 - (1.0 - (age / 600.0).min(1.0)).powi(2));
            let size = if hit.strong {
                layout.big_note_diameter
            } else {
                layout.normal_note_diameter
            };
            let diameter = (size as f64 * scale).round().max(1.0) as i64;
            let color = if hit.rim {
                RIM_NOTE_COLOR
            } else {
                CENTRE_NOTE_COLOR
            };
            let sprite = cached_note_disc(
                cache,
                color,
                diameter,
                crate::render::geometry::scale_stroke_px(1.0, layout.render_scale),
                false,
            );
            // 游戏的竖直命中动画仍处于滚动容器中，水平位置继续按该物件的 SV 向左推进。
            let x = object_x(hit.source_time, time, hit.multiplier, layout) - diameter / 2;
            let y = (center_y + offset - diameter as f64 / 2.0).round() as i64;
            // GIF 各行拥有独立时间，击飞不能盖住相邻行及时间标签；MP4 可溢出轨道。
            let (top, bottom) =
                if layout.output_format == crate::render::geometry::OutputFormat::Gif {
                    (
                        gif_row_top(row, layout),
                        gif_row_top(row, layout) + layout.row_height,
                    )
                } else {
                    (0, layout.image_height)
                };
            if y >= bottom || y + diameter <= top {
                continue;
            }
            if y >= top && y + diameter <= bottom {
                sprites.push((sprite.clone(), (x, y), 1.0 - age / 800.0));
            } else {
                let clipped = sprite.crop(
                    0,
                    (top - y).max(0) as u32,
                    sprite.w,
                    (bottom - y).min(diameter) as u32,
                );
                sprites.push((clipped, (x, y.max(top)), 1.0 - age / 800.0));
            }
        }
        for (sprite, alpha) in self.drum_sprites.iter().zip(drum_alpha) {
            if alpha > 0.0 {
                sprites.push((
                    sprite.clone(),
                    (
                        (drum_x - sprite.w as f64 / 2.0).round() as i64,
                        (center_y - sprite.h as f64 / 2.0).round() as i64,
                    ),
                    alpha,
                ));
            }
        }
        sprites
    }
}

thread_local! {
    static AUTOPLAY_CACHE: std::cell::RefCell<RenderCache> =
        std::cell::RefCell::new(RenderCache::default());
}

#[cfg(test)]
mod tests {
    use super::super::animation_render::{
        build_animation_layout_with_segments_and_format, build_multiplier_points,
        prepare_hit_objects, MultiplierLookup,
    };
    use super::*;
    use crate::domain::models::TaikoHitObject;
    use crate::render::geometry::OutputFormat;

    #[test]
    fn events_follow_game_hands_strong_hits_rolls_and_swell_keys() {
        let objects = prepare_hit_objects(
            &[
                TaikoHitObject {
                    start_time: 1000,
                    end_time: 1000,
                    hit_type: 1,
                    ..Default::default()
                },
                TaikoHitObject {
                    start_time: 1200,
                    end_time: 1200,
                    hit_type: 1,
                    hitsound: HIT_SOUNDS_RIM | HIT_SOUNDS_STRONG,
                    ..Default::default()
                },
                TaikoHitObject {
                    start_time: 1500,
                    end_time: 2000,
                    hit_type: DRUMROLL_FLAG,
                    ..Default::default()
                },
                TaikoHitObject {
                    start_time: 2500,
                    end_time: 3500,
                    hit_type: SWELL_FLAG,
                    ..Default::default()
                },
            ],
            &MultiplierLookup {
                points: build_multiplier_points(&[], 1.0),
            },
            &[],
            1.0,
        );
        let layout = build_animation_layout_with_segments_and_format(1000.0, 1, OutputFormat::Gif);
        let autoplay = Autoplay::new(&objects, &layout);
        assert!(!autoplay.hits[0].right);
        assert!(autoplay.hits[1].strong && autoplay.hits[1].rim);
        let roll: Vec<_> = autoplay
            .hits
            .iter()
            .filter(|hit| (1500.0..=2000.0).contains(&hit.time))
            .collect();
        assert_eq!(roll.len(), objects[2].drum_roll_ticks.len());
        assert!(roll.windows(2).all(|pair| pair[0].right != pair[1].right));
        let swell: Vec<_> = autoplay
            .hits
            .iter()
            .filter(|hit| hit.time >= 2500.0)
            .collect();
        assert_eq!(swell.len(), objects[3].swell_required_hits as usize);
        assert_eq!(
            swell
                .iter()
                .take(4)
                .map(|hit| (hit.rim, hit.right))
                .collect::<Vec<_>>(),
            vec![(false, false), (true, false), (false, true), (true, true)]
        );
        assert!(swell.iter().all(|hit| !hit.flying));
    }

    #[test]
    fn hit_flies_up_then_falls_past_its_start_position() {
        let row_height = 400;
        let travel = 200.0 * row_height as f64 / TAIKO_BASE_HEIGHT;
        assert_eq!(hit_flight_offset(0.0, row_height), 0.0);
        assert_eq!(hit_flight_offset(300.0, row_height), -travel);
        assert!(hit_flight_offset(700.0, row_height) < 0.0);
        assert_eq!(hit_flight_offset(900.0, row_height), 2.0 * travel);
    }

    #[test]
    fn flying_notes_continue_left_at_each_objects_scroll_velocity() {
        use super::super::animation_render::judgement_line_x;

        let mut config = crate::config::CoreConfig::default();
        config.render.taiko.mp4.SCALE = 0.75;
        crate::config::with_config(std::sync::Arc::new(config), || {
            let layout = super::super::animation_render::build_video_animation_layout(
                1000.0,
                OutputFormat::Mp4,
            );
            for roll in [false, true] {
                let mut objects = prepare_hit_objects(
                    &[TaikoHitObject {
                        start_time: 1000,
                        end_time: if roll { 2000 } else { 1000 },
                        hit_type: if roll { DRUMROLL_FLAG } else { 1 },
                        ..Default::default()
                    }],
                    &MultiplierLookup {
                        points: build_multiplier_points(&[], 1.0),
                    },
                    &[],
                    1.0,
                );
                for multiplier in [0.5, 1.0, 2.0] {
                    objects[0].start_multiplier = multiplier;
                    objects[0].drum_roll_ticks =
                        vec![super::super::animation_render::PreparedAnimationPoint {
                            time: 1000.0,
                            multiplier: multiplier * 1.5,
                        }];
                    let autoplay = Autoplay::new(&objects, &layout);
                    let hit = autoplay.hits[0];
                    let expected_multiplier = if roll { multiplier * 1.5 } else { multiplier };
                    assert_eq!(hit.multiplier, expected_multiplier);
                    let mut cache = RenderCache::default();
                    let mut previous_x = judgement_line_x(&layout);
                    for age in [0, 40, 100] {
                        let sprites = autoplay.sprites(&layout, 0, 1000 + age, &mut cache);
                        let (sprite, (x, y), _) = &sprites[0];
                        let center_x = x + i64::from(sprite.w) / 2;
                        assert_eq!(
                            center_x,
                            (judgement_line_x(&layout) as f64
                                - age as f64 / layout.time_range
                                    * expected_multiplier
                                    * layout.segment_width as f64)
                                .round() as i64,
                        );
                        if age > 0 {
                            assert!(center_x < previous_x);
                            assert!(y + i64::from(sprite.h) / 2 < gif_row_center_y(0, &layout));
                        }
                        previous_x = center_x;
                    }
                }
            }
        });
    }

    #[test]
    fn pressed_drum_matches_note_colour_on_the_opaque_panel_and_in_the_scene() {
        use super::super::animation_render::draw_row_background;
        use crate::render::scene::DrawCommand;

        for format in [OutputFormat::Gif, OutputFormat::Mp4] {
            for scale in [0.75, 1.5] {
                let mut config = crate::config::CoreConfig::default();
                config.render.taiko.gif.SCALE = scale;
                config.render.taiko.mp4.SCALE = scale;
                crate::config::with_config(std::sync::Arc::new(config), || {
                    let layout = build_animation_layout_with_segments_and_format(1000.0, 1, format);
                    for rim in [false, true] {
                        for right in [false, true] {
                            for strong in [false, true] {
                                let mut autoplay = Autoplay::new(&[], &layout);
                                autoplay.hits.push(Hit {
                                    time: 1000.0,
                                    source_time: 1000.0,
                                    drum: true,
                                    multiplier: 1.0,
                                    rim,
                                    right,
                                    strong,
                                    flying: false,
                                });
                                let color = if rim {
                                    RIM_NOTE_COLOR
                                } else {
                                    CENTRE_NOTE_COLOR
                                };
                                let mut cache = RenderCache::default();
                                let note = cached_note_disc(
                                    &mut cache,
                                    color,
                                    if strong {
                                        layout.big_note_diameter
                                    } else {
                                        layout.normal_note_diameter
                                    },
                                    crate::render::geometry::scale_stroke_px(
                                        1.0,
                                        layout.render_scale,
                                    ),
                                    false,
                                );
                                let expected = [color[0], color[1], color[2], 255];
                                assert_eq!(note.get(note.w / 2, note.h / 2), expected);
                                let mut image = Img::new(
                                    layout.image_width as u32,
                                    layout.image_height as u32,
                                    [0; 4],
                                );
                                draw_row_background(&mut image, &layout, 0);
                                autoplay.draw(&mut image, &layout, 0, 1040, &mut cache);
                                let mut builder = FrameSceneBuilder::new(image.w, image.h, 1040);
                                autoplay.draw_scene(&mut builder, &layout, 1040);
                                let scene = builder.finish();
                                assert_eq!(scene.commands.len(), if strong { 2 } else { 1 });
                                for command in scene.commands.iter() {
                                    let DrawCommand::Sprite {
                                        resource,
                                        destination,
                                        alpha,
                                    } = command
                                    else {
                                        panic!("按键场景必须使用同色精灵");
                                    };
                                    assert_eq!(*alpha, 1.0);
                                    let sprite = &scene.resources[resource];
                                    let (x, y) = (0..sprite.h)
                                        .flat_map(|y| (0..sprite.w).map(move |x| (x, y)))
                                        .find(|&(x, y)| sprite.get(x, y)[3] == 255)
                                        .unwrap();
                                    assert_eq!(sprite.get(x, y), expected);
                                    assert_eq!(
                                        image.get(
                                            destination.x as u32 + x,
                                            destination.y as u32 + y
                                        ),
                                        expected
                                    );
                                }
                            }
                        }
                    }
                });
            }
        }
    }

    #[test]
    fn frames_are_seek_independent_clipped_to_gif_row_and_expire() {
        for format in [OutputFormat::Gif, OutputFormat::Mp4] {
            for scale in [0.75, 1.5] {
                let mut config = crate::config::CoreConfig::default();
                config.render.taiko.gif.SCALE = scale;
                config.render.taiko.mp4.SCALE = scale;
                crate::config::with_config(std::sync::Arc::new(config), || {
                    let layout = build_animation_layout_with_segments_and_format(1000.0, 2, format);
                    let objects = prepare_hit_objects(
                        &[TaikoHitObject {
                            start_time: 1000,
                            end_time: 1000,
                            hit_type: 1,
                            ..Default::default()
                        }],
                        &MultiplierLookup {
                            points: build_multiplier_points(&[], 1.0),
                        },
                        &[],
                        1.0,
                    );
                    let autoplay = Autoplay::new(&objects, &layout);
                    let mut cache = RenderCache::default();
                    let mut draw = |time| {
                        let mut image = Img::new(
                            layout.image_width as u32,
                            layout.image_height as u32,
                            [0; 4],
                        );
                        autoplay.draw(&mut image, &layout, 0, time, &mut cache);
                        image
                    };
                    let first = draw(1100);
                    assert!(first.data.chunks_exact(4).any(|pixel| pixel[3] != 0));
                    for time in [999, 2040] {
                        assert!(draw(time).data.iter().all(|value| *value == 0));
                    }
                    assert_eq!(first.data, draw(1100).data);
                    if format == OutputFormat::Gif {
                        let top = gif_row_top(0, &layout) as usize;
                        let bottom = top + layout.row_height as usize;
                        let stride = first.w as usize * 4;
                        assert!(first.data[..top * stride].iter().all(|value| *value == 0));
                        assert!(first.data[bottom * stride..]
                            .iter()
                            .all(|value| *value == 0));
                    }
                });
            }
        }
    }
}
