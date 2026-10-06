//! 参考 lazer 的 OsuAutoGenerator 生成光标位置帧；不执行判定或计分。

use std::sync::{Arc, OnceLock};

use crate::domain::shared::slider_path::path_position_at;
use crate::render::canvas::Img;
use crate::render::visibility::AutoplayPath;
use crate::storyboard::eval::apply_easing_f64;

use super::context::{stacked_position, to_frame_point, RenderCache, RenderContext};
use super::slider::get_slider_render_data;

#[derive(Clone, Copy)]
struct Frame {
    time: f64,
    position: [f64; 2],
    held: bool,
    key_up: bool,
}

pub struct AutoplayCache(OnceLock<AutoplayCursor>);

impl AutoplayCache {
    pub fn new() -> Self {
        Self(OnceLock::new())
    }

    /// 等最终画布布局就绪后构建，CPU 多线程导出共享同一份轨迹与精灵。
    pub fn get(&self, context: &RenderContext) -> &AutoplayCursor {
        self.0.get_or_init(|| AutoplayCursor::new(context))
    }
}

pub struct AutoplayCursor {
    frames: Vec<Frame>,
    presses: Vec<f64>,
    animations: Vec<(f64, bool, f64)>,
    sprites: Vec<Arc<Img>>,
    rate: f64,
}

