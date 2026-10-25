//! osu!catch 自动接盘时间轴与几何，供 FL 定位与接盘绘制复用。

use super::animation::AnimationLayout;
use super::constants::*;
use super::objects::{ObjType, RenderObject};
use super::route::{plan_banana_route, BananaRouteAnchor};
use crate::render::canvas::Img;
use crate::render::scene::{FrameSceneBuilder, SceneRect};

const PLATE_COLOR: [u8; 3] = [0x4b, 0x9b, 0xe6];

#[derive(Clone, Copy, Debug)]
struct Keyframe {
    time: f64,
    x: f64,
    dash: bool,
}

#[derive(Clone, Debug)]
pub struct CatcherPath {
    keys: Vec<Keyframe>,
    hyper_intervals: Vec<(f64, f64)>,
    colors: Vec<(f64, [f64; 3], bool)>,
}

impl CatcherPath {
    fn add(&mut self, time: f64, x: f64, dash: bool) {
        self.keys.push(Keyframe {
            time,
            x: x.clamp(0.0, PLAYFIELD_WIDTH),
            dash,
        });
    }

    /// 采用回放的线性插值；同刻帧取最后一个，避免零时间跨度除零。
    pub fn position_at(&self, time: i64) -> f64 {
        self.position_at_f64(time as f64)
    }

    fn position_at_f64(&self, time: f64) -> f64 {
        let index = self.keys.partition_point(|key| key.time <= time);
        if index == 0 {
            return PLAYFIELD_WIDTH / 2.0;
        }
        let a = self.keys[index - 1];
        let Some(b) = self.keys.get(index) else {
            return a.x;
        };
        a.x + (b.x - a.x) * ((time - a.time) / (b.time - a.time))
    }

    fn state_at(&self, time: f64) -> (bool, bool) {
        let index = self.keys.partition_point(|key| key.time <= time);
        let dash = index > 0 && self.keys[index - 1].dash;
        let hyper_index = self
            .hyper_intervals
            .partition_point(|&(start, _)| start <= time);
        let hyper = hyper_index > 0 && time < self.hyper_intervals[hyper_index - 1].1;
        (dash, hyper)
    }

    /// 普通物件逐项移植 lazer AutoGenerator，包括其 Dash 分段插值规则。
    fn move_normal(
        &mut self,
        time: f64,
        target: f64,
        target_hyper: bool,
        half_width: f64,
        last: &mut (f64, f64),
    ) {
        let (last_time, last_x) = *last;
        if time < last_time {
            return;
        }
        let distance = (target - last_x).abs();
        let duration = time - last_time;
        let speed = if distance == 0.0 {
            0.0
        } else {
            distance / duration
        };
        if distance < half_width {
            self.add(time, last_x, false);
            last.0 = time;
            return;
        }
        if speed > BASE_DASH_SPEED {
            self.add(time, target, false);
        } else if target_hyper {
            self.add(last_time, last_x, false);
            self.add(time, target, false);
        } else if speed > BASE_WALK_SPEED {
            let dash_time = (distance / BASE_WALK_SPEED - duration) / 2.0;
            // 与 lazer 保持一致：中间位置按可用时间比例插值，并非按物理速度积分。
            let middle = last_x + (target - last_x) * dash_time / duration;
            self.add(last_time + 1.0, last_x, true);
            self.add(last_time + dash_time, middle, false);
            self.add(time, target, false);
        } else {
            self.add(time - distance / BASE_WALK_SPEED, last_x, false);
            self.add(time, target, false);
        }
        *last = (time, target);
    }

    /// 香蕉按规划中的实际中心运动，Dash 使用最短必要持续时间。
    fn move_banana(&mut self, time: f64, target: f64, last: &mut (f64, f64)) {
        let (last_time, last_x) = *last;
        let duration = (time - last_time).max(0.0);
        let distance = (target - last_x).abs();
        let dash_time = ((distance - duration * BASE_WALK_SPEED)
            / (BASE_DASH_SPEED - BASE_WALK_SPEED))
            .clamp(0.0, duration);
        if dash_time > 0.0 {
            self.add(last_time, last_x, true);
            let direction = (target - last_x).signum();
            self.add(
                last_time + dash_time,
                last_x + direction * BASE_DASH_SPEED * dash_time,
                false,
            );
        } else {
            self.add(
                (time - distance / BASE_WALK_SPEED).max(last_time),
                last_x,
                false,
            );
        }
        self.add(time, target, false);
        *last = (time, target);
    }

