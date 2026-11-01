//! 自动预览与回放接取后的堆叠与清盘时间轴；按绝对时间查询，供 CPU 与 WGPU 共用。

use std::{collections::HashMap, sync::Arc};

use super::animation::AnimationLayout;
use super::catcher::CatcherPath;
use super::constants::*;
use super::drawing::{draw_catch_object_clipped, draw_catch_object_scene, object_diameter};
use super::objects::{circle_scale, ObjType, RenderObject};
use crate::domain::shared::legacy_random::LegacyRandom;
use crate::render::canvas::Img;
use crate::render::scene::{FrameSceneBuilder, SceneRect};

const RELEASE_DURATION: f64 = 750.0;

#[derive(Clone)]
struct CaughtNote {
    object: RenderObject,
    time: f64,
    offset: [f64; 2],
}

struct Stack {
    since: f64,
    notes: Vec<CaughtNote>,
}

struct Release {
    time: f64,
    center: f64,
    explode: bool,
    notes: Vec<CaughtNote>,
}

/// 每行覆盖最大避让距离，只需检查相邻三行；不改变距离判定和随机数消耗。
struct StackOffsets {
    row_height: f64,
    rows: HashMap<i64, Vec<[f64; 2]>>,
}

impl StackOffsets {
    fn overlaps(&self, notes: &[CaughtNote], offset: [f64; 2], separation: f64) -> bool {
        let overlaps =
            |other: [f64; 2]| (other[0] - offset[0]).hypot(other[1] - offset[1]) < separation;
        // 极端难度可能产生零或非有限尺寸，沿用原始扫描的浮点行为。
        if self.row_height <= 0.0 || !self.row_height.is_finite() {
            return notes.iter().any(|note| overlaps(note.offset));
        }
        let row = (offset[1] / self.row_height).floor() as i64;
        (row.saturating_sub(1)..=row.saturating_add(1)).any(|row| {
            self.rows
                .get(&row)
                .is_some_and(|offsets| offsets.iter().copied().any(overlaps))
        })
    }

    fn insert(&mut self, offset: [f64; 2]) {
        let row = (offset[1] / self.row_height).floor() as i64;
        self.rows.entry(row).or_default().push(offset);
    }
}

pub struct CatcherDisplay {
    path: Arc<CatcherPath>,
    half_catch_width: f64,
    stacks: Vec<Stack>,
    releases: Vec<Release>,
}

impl CatcherDisplay {
    pub fn new(objects: &[RenderObject], path: Arc<CatcherPath>, circle_size: f64) -> Self {
        let object_scale = circle_scale(circle_size).abs();
        let mut display = Self {
            path,
            half_catch_width: CATCHER_BASE_SIZE * object_scale * ALLOWED_CATCH_RANGE,
            stacks: vec![Stack {
                since: f64::NEG_INFINITY,
                notes: Vec::new(),
            }],
            releases: Vec::new(),
        };
        let mut ordered: Vec<_> = objects
            .iter()
            .filter(|object| {
                object.event_time_or_start().is_finite()
                    && display.path.active_at(object.event_time_or_start())
            })
            .collect();
        ordered.sort_by(|a, b| a.event_time_or_start().total_cmp(&b.event_time_or_start()));
        let mut offsets = StackOffsets {
            row_height: ordered
                .iter()
                .map(|object| {
                    object_diameter(object_scale, 1.0, object.scale_factor) * 10.0 / OBJECT_RADIUS
                })
                .fold(0.0, f64::max),
            rows: HashMap::new(),
        };
        // 仅用于视觉避让，不能消耗谱面展开所用的随机序列。
        let mut rng = LegacyRandom::new(RNG_SEED as u32);
        for object in ordered {
            let time = object.event_time_or_start();
            let center = display.path.position_at_f64(time);
            let caught = display.catches(object);
            let stack = display.stacks.last_mut().unwrap();
            if caught {
                let diameter = object_diameter(object_scale, 1.0, object.scale_factor);
                let separation = diameter * 10.0 / OBJECT_RADIUS;
                let mut offset = [object_x(object) - center, 0.0];
                // 与游戏一样随机向上避让；次数上限为畸形密集谱面提供确定的回退。
                let mut attempts = 0;
                while separation > 0.0 && offsets.overlaps(&stack.notes, offset, separation) {
                    if attempts < 1024 {
                        offset[0] += (rng.next_double() * 2.0 - 1.0) * separation;
                        offset[1] -= rng.next_double() * 5.0;
                    } else {
                        offset[1] -= separation;
                    }
                    attempts += 1;
                }
                let note = CaughtNote {
                    object: object.clone(),
                    time,
                    offset,
                };
                if matches!(object.object_type, ObjType::Droplet | ObjType::TinyDroplet) {
                    display.releases.push(Release {
                        time,
                        center,
                        explode: true,
                        notes: vec![note],
                    });
                } else {
                    offsets.insert(offset);
                    stack.notes.push(note);
                }
            }
            if object.last_in_combo {
                if !stack.notes.is_empty() {
                    display.releases.push(Release {
                        time,
                        center,
                        explode: caught || object.object_type == ObjType::Banana,
                        notes: stack.notes.clone(),
                    });
                }
                display.stacks.push(Stack {
                    since: time,
                    notes: Vec::new(),
                });
                offsets.rows.clear();
            }
        }
        display
    }