impl AutoplayCursor {
    fn new(context: &RenderContext) -> Self {
        let scale = context.frame_layout.scale;
        let mut cursor = Self {
            frames: Vec::new(),
            presses: Vec::new(),
            animations: Vec::new(),
            // 外环缩放量化为 0.01，CPU/WGPU 共用精灵，避免逐帧重采样。
            sprites: if context.show_cursor {
                (0..=50)
                    .map(|index| Arc::new(argon_sprite(scale, 0.9 + index as f64 * 0.01)))
                    .collect()
            } else {
                Vec::new()
            },
            rate: context.spinner_rate,
        };
        let Some(first) = context.hit_objects.first() else {
            return cursor;
        };
        let initial = to_frame_point(256.0, 500.0, &context.frame_layout);
        cursor.add(
            first.start_time as f64 - 1500.0,
            [initial.0, initial.1],
            false,
            false,
        );
        let mut cache = RenderCache::default();
        let rate = context.spinner_rate;
        let delay = 1000.0 / 60.0 * rate;
        let spin_center = to_frame_point(256.0, 192.0, &context.frame_layout);
        let spin_center = [spin_center.0, spin_center.1];
        let radius = 50.0 * scale;
        for (index, object) in context.hit_objects.iter().enumerate() {
            let start = object.start_time as f64;
            let end = object.end_time.max(object.start_time) as f64;
            let spinner = object.hit_type & 8 != 0;
            let slider = object.hit_type & 2 != 0 && !spinner;
            // 无需旋转即可完成的短转盘与官方生成器一致，不改变光标位置。
            let rpm = if context.spinner_od < 5.0 {
                90.0 + 12.0 * context.spinner_od
            } else {
                150.0 + 15.0 * (context.spinner_od - 5.0)
            };
            if spinner && (rpm / 60.0 * (end - start) / 1000.0 + 0.0001) < 1.0 {
                continue;
            }
            let last = *cursor.frames.last().unwrap();
            let world = stacked_position(object, &context.settings);
            let point = to_frame_point(world.0, world.1, &context.frame_layout);
            let (target, direction, ease_in) = if spinner {
                spinner_entry(last.position, spin_center, radius)
            } else {
                ([point.0, point.1], 1.0, false)
            };
            // 100ms 是实际时间，倍速后换回谱面时间；物件间采用默认 Out 二次缓动。
            let wait = start - (context.settings.preempt_ms as f64 - 100.0 * rate).max(0.0);
            if wait > last.time {
                cursor.add(wait, last.position, last.held, false);
            }
            let last_index = cursor.frames.len() - 1;
            let mut last = cursor.frames[last_index];
            if start >= last.time {
                if last.key_up && wait <= last.time && last_index > 0 {
                    let previous = cursor.frames[last_index - 1];
                    last.position = eased(
                        previous.time,
                        start,
                        last.time,
                        last.position,
                        target,
                        ease_in,
                    );
                    cursor.frames[last_index].position = last.position;
                }
                let mut time = last.time + delay;
                while time < start {
                    cursor.add(
                        time.floor(),
                        eased(last.time, start, time, last.position, target, ease_in),
                        last.held,
                        false,
                    );
                    time += delay;
                }
            }
            let release = end + 50.0 + if spinner { 1.0 } else { 0.0 };
            let insertion = cursor.frames.partition_point(|frame| frame.time <= start);
            // 重叠时较新的物件优先；保留它结束之后的旧物件帧，避免整条长滑条丢失。
            if insertion > 0 && cursor.frames[insertion - 1].held {
                let end_index = cursor.frames.partition_point(|frame| frame.time <= release);
                cursor.frames.drain(insertion..end_index);
            }
            cursor.add(start, target, true, false);
            cursor.presses.push(start);
            let mut end_position = target;
            if slider {
                let data = get_slider_render_data(&mut cache, context, index);
                let spans = object.slider_repeats.max(1) as f64;
                let position = |time: f64| {
                    let progress = if end > start {
                        (time - start) / (end - start) * spans
                    } else {
                        spans
                    };
                    let span = progress.floor();
                    let fraction = progress - span;
                    let fraction = if (span as u64).is_multiple_of(2) {
                        fraction
                    } else {
                        1.0 - fraction
                    };
                    let point = path_position_at(&data.timing_path, fraction);
                    [point.0, point.1]
                };
                let mut time = start + delay;
                while time < end {
                    cursor.add(time, position(time), true, false);
                    time += delay;
                }
                end_position = position(end);
                cursor.add(end, end_position, true, false);
            } else if spinner {
                let angle = (target[1] - spin_center[1]).atan2(target[0] - spin_center[0]);
                let position = |time: f64| {
                    let angle = angle + (time - start) / rate * direction * 0.05;
                    [
                        spin_center[0] + radius * angle.cos(),
                        spin_center[1] + radius * angle.sin(),
                    ]
                };
                let mut time = start + delay;
                while time < end {
                    cursor.add(time.floor(), position(time), true, false);
                    time += delay;
                }
                end_position = position(end);
                cursor.add(end, end_position, true, false);
            }
            if cursor.frames.last().unwrap().time <= release {
                cursor.add(release, end_position, false, true);
            }
        }
        if context.show_cursor {
            let mut events = cursor
                .presses
                .iter()
                .map(|&time| (time, true))
                .collect::<Vec<_>>();
            events.extend(
                cursor
                    .frames
                    .iter()
                    .filter(|frame| frame.key_up)
                    .map(|frame| (frame.time, false)),
            );
            events.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (time, pressed) in events {
                let from = if pressed {
                    1.0
                } else {
                    cursor.expansion_at(time)
                };
                cursor.animations.push((time, pressed, from));
            }
        }
        cursor
    }

    /// 通常顺序追加为 O(1)，只有重叠物件需要插入已有的未来帧。
    fn add(&mut self, time: f64, position: [f64; 2], held: bool, key_up: bool) {
        let frame = Frame {
            time,
            position,
            held,
            key_up,
        };
        if self.frames.last().is_none_or(|last| last.time <= time) {
            self.frames.push(frame);
        } else {
            let index = self.frames.partition_point(|frame| frame.time <= time);
            self.frames.insert(index, frame);
        }
    }

    /// 查询无状态、O(log n)，跳转、乱序导出和不同帧率得到相同位置。
    pub fn position_at(&self, time: i64) -> Option<[f64; 2]> {
        self.position_at_f64(time as f64)
    }