    fn color_at(&self, time: f64) -> [u8; 3] {
        let hyper = crate::config::current().skin.HYPER_DASH;
        let index = self.colors.partition_point(|&(at, _, _)| at <= time);
        let color = if index == 0 {
            PLATE_COLOR.map(f64::from)
        } else {
            let (at, from, active) = self.colors[index - 1];
            blend_color(from, if active { hyper } else { PLATE_COLOR }, time - at)
        };
        color.map(|v| v.round().clamp(0.0, 255.0) as u8)
    }

    /// 固定时间采样而不是依赖输出帧率，使跳转和多段 GIF 保持相同拖尾。
    fn plates(
        &self,
        time: i64,
        layout: &AnimationLayout,
        mut draw: impl FnMut(SceneRect, [u8; 4], bool),
    ) {
        // 与 lazer 一致：每 16ms 生成完整盘子，初始透明度 0.4，800ms 内按 OutQuint 淡出。
        for sample in ((time - 800).div_euclid(16) + 1)..=time.div_euclid(16) {
            let at = (sample * 16) as f64;
            let (dash, hyper) = self.state_at(at);
            if !dash && !hyper {
                continue;
            }
            let age = ((time as f64 - at) / 800.0).clamp(0.0, 1.0);
            let alpha = (102.0 * (1.0 - age).powi(5)).round() as u8;
            let rgb = if hyper {
                crate::config::current().skin.HYPER_DASH
            } else {
                PLATE_COLOR
            };
            plate_geometry(
                self.position_at_f64(at),
                layout,
                [rgb[0], rgb[1], rgb[2], alpha],
                |rect, color| draw(rect, color, true),
            );
        }
        let rgb = self.color_at(time as f64);
        plate_geometry(
            self.position_at(time),
            layout,
            [rgb[0], rgb[1], rgb[2], 255],
            |rect, color| draw(rect, color, false),
        );
    }

    pub fn draw_cpu(&self, frame: &mut Img, time: i64, layout: &AnimationLayout) {
        self.plates(time, layout, |rect, color, additive| {
            if additive {
                let x = rect.x.round() as i64;
                let y = rect.y.round() as i64;
                let width = rect.width.round().max(1.0) as i64;
                let height = rect.height.round().max(1.0) as i64;
                // lazer 残影使用 SrcAlpha / One 加色，本体仍使用普通透明合成。
                for py in y.max(0)..(y + height).min(frame.h as i64) {
                    for px in x.max(0)..(x + width).min(frame.w as i64) {
                        let offset = ((py as u32 * frame.w + px as u32) * 4) as usize;
                        for channel in 0..3 {
                            let added = (color[channel] as u32 * color[3] as u32 + 127) / 255;
                            frame.data[offset + channel] =
                                frame.data[offset + channel].saturating_add(added as u8);
                        }
                        frame.data[offset + 3] = frame.data[offset + 3].saturating_add(color[3]);
                    }
                }
                return;
            }
            frame.fill_rect_size(
                rect.x.round() as i64,
                rect.y.round() as i64,
                rect.width.round().max(1.0) as i64,
                rect.height.round().max(1.0) as i64,
                color,
            )
        });
    }

    pub fn draw_scene(&self, scene: &mut FrameSceneBuilder, time: i64, layout: &AnimationLayout) {
        let white = std::sync::Arc::new(Img::new(1, 1, [255; 4]));
        self.plates(time, layout, |rect, color, additive| {
            if additive {
                scene.transformed_sprite(
                    white.clone(),
                    crate::render::scene::SpriteSpec {
                        position: [rect.x, rect.y],
                        origin: [0.0, 0.0],
                        size: [rect.width, rect.height],
                        rotation: 0.0,
                        color,
                        additive: true,
                    },
                );
            } else {
                scene.rectangle(rect, color);
            }
        });
    }
}