    fn catches(&self, object: &RenderObject) -> bool {
        self.path.active_at(object.event_time_or_start())
            && (object_x(object) - self.path.position_at_f64(object.event_time_or_start())).abs()
                <= self.half_catch_width
    }

    /// 被接住的下落物件由盘上或散出副本接替，避免判定线处重复绘制。
    pub fn has_caught(&self, object: &RenderObject, time: i64) -> bool {
        object.event_time_or_start() <= time as f64 && self.catches(object)
    }

    fn notes_at(
        &self,
        time: i64,
        layout: &AnimationLayout,
        mut draw: impl FnMut(&RenderObject, [f64; 2], f64, u8, Option<i64>),
    ) {
        let time = time as f64;
        let scale = layout.playfield_scale;
        let origin = [
            layout.playfield_left,
            layout.playfield_top + STABLE_CATCHER_Y * scale,
        ];
        let mut emit =
            |note: &CaughtNote, center: f64, displacement: [f64; 2], alpha, clip_bottom| {
                let diameter =
                    object_diameter(layout.object_scale.abs(), scale, note.object.scale_factor)
                        * 0.5;
                let position = [
                    origin[0] + (center + note.offset[0] + displacement[0]) * scale,
                    // 游戏的 CaughtObject 原点在中心；容器只上移 5 个接盘局部像素。
                    origin[1]
                        + (note.offset[1] + displacement[1] - 10.0 * layout.object_scale.abs())
                            * scale,
                ];
                draw(&note.object, position, diameter, alpha, clip_bottom);
            };
        let stack_index = self.stacks.partition_point(|stack| stack.since <= time) - 1;
        let stack = &self.stacks[stack_index];
        let count = stack.notes.partition_point(|note| note.time <= time);
        let center = self.path.position_at_f64(time);
        // 游戏的接盘主体覆盖盘上果堆的下半部分；薄盘样式用判定线裁剪模拟该遮挡。
        let clip_bottom = origin[1].round() as i64;
        for note in &stack.notes[..count] {
            emit(note, center, [0.0; 2], 255, Some(clip_bottom));
        }
        let lo = self
            .releases
            .partition_point(|release| release.time <= time - RELEASE_DURATION);
        let hi = self
            .releases
            .partition_point(|release| release.time <= time);
        for release in &self.releases[lo..hi] {
            let age = time - release.time;
            let alpha = ((1.0 - age / RELEASE_DURATION) * 255.0).round() as u8;
            let vertical = if release.explode {
                if age < 250.0 {
                    -50.0 * (age / 250.0 * std::f64::consts::FRAC_PI_2).sin()
                } else {
                    -50.0
                        + 100.0
                            * (1.0 - ((age - 250.0) / 500.0 * std::f64::consts::FRAC_PI_2).cos())
                }
            } else {
                75.0 * (1.0 - (age / RELEASE_DURATION * std::f64::consts::FRAC_PI_2).cos())
            };
            for note in &release.notes {
                let horizontal = if release.explode {
                    note.offset[0] * 6.0 * age / 1000.0
                } else {
                    0.0
                };
                emit(note, release.center, [horizontal, vertical], alpha, None);
            }
        }
    }

    fn visible_notes_at(
        &self,
        time: i64,
        layout: &AnimationLayout,
        mut draw: impl FnMut(&RenderObject, [f64; 2], f64, u8, Option<i64>),
    ) {
        self.notes_at(
            time,
            layout,
            |object, position, diameter, alpha, clip_bottom| {
                // 长 Combo 的果堆可能超出画布；保留 hyperdash 外环和抗锯齿边缘。
                let radius = diameter.abs() / 2.0 * if object.hyper_dash { 1.6 } else { 1.0 } + 2.0;
                if position[0] + radius < 0.0
                    || position[0] - radius > layout.frame_width as f64
                    || position[1] + radius < 0.0
                    || position[1] - radius > layout.frame_height as f64
                {
                    return;
                }
                draw(object, position, diameter, alpha, clip_bottom);
            },
        );
    }