    fn position_at_f64(&self, time: f64) -> Option<[f64; 2]> {
        let first = self.frames.first()?;
        if time < first.time {
            return None;
        }
        let index = self.frames.partition_point(|frame| frame.time <= time) - 1;
        let from = self.frames[index];
        let Some(to) = self.frames.get(index + 1) else {
            return Some(from.position);
        };
        Some(lerp(
            from.position,
            to.position,
            ((time - from.time) / (to.time - from.time)).clamp(0.0, 1.0),
        ))
    }

    fn expansion_at(&self, time: f64) -> f64 {
        let index = self.animations.partition_point(|event| event.0 <= time);
        let Some(&(start, pressed, from)) =
            index.checked_sub(1).map(|index| &self.animations[index])
        else {
            return 1.0;
        };
        let progress = ((time - start) / (400.0 * self.rate)).clamp(0.0, 1.0);
        let easing = apply_easing_f64(if pressed { 26 } else { 4 }, progress);
        from + ((if pressed { 1.2 } else { 1.0 }) - from) * easing
    }

    pub fn sprite_at(&self, time: i64) -> Option<&Arc<Img>> {
        let index = ((self.expansion_at(time as f64) - 0.9) * 100.0)
            .round()
            .clamp(0.0, 50.0) as usize;
        self.sprites.get(index)
    }

    /// 只遍历最近 300ms 实际时间的轨迹，按 4ms 分段避免淡出出现明显阶梯。
    /// 不保存上一帧状态，也不分配临时数组；暂停、seek 和乱序导出保持一致。
    pub fn visit_trail(
        &self,
        time: i64,
        scale: f64,
        mut draw: impl FnMut([f64; 2], [f64; 2], f64, u8),
    ) {
        if self.sprites.is_empty() || self.frames.len() < 2 {
            return;
        }
        let now = time as f64;
        let duration = 300.0 * self.rate;
        let start = now - duration;
        let first = self
            .frames
            .partition_point(|frame| frame.time < start)
            .saturating_sub(1);
        let last = self
            .frames
            .partition_point(|frame| frame.time < now)
            .min(self.frames.len() - 1);
        for pair in self.frames[first.min(last)..=last].windows(2) {
            let span = pair[1].time - pair[0].time;
            if span <= 0.0 || pair[0].position == pair[1].position {
                continue;
            }
            let mut from_time = pair[0].time.max(start);
            let end = pair[1].time.min(now);
            while from_time < end {
                let to_time = (from_time + 4.0 * self.rate).min(end);
                let from = lerp(
                    pair[0].position,
                    pair[1].position,
                    (from_time - pair[0].time) / span,
                );
                let to = lerp(
                    pair[0].position,
                    pair[1].position,
                    (to_time - pair[0].time) / span,
                );
                let remaining =
                    (1.0 - (now - (from_time + to_time) / 2.0) / duration).clamp(0.0, 1.0);
                let alpha = (255.0 * 0.8 * remaining.powi(4)).round() as u8;
                if alpha > 0 {
                    draw(from, to, 2.0 * scale * self.expansion_at(to_time), alpha);
                }
                from_time = to_time;
            }
        }
    }

    /// 游戏每次鼠标更新用 120ms 的 Out 插值。固定按实际时间 60Hz 预计算，
    /// 复现默认跟随延迟，同时避免渲染帧率、乱序查询和 seek 改变光圈位置。
    pub fn flashlight_path(&self, rate: f64) -> AutoplayPath {
        let Some(first) = self.frames.first() else {
            return AutoplayPath::new(Vec::new(), 0.0);
        };
        let step = 1000.0 / 60.0 * rate;
        let mut position = first.position;
        let mut points = vec![(first.time, position)];
        let end = self.frames.last().unwrap().time + 1200.0 * rate;
        let progress = 1.0 - (1.0 - (step / rate / 120.0).min(1.0)).powi(2);
        let mut time = first.time + step;
        while time < end {
            position = lerp(position, self.position_at_f64(time).unwrap(), progress);
            points.push((time, position));
            time += step;
        }
        points.push((end, self.frames.last().unwrap().position));
        AutoplayPath::new(points, 0.0)
    }
}