fn blend_color(from: [f64; 3], to: [u8; 3], elapsed: f64) -> [f64; 3] {
    let amount = 1.0 - (1.0 - (elapsed / 180.0).clamp(0.0, 1.0)).powi(5);
    std::array::from_fn(|i| from[i] + (to[i] as f64 - from[i]) * amount)
}

fn plate_geometry(
    x: f64,
    layout: &AnimationLayout,
    color: [u8; 4],
    mut draw: impl FnMut(SceneRect, [u8; 4]),
) {
    let scale = layout.playfield_scale;
    let width = CATCHER_BASE_SIZE * (layout.object_scale * 2.0).abs() * scale;
    let center = layout.playfield_left + x * scale;
    let y = layout.playfield_top + STABLE_CATCHER_Y * scale;
    // 深色外沿分离判定线和拖尾；由逐行收窄的浅碗代替三块白色台阶。
    let outline = [25, 35, 48, color[3]];
    let shade = |factor: f64| {
        [
            (color[0] as f64 * factor).round() as u8,
            (color[1] as f64 * factor).round() as u8,
            (color[2] as f64 * factor).round() as u8,
            color[3],
        ]
    };
    for row in 0..16 {
        let depth = row as f64 / 15.0;
        let fraction = (1.0 - depth * depth * 0.45).sqrt();
        let row_width = width * fraction;
        draw(
            SceneRect {
                x: (center - row_width / 2.0 - scale) as f32,
                y: (y + row as f64 * scale) as f32,
                width: (row_width + 2.0 * scale) as f32,
                height: scale as f32,
            },
            outline,
        );
        if row > 0 && row < 15 {
            draw(
                SceneRect {
                    x: (center - row_width / 2.0 + scale) as f32,
                    y: (y + row as f64 * scale) as f32,
                    width: (row_width - 2.0 * scale).max(scale) as f32,
                    height: scale as f32,
                },
                shade(if row <= 3 { 1.0 } else { 0.72 + depth * 0.13 }),
            );
        }
    }
}