    pub fn draw_notes_cpu(&self, frame: &mut Img, time: i64, layout: &AnimationLayout) {
        self.visible_notes_at(
            time,
            layout,
            |object, position, diameter, alpha, clip_bottom| {
                draw_catch_object_clipped(frame, object, position, diameter, alpha, clip_bottom);
            },
        );
    }

    pub fn draw_notes_scene(
        &self,
        scene: &mut FrameSceneBuilder,
        time: i64,
        layout: &AnimationLayout,
    ) {
        let mut clipped = false;
        self.visible_notes_at(
            time,
            layout,
            |object, position, diameter, alpha, clip_bottom| {
                if let Some(bottom) = clip_bottom {
                    if !clipped {
                        scene.push_clip(SceneRect {
                            x: 0.0,
                            y: 0.0,
                            width: layout.frame_width as f32,
                            height: bottom.clamp(0, layout.frame_height) as f32,
                        });
                        clipped = true;
                    }
                } else if clipped {
                    scene.pop_clip();
                    clipped = false;
                }
                draw_catch_object_scene(scene, object, position, diameter, alpha);
            },
        );
        if clipped {
            scene.pop_clip();
        }
    }

    pub fn draw_catcher_cpu(&self, frame: &mut Img, time: i64, layout: &AnimationLayout) {
        self.path.draw_cpu(frame, time, layout);
    }

    pub fn draw_catcher_scene(
        &self,
        scene: &mut FrameSceneBuilder,
        time: i64,
        layout: &AnimationLayout,
    ) {
        self.path.draw_scene(scene, time, layout);
    }
}