/// Argon Pro 继承 ArgonCursor：渐变粗环、白色细边、暗色填充和发光中心。
/// 点击仅缩放外环，中心点与光晕保持固定大小。
fn argon_sprite(scale: f64, expansion: f64) -> Img {
    let side = (68.0 * scale).ceil().max(1.0) as u32;
    let mut image = Img::new(side, side, [0, 0, 0, 0]);
    let center = side as f64 / 2.0;
    let radius = 14.0 * scale * expansion;
    let border = 6.0 * scale * expansion;
    let white_border = 2.0 * scale * expansion;
    for y in 0..side {
        for x in 0..side {
            let distance = (x as f64 + 0.5 - center).hypot(y as f64 + 0.5 - center);
            let outer = (radius - distance + 0.5).clamp(0.0, 1.0);
            if outer > 0.0 {
                image.blend_px(x as i64, y as i64, [101, 39, 57, (102.0 * outer) as u8]);
                let ring = outer * (distance - (radius - border) + 0.5).clamp(0.0, 1.0);
                let gradient =
                    ((y as f64 + 0.5 - center + radius) / (radius * 2.0)).clamp(0.0, 1.0);
                image.blend_px(
                    x as i64,
                    y as i64,
                    [
                        (252.0 - 65.0 * gradient) as u8,
                        (97.0 - 71.0 * gradient) as u8,
                        (143.0 - 78.0 * gradient) as u8,
                        (255.0 * ring) as u8,
                    ],
                );
                let edge = outer * (distance - (radius - white_border) + 0.5).clamp(0.0, 1.0);
                image.blend_px(x as i64, y as i64, [255, 255, 255, (204.0 * edge) as u8]);
            }
            // 用平滑径向衰减近似游戏 EdgeEffect 的 20px Glow。
            let glow = (1.0 - ((distance / scale - 2.8) / 20.0).clamp(0.0, 1.0)).powi(3);
            image.blend_px(x as i64, y as i64, [171, 255, 255, (100.0 * glow) as u8]);
        }
    }
    image.fill_circle_aa(center, center, 2.8 * scale, [255, 255, 255, 255]);
    image
}

fn lerp(from: [f64; 2], to: [f64; 2], progress: f64) -> [f64; 2] {
    std::array::from_fn(|axis| from[axis] + (to[axis] - from[axis]) * progress)
}

fn eased(start: f64, end: f64, time: f64, from: [f64; 2], to: [f64; 2], ease_in: bool) -> [f64; 2] {
    let progress = if end > start {
        ((time - start) / (end - start)).clamp(0.0, 1.0)
    } else {
        1.0
    };
    lerp(
        from,
        to,
        if ease_in {
            progress * progress
        } else {
            1.0 - (1.0 - progress).powi(2)
        },
    )
}

/// 转盘外从切线方向进入；内部从最近的圆周点开始，中心位置单独处理。
fn spinner_entry(previous: [f64; 2], center: [f64; 2], radius: f64) -> ([f64; 2], f64, bool) {
    let delta = [center[0] - previous[0], center[1] - previous[1]];
    let distance = delta[0].hypot(delta[1]);
    if distance > radius {
        let angle = (radius / distance).asin();
        // 保留参考生成器的顺序更新方式，使入场方向与其保持一致。
        let x = delta[0] * angle.cos() - delta[1] * angle.sin();
        let y = x * angle.sin() + delta[1] * angle.cos();
        let length = x.hypot(y);
        let tangent = (distance * distance - radius * radius).sqrt();
        (
            [
                previous[0] + x / length * tangent,
                previous[1] + y / length * tangent,
            ],
            -1.0,
            true,
        )
    } else if distance > 0.0 {
        (
            [
                center[0] - delta[0] * radius / distance,
                center[1] - delta[1] * radius / distance,
            ],
            1.0,
            false,
        )
    } else {
        ([center[0], center[1] - radius], 1.0, false)
    }
}