/// 输入为未按绘制顺序排序的展开物件；稳定排序保留同刻物件顺序。
/// 香蕉规划只改临时副本，动态输出保留原色，FL 不参与任何规划决策。
pub fn build_catcher_path(objects: &[RenderObject], circle_size: f64) -> CatcherPath {
    let mut ordered: Vec<_> = objects
        .iter()
        .filter(|object| object.event_time_or_start().is_finite())
        .cloned()
        .collect();
    for object in &mut ordered {
        object.x = if object.x.is_finite() {
            object.x.clamp(0.0, PLAYFIELD_WIDTH)
        } else {
            PLAYFIELD_WIDTH / 2.0
        };
    }
    ordered.sort_by(|a, b| a.event_time_or_start().total_cmp(&b.event_time_or_start()));
    let initial_time = ordered
        .first()
        .map_or(0.0, |o| o.event_time_or_start().min(0.0));
    let mut path = CatcherPath {
        keys: vec![Keyframe {
            time: initial_time,
            x: PLAYFIELD_WIDTH / 2.0,
            dash: false,
        }],
        hyper_intervals: Vec::new(),
        colors: Vec::new(),
    };
    let half_width =
        CATCHER_BASE_SIZE * (1.0 - 0.7 * ((circle_size - 5.0) / 5.0)).abs() * ALLOWED_CATCH_RANGE
            / 2.0;
    let mut last = (initial_time, PLAYFIELD_WIDTH / 2.0);
    let mut index = 0;
    while index < ordered.len() {
        let object = &ordered[index];
        if object.object_type == ObjType::Banana {
            let start = index;
            while index < ordered.len() && ordered[index].object_type == ObjType::Banana {
                index += 1;
            }
            let exit = ordered[index..]
                .iter()
                .find(|o| matches!(o.object_type, ObjType::Fruit | ObjType::Droplet))
                .map(|o| BananaRouteAnchor {
                    x: o.x,
                    time: o.event_time_or_start(),
                });
            plan_banana_route(
                &mut ordered[start..index],
                circle_size,
                Some(BananaRouteAnchor {
                    time: last.0,
                    x: last.1,
                }),
                exit,
                true,
            );
            for banana in &ordered[start..index] {
                if let Some(x) = banana.banana_route_x {
                    path.move_banana(banana.event_time_or_start(), x, &mut last);
                }
            }
            // 规划出口约束的是接取范围；不能再强制到水果中心，否则会引入不必要跳转。
            if let Some(exit) = exit {
                // 极端 CS 或边界物件可能没有非空判定区间；此时直接保留目标中心。
                let margin = half_width.min(1e-7);
                let minimum = (exit.x - half_width + margin).max(0.0);
                let maximum = (exit.x + half_width - margin).min(PLAYFIELD_WIDTH);
                let landing = if minimum <= maximum {
                    last.1.clamp(minimum, maximum)
                } else {
                    exit.x
                };
                if (landing - last.1).abs() <= (exit.time - last.0).max(0.0) * BASE_DASH_SPEED {
                    path.move_banana(exit.time, landing, &mut last);
                }
            }
            continue;
        }
        let time = object.event_time_or_start();
        path.move_normal(time, object.x, object.hyper_dash, half_width, &mut last);
        if object.object_type != ObjType::TinyDroplet && object.hyper_dash {
            if let Some(target) = ordered[index + 1..]
                .iter()
                .find(|o| matches!(o.object_type, ObjType::Fruit | ObjType::Droplet))
            {
                let end = target.event_time_or_start();
                let velocity = (target.x - last.1).abs() / (end - time - 1000.0 / 60.0).max(1.0);
                if velocity > BASE_DASH_SPEED {
                    path.hyper_intervals.push((time, end));
                }
            }
        }
        index += 1;
    }
    path.keys.sort_by(|a, b| a.time.total_cmp(&b.time));
    // 回放直接插值位置；到达 hyperdash 目标即结束状态，而非等待目标判定。
    for i in 0..path.hyper_intervals.len() {
        let (start, end) = path.hyper_intervals[i];
        // lazer 的同刻重叠物件没有可用移动时间；回放允许直接切换位置，
        // 但不能为零时长状态构造反向切片或插值区间。
        if end <= start {
            continue;
        }
        let target = ordered
            .iter()
            .find(|o| {
                matches!(o.object_type, ObjType::Fruit | ObjType::Droplet)
                    && o.event_time_or_start() == end
            })
            .map_or(path.position_at_f64(end), |o| o.x);
        let direction = (target - path.position_at_f64(start)).signum();
        if direction == 0.0 {
            path.hyper_intervals[i].1 = start;
            continue;
        }
        for pair in path.keys.windows(2) {
            let a = pair[0];
            let b = pair[1];
            if b.time <= start || a.time >= end {
                continue;
            }
            if (b.x - target) * direction >= 0.0 && b.x != a.x {
                let crossing = a.time + (target - a.x) / (b.x - a.x) * (b.time - a.time);
                if crossing >= start {
                    path.hyper_intervals[i].1 = crossing.min(end);
                    break;
                }
            }
        }
    }
    // 香蕉也会提交接取判定并清除 hyperdash；小水滴不会改变该状态。
    for i in 0..path.hyper_intervals.len() {
        let (start, end) = path.hyper_intervals[i];
        if end <= start {
            continue;
        }
        let lo = ordered.partition_point(|object| object.event_time_or_start() <= start);
        let hi = ordered.partition_point(|object| object.event_time_or_start() < end);
        for banana in ordered.get(lo..hi).unwrap_or(&[]) {
            if banana.object_type == ObjType::Banana {
                let time = banana.event_time_or_start();
                if (banana.x - path.position_at_f64(time)).abs() <= half_width {
                    path.hyper_intervals[i].1 = time;
                    break;
                }
            }
        }
    }
    path.hyper_intervals
        .retain(|&(start, end)| start.is_finite() && end.is_finite() && end > start);
    // 同一时刻 hyperdash 优先，连续触发合并后不会让颜色过渡倒序。
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for &(start, end) in &path.hyper_intervals {
        if let Some(previous) = merged.last_mut() {
            if start <= previous.1 {
                previous.1 = previous.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    path.hyper_intervals = merged;
    let hyper_color = crate::config::current().skin.HYPER_DASH;
    for &(start, end) in &path.hyper_intervals {
        for (at, active) in [(start, true), (end, false)] {
            let from = path.colors.last().map_or(
                PLATE_COLOR.map(f64::from),
                |&(previous, color, was_active)| {
                    blend_color(
                        color,
                        if was_active { hyper_color } else { PLATE_COLOR },
                        at - previous,
                    )
                },
            );
            path.colors.push((at, from, active));
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::super::objects::build_fruit_object;
    use super::*;

    fn object(x: f64, time: i64) -> RenderObject {
        build_fruit_object(x, time, [100, 150, 200], None)
    }

    #[test]
    fn walk_waits_and_range_preserves_actual_center() {
        let path = build_catcher_path(&[object(100.0, 1000), object(110.0, 1100)], 5.0);
        assert_eq!(path.position_at(500), 256.0);
        assert_eq!(path.position_at(844), 178.0);
        assert_eq!(path.position_at(1100), 100.0);
        assert_eq!(path.position_at(-1000), 256.0);
        assert_eq!(path.position_at(844), 178.0);
    }

    #[test]
    fn dash_frames_match_lazer_and_hyper_is_red_only_after_trigger() {
        let path = build_catcher_path(&[object(100.0, 1000), object(250.0, 1200)], 5.0);
        assert!(path.state_at(1020.0).0);
        assert!(!path.state_at(1060.0).0);
        assert_eq!(path.position_at(1050), 137.5);
        assert_eq!(path.color_at(1040.0), PLATE_COLOR);
        let mut source = object(0.0, 1000);
        source.hyper_dash = true;
        let path = build_catcher_path(&[source, object(512.0, 1500)], 5.0);
        assert!(!path.state_at(999.0).1);
        assert!(path.state_at(1100.0).1);
        assert_eq!(
            path.color_at(1180.0),
            crate::config::current().skin.HYPER_DASH
        );
        assert!(!path.state_at(1500.0).1);
        assert_eq!(path.color_at(1680.0), PLATE_COLOR);
    }

    #[test]
    fn caught_banana_clears_hyper_but_tiny_droplet_does_not() {
        let mut source = object(0.0, 1000);
        source.hyper_dash = true;
        let mut banana = object(0.0, 1100);
        banana.object_type = ObjType::Banana;
        let path = build_catcher_path(&[source.clone(), banana, object(512.0, 1400)], 5.0);
        assert!(path.state_at(1050.0).1);
        assert!(!path.state_at(1100.0).1);
        let mut droplet = object(200.0, 1200);
        droplet.object_type = ObjType::TinyDroplet;
        let path = build_catcher_path(&[source, droplet, object(512.0, 1500)], 5.0);
        assert!(path.state_at(1250.0).1);
    }

    #[test]
    fn overlapping_hyper_objects_keep_replay_finite_and_mods_switchable() {
        use crate::domain::mods::ModSettings;
        let beatmap = crate::parse_beatmap_bytes(
            b"osu file format v14\n[General]\nMode:2\n[Difficulty]\nCircleSize:4\nApproachRate:5\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n0,192,1000,1,0\n512,192,1000,1,0\n0,192,1000,1,0\n512,192,1100,1,0\n"
        ).unwrap();
        for flashlight in [false, true, false] {
            let mods = ModSettings {
                flashlight,
                ..ModSettings::new()
            };
            let source = crate::render::wgpu::prepare_catch(&beatmap, Some(&mods)).unwrap();
            for time in [999, 1000, 1001, 1100, 1000] {
                assert!(!source.render(time).unwrap().commands.is_empty());
            }
        }
        let mut hyper = object(0.0, 1000);
        hyper.hyper_dash = true;
        let path = build_catcher_path(&[hyper, object(512.0, 1000), object(100.0, 2000)], 4.0);
        assert_eq!(path.position_at(1000), 512.0);
        assert!(path.hyper_intervals.iter().all(|&(start, end)| end > start));
        assert!(path.position_at(1001).is_finite());
    }

    #[test]
    fn invalid_coordinates_and_zero_catch_width_do_not_panic() {
        let mut banana = object(512.0, 1000);
        banana.object_type = ObjType::Banana;
        for cs in [5.0, 5.0 + 5.0 / 0.7] {
            let path = build_catcher_path(
                &[
                    banana.clone(),
                    object(-1000.0, 1100),
                    object(f64::NAN, 1200),
                ],
                cs,
            );
            for time in [1000, 1100, 1200] {
                assert!((0.0..=512.0).contains(&path.position_at(time)));
            }
        }
    }

    #[test]
    fn precise_and_same_time_events_have_finite_positions() {
        let mut first = object(0.0, 1000);
        first.event_time = Some(1000.25);
        let mut second = object(512.0, 1000);
        second.event_time = Some(1000.25);
        let path = build_catcher_path(&[first, second], 5.0);
        assert_eq!(path.position_at_f64(1000.25), 512.0);
        assert!(path.position_at(1000).is_finite());
    }

    #[test]
    fn banana_planner_starts_at_actual_center_and_preserves_colors() {
        let mut bananas = vec![object(0.0, 1100), object(512.0, 1150), object(20.0, 1200)];
        for banana in &mut bananas {
            banana.object_type = ObjType::Banana;
        }
        let mut objects = vec![object(0.0, 1000)];
        objects.extend(bananas.clone());
        objects.push(object(20.0, 1300));
        let path = build_catcher_path(&objects, 5.0);
        assert_eq!(path.position_at(1000), 0.0);
        assert!(path.position_at(1150) < 200.0);
        assert!((path.position_at(1300) - 20.0).abs() <= CATCHER_BASE_SIZE * 0.4);
        for pair in path
            .keys
            .windows(2)
            .filter(|p| p[0].time >= 1000.0 && p[1].time <= 1200.0)
        {
            assert!((pair[1].x - pair[0].x).abs() <= pair[1].time - pair[0].time + 1e-6);
        }
        assert_eq!(objects[1].color, bananas[0].color);
    }

    #[test]
    fn flashlight_consumes_catcher_position_and_keeps_combo_radius() {
        use super::super::animation::{build_animation_layout, CatchFlashlight};
        use crate::render::geometry::OutputFormat;
        use std::sync::Arc;
        let objects = [object(0.0, 1000), object(512.0, 1500)];
        let path = Arc::new(build_catcher_path(&objects, 5.0));
        let flashlight = CatchFlashlight::new(&objects, &[], path.clone());
        let layout = build_animation_layout(5.0, 5.0, OutputFormat::Gif);
        for time in [0, 1000, 1250, 1500, 1250] {
            let fl = flashlight.at(time, &layout);
            assert_eq!(
                fl.center[0],
                layout.playfield_left + path.position_at(time) * layout.playfield_scale
            );
            assert!(fl.radius > 0.0);
        }
    }

    #[test]
    fn flashlight_preserves_the_path_without_drawing_the_plate() {
        use super::super::animation::build_video_animation_layout;
        use super::super::objects::{build_catch_render_objects, effective_difficulty};
        use crate::domain::mods::ModSettings;
        use crate::render::geometry::OutputFormat;
        let beatmap = crate::parse_beatmap_bytes(
            b"osu file format v14\n[General]\nMode:2\n[Difficulty]\nCircleSize:5\nApproachRate:5\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n0,192,1000,1,0\n512,192,1400,1,0\n256,192,2000,8,0,2500\n128,192,3000,1,0\n"
        ).unwrap();
        for (easy, hard_rock, speed) in
            [(false, false, 1.0), (true, false, 1.5), (false, true, 0.75)]
        {
            let mut reference = None;
            for flashlight in [false, true] {
                let mods = ModSettings {
                    flashlight,
                    easy,
                    hard_rock,
                    hidden: true,
                    speed_multiplier: speed,
                    ..ModSettings::new()
                };
                let difficulty = effective_difficulty(&beatmap, Some(&mods));
                let objects = build_catch_render_objects(
                    &beatmap,
                    beatmap.hit_objects.as_catch().unwrap(),
                    Some(&mods),
                    &difficulty,
                    false,
                )
                .unwrap();
                let path = build_catcher_path(&objects, difficulty.cs);
                let positions: Vec<_> = [0, 1000, 1200, 2200, 3000, 1200]
                    .map(|t| path.position_at(t))
                    .into_iter()
                    .collect();
                if let Some(expected) = &reference {
                    assert_eq!(&positions, expected);
                } else {
                    reference = Some(positions);
                }
                let source = crate::render::wgpu::prepare_catch(&beatmap, Some(&mods)).unwrap();
                let scene = source.render(1000).unwrap();
                let rectangles = scene
                    .commands
                    .iter()
                    .filter(|command| {
                        matches!(command, crate::render::scene::DrawCommand::Rectangle { .. })
                    })
                    .count();
                assert_eq!(rectangles, 2);
                let layout =
                    build_video_animation_layout(difficulty.cs, difficulty.ar, OutputFormat::Mp4);
                let center =
                    layout.playfield_left + path.position_at(1000) * layout.playfield_scale;
                assert!(center.is_finite());
            }
        }
    }

    #[test]
    fn geometry_scales_with_the_current_output_and_cs() {
        use super::super::animation::build_animation_layout;
        use crate::render::geometry::OutputFormat;
        let base = build_animation_layout(5.0, 5.0, OutputFormat::Gif);
        let mut scaled = build_animation_layout(5.0, 5.0, OutputFormat::Gif);
        scaled.playfield_scale *= 2.0;
        scaled.playfield_left *= 2.0;
        scaled.playfield_top *= 2.0;
        scaled.render_scale *= 2.0;
        let mut a = Vec::new();
        let mut b = Vec::new();
        plate_geometry(256.0, &base, [255; 4], |rect, _| a.push(rect));
        plate_geometry(256.0, &scaled, [255; 4], |rect, _| b.push(rect));
        for (a, b) in a.iter().zip(b.iter()) {
            assert_eq!(a.x * 2.0, b.x);
            assert_eq!(a.y * 2.0, b.y);
            assert_eq!(a.width * 2.0, b.width);
            assert_eq!(a.height * 2.0, b.height);
        }
    }

    #[test]
    fn cpu_and_scene_use_identical_plate_rectangles() {
        use super::super::animation::build_animation_layout;
        use crate::render::geometry::OutputFormat;
        let path = build_catcher_path(&[object(0.0, 1000)], 5.0);
        let layout = build_animation_layout(5.0, 5.0, OutputFormat::Gif);
        let mut scene =
            FrameSceneBuilder::new(layout.frame_width as u32, layout.frame_height as u32, 1000);
        path.draw_scene(&mut scene, 1000, &layout);
        let scene = scene.finish();
        assert_eq!(scene.commands.len(), 30);
        let mut image = Img::new(
            layout.frame_width as u32,
            layout.frame_height as u32,
            [0; 4],
        );
        path.draw_cpu(&mut image, 1000, &layout);
        let mut expected = Img::new(image.w, image.h, [0; 4]);
        for command in scene.commands.iter() {
            if let crate::render::scene::DrawCommand::Rectangle { rect, color } = command {
                expected.fill_rect_size(
                    rect.x.round() as i64,
                    rect.y.round() as i64,
                    rect.width.round().max(1.0) as i64,
                    rect.height.round().max(1.0) as i64,
                    *color,
                );
            } else {
                panic!("盘子只使用共享矩形几何");
            }
        }
        assert_eq!(image.data, expected.data);
    }
}