fn object_x(object: &RenderObject) -> f64 {
    if object.x.is_finite() {
        object.x.clamp(0.0, PLAYFIELD_WIDTH)
    } else {
        PLAYFIELD_WIDTH / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::super::animation::build_animation_layout;
    use super::super::catcher::build_catcher_path;
    use super::super::objects::build_fruit_object;
    use super::*;
    use crate::render::geometry::OutputFormat;

    fn fruit(x: f64, time: i64) -> RenderObject {
        build_fruit_object(x, time, [40, 150, 220], None)
    }

    fn build_display(objects: &[RenderObject]) -> CatcherDisplay {
        CatcherDisplay::new(objects, Arc::new(build_catcher_path(objects, 5.0)), 5.0)
    }

    fn snapshot(display: &CatcherDisplay, time: i64, scale: f64) -> Vec<([f64; 2], f64, u8)> {
        let mut layout = build_animation_layout(5.0, 5.0, OutputFormat::Gif);
        layout.playfield_scale *= scale;
        layout.playfield_left *= scale;
        layout.playfield_top *= scale;
        let mut notes = Vec::new();
        display.notes_at(time, &layout, |_, position, diameter, alpha, _| {
            notes.push((position, diameter, alpha));
        });
        notes
    }

    #[test]
    fn caught_fruit_stacks_moves_and_uses_half_size_without_frame_history() {
        let objects = [fruit(256.0, 1000), fruit(256.0, 1100), fruit(400.0, 3000)];
        let display = build_display(&objects);
        let before = snapshot(&display, 999, 1.0);
        assert!(before.is_empty());
        let stacked = snapshot(&display, 1100, 1.0);
        assert_eq!(stacked.len(), 2);
        assert_ne!(stacked[0].0, stacked[1].0);
        assert!(stacked[1].0[1] < stacked[0].0[1]);
        let layout = build_animation_layout(5.0, 5.0, OutputFormat::Gif);
        assert_eq!(
            stacked[0].1,
            object_diameter(layout.object_scale, layout.playfield_scale, 1.0) * 0.5
        );
        let judgement_y = layout.playfield_top + STABLE_CATCHER_Y * layout.playfield_scale;
        let center_y = judgement_y - 10.0 * layout.object_scale.abs() * layout.playfield_scale;
        assert!((stacked[0].0[1] - center_y).abs() < 1e-9);
        assert!(stacked[0].0[1] + stacked[0].1 / 2.0 > judgement_y);
        let later = snapshot(&display, 2900, 1.0);
        assert!(later[0].0[0] > stacked[0].0[0]);
        assert_eq!(stacked, snapshot(&display, 1100, 1.0));
        assert_eq!(stacked, snapshot(&build_display(&objects), 1100, 1.0));
        for scale in [0.75, 1.25] {
            let scaled = snapshot(&display, 1100, scale);
            for ((position, diameter, alpha), (scaled_position, scaled_diameter, scaled_alpha)) in
                stacked.iter().zip(scaled)
            {
                assert!((position[0] * scale - scaled_position[0]).abs() < 1e-9);
                assert!((position[1] * scale - scaled_position[1]).abs() < 1e-9);
                assert!((diameter * scale - scaled_diameter).abs() < 1e-9);
                assert_eq!(*alpha, scaled_alpha);
            }
        }
    }

    #[test]
    fn droplets_scatter_immediately_and_only_caught_objects_are_displayed() {
        for kind in [ObjType::Droplet, ObjType::TinyDroplet] {
            let mut droplet = fruit(256.0, 1200);
            droplet.object_type = kind;
            droplet.scale_factor = if kind == ObjType::Droplet {
                DROPLET_SCALE
            } else {
                TINY_DROPLET_SCALE
            };
            let objects = [fruit(256.0, 1000), droplet];
            let display = build_display(&objects);
            assert_eq!(display.stacks[0].notes.len(), 1);
            assert_eq!(snapshot(&display, 1200, 1.0).len(), 2);
            assert_eq!(snapshot(&display, 1950, 1.0).len(), 1);
        }
        let path = Arc::new(build_catcher_path(&[fruit(256.0, 1000)], 5.0));
        let mut edge = fruit(256.0 + CATCHER_BASE_SIZE * 0.5 * ALLOWED_CATCH_RANGE, 1000);
        let at_edge = CatcherDisplay::new(std::slice::from_ref(&edge), path.clone(), 5.0);
        assert_eq!(snapshot(&at_edge, 1000, 1.0).len(), 1);
        edge.x += 0.001;
        let missed = CatcherDisplay::new(std::slice::from_ref(&edge), path, 5.0);
        assert!(snapshot(&missed, 1000, 1.0).is_empty());
        assert!(!missed.has_caught(&edge, 1000));
    }

    #[test]
    fn combo_end_adds_the_last_fruit_before_exploding_and_releases_in_world_space() {
        let mut last = fruit(260.0, 1100);
        last.last_in_combo = true;
        let display = build_display(&[fruit(256.0, 1000), last, fruit(512.0, 1200)]);
        assert_eq!(display.releases[0].notes.len(), 2);
        assert!(display.releases[0].explode);
        let start = snapshot(&display, 1100, 1.0);
        let top = snapshot(&display, 1350, 1.0);
        assert_eq!(start.len(), 2);
        // 新 Combo 的水果在盘上，旧果堆的中心不会随接盘移到 512。
        assert_eq!(top.len(), 3);
        assert!(top[1].0[0] < top[0].0[0]);
        assert!(top[1].0[1] < start[0].0[1]);
        assert!(top[1].2 < 255);
        assert_eq!(snapshot(&display, 1850, 1.0).len(), 1);
        let layout = build_animation_layout(5.0, 5.0, OutputFormat::Gif);
        let mut scene =
            FrameSceneBuilder::new(layout.frame_width as u32, layout.frame_height as u32, 1350);
        display.draw_notes_scene(&mut scene, 1350, &layout);
        let scene = scene.finish();
        let mut clipped = false;
        let mut released = 0;
        for command in scene.commands.iter() {
            match command {
                crate::render::scene::DrawCommand::PushClip(_) => clipped = true,
                crate::render::scene::DrawCommand::PopClip => clipped = false,
                crate::render::scene::DrawCommand::Circle { color, .. } if color[3] < 255 => {
                    assert!(!clipped);
                    released += 1;
                }
                _ => (),
            }
        }
        assert_eq!(released, 2);
        assert!(!clipped);
    }

    #[test]
    fn missed_combo_tail_drops_but_missed_last_banana_explodes() {
        for kind in [ObjType::Fruit, ObjType::Banana] {
            let mut last = fruit(0.0, 1200);
            last.object_type = kind;
            last.last_in_combo = true;
            let path = Arc::new(build_catcher_path(&[fruit(256.0, 1000)], 5.0));
            let display = CatcherDisplay::new(&[fruit(256.0, 1000), last], path, 5.0);
            assert_eq!(display.stacks.last().unwrap().notes.len(), 0);
            assert_eq!(display.releases[0].explode, kind == ObjType::Banana);
            let before = snapshot(&display, 1199, 1.0);
            let after = snapshot(&display, 1450, 1.0);
            if kind == ObjType::Fruit {
                assert!(after[0].0[1] > before[0].0[1]);
            } else {
                assert!(after[0].0[1] < before[0].0[1]);
            }
            assert!(snapshot(&display, 1950, 1.0).is_empty());
        }
    }

    #[test]
    fn indexed_placement_preserves_legacy_random_offsets_across_combo_resets() {
        let objects: Vec<_> = (0..384)
            .map(|i| {
                let mut object = fruit(250.0 + (i % 3) as f64 * 6.0, 1000 + i);
                object.object_type = match i % 7 {
                    0 => ObjType::TinyDroplet,
                    1 => ObjType::Droplet,
                    2 => ObjType::Banana,
                    _ => ObjType::Fruit,
                };
                object.scale_factor = match object.object_type {
                    ObjType::TinyDroplet => TINY_DROPLET_SCALE,
                    ObjType::Droplet => DROPLET_SCALE,
                    ObjType::Banana => BANANA_SCALE,
                    ObjType::Fruit => 1.0,
                };
                object.last_in_combo = i % 128 == 127;
                object
            })
            .collect();
        let path = Arc::new(build_catcher_path(&[fruit(256.0, 0)], 5.0));
        for cs in [0.0, 5.0, 10.0] {
            let display = CatcherDisplay::new(&objects, Arc::clone(&path), cs);
            let mut rng = LegacyRandom::new(RNG_SEED as u32);
            let mut stack: Vec<[f64; 2]> = Vec::new();
            for object in &objects {
                let separation = object_diameter(circle_scale(cs).abs(), 1.0, object.scale_factor)
                    * 10.0
                    / OBJECT_RADIUS;
                let mut offset = [object.x - 256.0, 0.0];
                let mut attempts = 0;
                // 保留旧的全栈扫描作为参照，验证索引不会改变后续 RNG 序列。
                while stack
                    .iter()
                    .any(|other| (other[0] - offset[0]).hypot(other[1] - offset[1]) < separation)
                {
                    if attempts < 1024 {
                        offset[0] += (rng.next_double() * 2.0 - 1.0) * separation;
                        offset[1] -= rng.next_double() * 5.0;
                    } else {
                        offset[1] -= separation;
                    }
                    attempts += 1;
                }
                let actual = display
                    .stacks
                    .iter()
                    .flat_map(|stack| &stack.notes)
                    .chain(display.releases.iter().flat_map(|release| &release.notes))
                    .find(|note| note.time == object.event_time_or_start())
                    .unwrap();
                assert_eq!(actual.offset, offset);
                if !matches!(object.object_type, ObjType::Droplet | ObjType::TinyDroplet) {
                    stack.push(offset);
                }
                if object.last_in_combo {
                    stack.clear();
                }
            }
        }
    }

    #[test]
    fn offscreen_stack_culling_preserves_pixels_and_seek_results() {
        let objects: Vec<_> = (0..1024)
            .map(|i| {
                let mut object = fruit(256.0, 1000 + i);
                object.hyper_dash = i % 3 == 0;
                object.last_in_combo = i == 1023;
                object
            })
            .collect();
        let display = build_display(&objects);
        for scale in [0.75, 1.25] {
            let mut config = crate::config::CoreConfig::default();
            config.render.catch.gif.SCALE = scale;
            crate::config::with_config(Arc::new(config), || {
                let layout = build_animation_layout(5.0, 5.0, OutputFormat::Gif);
                for time in [2022, 2023, 2273, 2773, 2022] {
                    let mut expected = Img::new(
                        layout.frame_width as u32,
                        layout.frame_height as u32,
                        [0; 4],
                    );
                    display.notes_at(
                        time,
                        &layout,
                        |object, position, diameter, alpha, clip_bottom| {
                            draw_catch_object_clipped(
                                &mut expected,
                                object,
                                position,
                                diameter,
                                alpha,
                                clip_bottom,
                            );
                        },
                    );
                    display.path.draw_cpu(&mut expected, time, &layout);
                    let mut actual = Img::new(expected.w, expected.h, [0; 4]);
                    display.draw_notes_cpu(&mut actual, time, &layout);
                    display.draw_catcher_cpu(&mut actual, time, &layout);
                    assert!(actual.data == expected.data, "scale={scale}, time={time}");
                    if time == 2022 {
                        let mut visible = 0;
                        display.visible_notes_at(time, &layout, |_, _, _, _, _| visible += 1);
                        assert!(visible > 0 && visible < 1023);
                    }
                }
            });
        }
    }
}