#[cfg(test)]
mod tests {
    use super::super::context::{
        apply_standard_object_mods, build_video_render_context, standard_objects,
    };
    use super::*;
    use crate::domain::mods::{parse_mods, ModSettings};
    use crate::domain::parser::parse_beatmap_bytes;
    use crate::domain::shared::time_selection::TimeAxis;
    use crate::render::geometry::OutputFormat;

    fn context(objects: &str, mods: &ModSettings) -> RenderContext {
        let text = format!("osu file format v14\n[General]\nMode:0\n[Difficulty]\nCircleSize:4\nApproachRate:5\nOverallDifficulty:5\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n{objects}\n");
        let beatmap = parse_beatmap_bytes(text.as_bytes()).unwrap();
        let objects = apply_standard_object_mods(standard_objects(&beatmap).unwrap(), Some(mods));
        build_video_render_context(
            &beatmap,
            objects,
            Some(mods),
            TimeAxis::new(0),
            OutputFormat::Mp4,
        )
    }

    #[test]
    fn cursor_hits_circles_waits_and_supports_out_of_order_queries() {
        let mods = parse_mods(&["AT".into(), "HR".into()]).unwrap();
        let context = context("100,100,1000,1,0\n400,200,4000,1,0", &mods);
        let cursor = context.autoplay.as_ref().unwrap().get(&context);
        for index in [1, 0, 1] {
            let object = &context.hit_objects[index];
            let point = stacked_position(object, &context.settings);
            let expected = to_frame_point(point.0, point.1, &context.frame_layout);
            assert_eq!(
                cursor.position_at(object.start_time),
                Some([expected.0, expected.1])
            );
        }
        assert_eq!(cursor.position_at(2000), cursor.position_at(1000));
        assert!(cursor.position_at(-1000).is_none());
        assert!(std::ptr::eq(
            cursor,
            context.autoplay.as_ref().unwrap().get(&context)
        ));
    }

    #[test]
    fn slider_reverses_and_overlap_prioritises_new_circle() {
        let mods = parse_mods(&["AT".into()]).unwrap();
        let context = context("100,192,1000,2,0,L|400:192,2,300\n250,100,1500,1,0", &mods);
        let cursor = context.autoplay.as_ref().unwrap().get(&context);
        let circle = to_frame_point(250.0, 100.0, &context.frame_layout);
        assert_eq!(cursor.position_at(1500), Some([circle.0, circle.1]));
        let object = &context.hit_objects[0];
        let point = to_frame_point(100.0, 192.0, &context.frame_layout);
        let end = cursor.position_at(object.end_time).unwrap();
        assert!((end[0] - point.0).abs() < 0.001);
        assert!((end[1] - point.1).abs() < 0.001);
    }

    #[test]
    fn spinner_rotation_uses_real_time_and_scaled_radius() {
        let mods = parse_mods(&["AT".into(), "DT".into()]).unwrap();
        let context = context("256,192,1000,8,0,3000", &mods);
        let cursor = context.autoplay.as_ref().unwrap().get(&context);
        let center = to_frame_point(256.0, 192.0, &context.frame_layout);
        let position = cursor.position_at(2000).unwrap();
        let radius = (position[0] - center.0).hypot(position[1] - center.1);
        assert!((radius - 50.0 * context.frame_layout.scale).abs() < 0.001);
        assert!(cursor
            .frames
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time));
        assert!(cursor.frames.len() < 300);
    }

    #[test]
    fn disabled_mod_does_not_allocate_cursor_cache() {
        assert!(context("100,100,1000,1,0", &ModSettings::new())
            .autoplay
            .is_none());
    }

    #[test]
    fn trail_fades_stops_and_is_independent_of_query_order() {
        let mods = parse_mods(&["AT".into()]).unwrap();
        let context = context("100,192,1000,1,0\n400,192,2000,1,0", &mods);
        let cursor = context.autoplay.as_ref().unwrap().get(&context);
        let collect = |time| {
            let mut lines = Vec::new();
            cursor.visit_trail(
                time,
                context.frame_layout.scale,
                |from, to, width, alpha| lines.push((from, to, width, alpha)),
            );
            lines
        };
        let moving = collect(1900);
        assert!(!moving.is_empty());
        assert!(moving.iter().all(|line| line.3 <= 204 && line.2 > 0.0));
        assert!(moving.first().unwrap().3 < moving.last().unwrap().3);
        assert!(collect(2500).is_empty());
        assert_eq!(collect(1900), moving);
        assert!(collect(-1000).is_empty());
        let fl = parse_mods(&["FL".into()]).unwrap();
        let hidden = super::tests::context("100,192,1000,1,0\n400,192,2000,1,0", &fl);
        let mut count = 0;
        hidden
            .autoplay
            .as_ref()
            .unwrap()
            .get(&hidden)
            .visit_trail(1900, 1.0, |_, _, _, _| count += 1);
        assert_eq!(count, 0);
    }

    #[test]
    fn click_expands_outer_ring_and_release_contracts_it() {
        let mods = parse_mods(&["AT".into()]).unwrap();
        let context = context("100,192,1000,2,0,L|400:192,1,300", &mods);
        let cursor = context.autoplay.as_ref().unwrap().get(&context);
        assert_eq!(cursor.expansion_at(999.0), 1.0);
        assert!((cursor.expansion_at(1400.0) - 1.2).abs() < 0.00001);
        let release = context.hit_objects[0].end_time as f64 + 50.0;
        assert!((cursor.expansion_at(release + 400.0) - 1.0).abs() < 0.00001);
        let before = cursor.sprite_at(999).unwrap();
        let pressed = cursor.sprite_at(1400).unwrap();
        assert_ne!(before.data, pressed.data);
        assert_eq!(before.get(before.w / 2, before.h / 2), [255, 255, 255, 255]);
        assert_eq!(
            pressed.get(pressed.w / 2, pressed.h / 2),
            [255, 255, 255, 255]
        );
    }

    #[test]
    fn flashlight_only_shares_motion_without_allocating_sprites() {
        let fl = parse_mods(&["FL".into()]).unwrap();
        let both = parse_mods(&["FL".into(), "AT".into()]).unwrap();
        let objects = "100,192,1000,1,0\n400,192,2000,1,0";
        let hidden = context(objects, &fl);
        let shown = context(objects, &both);
        assert!(!hidden.show_cursor && shown.show_cursor);
        let hidden_cursor = hidden.autoplay.as_ref().unwrap().get(&hidden);
        assert!(hidden_cursor.sprites.is_empty());
        assert!(hidden_cursor.sprite_at(1000).is_none());
        let shown_cursor = shown.autoplay.as_ref().unwrap().get(&shown);
        for time in [1900, 1000, 1800, 5000, 1900] {
            assert_eq!(
                hidden_cursor.position_at(time),
                shown_cursor.position_at(time)
            );
            let a = hidden
                .flashlight
                .as_ref()
                .unwrap()
                .get(&hidden)
                .at(&hidden, time);
            let b = shown
                .flashlight
                .as_ref()
                .unwrap()
                .get(&shown)
                .at(&shown, time);
            assert_eq!(a.center, b.center);
        }
        let center = hidden
            .flashlight
            .as_ref()
            .unwrap()
            .get(&hidden)
            .at(&hidden, 1900)
            .center;
        let cursor = hidden_cursor.position_at(1900).unwrap();
        assert!(center[0] < cursor[0]);
    }
}
