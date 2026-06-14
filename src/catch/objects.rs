//! osu!catch render-object expansion: fruits, juice streams, banana showers,
//! HR offsets, hyperdash. RNG call order mirrors Python/stable exactly.

use crate::errors::{PreviewError, Result};
use crate::legacy_random::{stateless_next_int, LegacyRandom};
use crate::models::{Beatmap, CatchHitObject, TimingPoint};
use crate::mods::ModSettings;
use crate::parser::round_half_even;
use crate::slider_path::{build_catch_slider_path, path_position_at, SliderPath};

use super::constants::*;

#[inline]
pub(crate) fn rhe(v: f64) -> i64 {
    round_half_even(v)
}

#[inline]
fn to_float32(v: f64) -> f32 {
    v as f32
}

// ─── render objects ───

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ObjType {
    TinyDroplet,
    Droplet,
    Fruit,
    Banana,
}

pub(crate) fn object_order(t: ObjType) -> i64 {
    match t {
        ObjType::TinyDroplet => 0,
        ObjType::Droplet => 1,
        ObjType::Fruit => 2,
        ObjType::Banana => 3,
    }
}

#[derive(Clone)]
pub(crate) struct RenderObject {
    pub(crate) object_type: ObjType,
    pub(crate) x: f64,
    pub(crate) start_time: i64,
    pub(crate) color: [u8; 3],
    pub(crate) scale_factor: f64,
    pub(crate) event_time: Option<f64>,
    pub(crate) hyper_dash: bool,
}

impl RenderObject {
    pub(crate) fn event_time_or_start(&self) -> f64 {
        self.event_time.unwrap_or(self.start_time as f64)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum EventType {
    Head,
    Tick,
    Repeat,
    Tail,
    LegacyLastTick,
}

#[derive(Clone, Copy)]
pub(crate) struct SliderEvent {
    pub(crate) event_type: EventType,
    pub(crate) time: f64,
    pub(crate) path_progress: f64,
}

// ─── difficulty ───

pub(crate) struct Difficulty {
    pub(crate) cs: f64,
    pub(crate) ar: f64,
    pub(crate) slider_multiplier: f64,
    pub(crate) slider_tick_rate: f64,
}

pub(crate) fn effective_difficulty(beatmap: &Beatmap, mods: Option<&ModSettings>) -> Difficulty {
    let d = &beatmap.difficulty;
    let od = d.get_f64_or("OverallDifficulty", 5.0);
    let mut diff = Difficulty {
        cs: d.get_f64_or("CircleSize", 5.0),
        ar: d.get_f64("ApproachRate").unwrap_or(od),
        slider_multiplier: d.get_f64_or("SliderMultiplier", 1.4),
        slider_tick_rate: d.get_f64_or("SliderTickRate", 1.0),
    };
    if let Some(m) = mods {
        if m.easy {
            diff.cs *= 0.5;
            diff.ar *= 0.5;
        }
        if m.hard_rock {
            diff.cs = (diff.cs * 1.3).min(10.0);
            diff.ar = (diff.ar * 1.4).min(10.0);
        }
    }
    diff
}

pub(crate) fn circle_scale(circle_size: f64) -> f64 {
    (1.0 - 0.7 * ((circle_size - 5.0) / 5.0)) / 2.0
}

pub(crate) fn difficulty_range(difficulty: f64, minimum: f64, middle: f64, maximum: f64) -> f64 {
    let scaled = (difficulty - 5.0) / 5.0;
    if difficulty > 5.0 {
        middle + (maximum - middle) * scaled
    } else if difficulty < 5.0 {
        middle + (middle - minimum) * scaled
    } else {
        middle
    }
}

pub(crate) fn catch_time_range(approach_rate: f64) -> f64 {
    difficulty_range(approach_rate, 1800.0, 1200.0, 450.0)
}

// ─── stateless colors ───

pub(crate) fn banana_color(seed: i64) -> [u8; 3] {
    BANANA_COLORS[stateless_next_int(3, seed, 0) as usize]
}

// ─── object expansion ───

/// 将谱面 hit object 展开为渲染对象（水果 / 果汁流 / 香蕉雨）。
///
/// combo 颜色按 new_combo 标志推进（与游戏一致），而不是按对象序号轮换；
/// 香蕉雨不参与 combo 计数。
pub(crate) fn build_catch_render_objects(
    beatmap: &Beatmap,
    hit_objects: &[CatchHitObject],
    mods: Option<&ModSettings>,
    difficulty: &Difficulty,
) -> Result<Vec<RenderObject>> {
    let beatmap_format_version = beatmap.format_version();
    let mut render_objects: Vec<RenderObject> = Vec::new();
    let mut rng = LegacyRandom::new(RNG_SEED);
    let hard_rock_offsets = mods.is_some_and(|m| m.hard_rock);
    let mut last_position: Option<f64> = None;
    let mut last_start_time = 0.0f64;

    // 谱面自带 [Colours] 优先；其次统一 skin.ini；最后 lazer 默认配色
    let skin_combo_colors = &crate::skin::skin().combo_colors;
    let combo_colors: &[[u8; 3]] = if !beatmap.combo_colors.is_empty() {
        &beatmap.combo_colors
    } else if !skin_combo_colors.is_empty() {
        skin_combo_colors
    } else {
        &LAZER_COMBO_COLORS
    };

    // combo 颜色追踪：首个对象固定取第 0 组色，之后 new_combo 时前进 1 + combo_offset
    let mut color_index: usize = 0;
    let mut seen_first_combo_object = false;

    for hit_object in hit_objects.iter() {
        if hit_object.hit_type & 8 != 0 {
            // 香蕉雨：颜色由香蕉自身随机决定，不影响 combo 颜色推进
            build_banana_shower_objects(hit_object, &mut rng, &mut render_objects);
            continue;
        }

        if seen_first_combo_object {
            if hit_object.new_combo {
                color_index = (color_index + 1 + hit_object.combo_offset as usize)
                    % combo_colors.len();
            }
        } else {
            seen_first_combo_object = true;
        }
        let combo_color = combo_colors[color_index];

        if hit_object.hit_type & 2 != 0 {
            last_position = Some(stable_slider_end_x(hit_object));
            last_start_time = hit_object.start_time as f64;
            build_juice_stream_objects(
                hit_object, combo_color, difficulty.slider_tick_rate,
                difficulty.slider_multiplier, beatmap_format_version,
                &beatmap.timing_points, &mut rng, &mut render_objects,
            )?;
            continue;
        }
        let mut fruit =
            build_fruit_object(hit_object.x as f64, hit_object.start_time, combo_color, None);
        if hard_rock_offsets {
            apply_hard_rock_fruit_offset(
                &mut fruit, &mut last_position, &mut last_start_time, &mut rng,
            );
        }
        render_objects.push(fruit);
    }

    apply_hyper_dash(&mut render_objects, difficulty.cs);
    Ok(render_objects)
}

pub(crate) fn build_fruit_object(
    x: f64, start_time: i64, combo_color: [u8; 3], event_time: Option<f64>,
) -> RenderObject {
    RenderObject { object_type: ObjType::Fruit, x, start_time, color: combo_color, scale_factor: 1.0, event_time, hyper_dash: false }
}

fn stable_slider_end_x(hit_object: &CatchHitObject) -> f64 {
    if let Some(&(px, _)) = hit_object.slider_points.last() {
        px as f64
    } else {
        hit_object.x as f64
    }
}

fn build_banana_shower_objects(
    hit_object: &CatchHitObject, rng: &mut LegacyRandom, out: &mut Vec<RenderObject>,
) {
    let start_time = hit_object.start_time;
    let end_time = hit_object.end_time;
    let mut spacing = to_float32((hit_object.end_time - hit_object.start_time) as f64);

    while spacing > 100.0 {
        spacing = to_float32(spacing as f64 / 2.0);
    }
    if spacing <= 0.0 {
        return;
    }

    let mut current_time = to_float32(start_time as f64);
    while current_time <= end_time as f32 {
        let x = rng.next_double() * PLAYFIELD_WIDTH;
        rng.next();
        rng.next();
        rng.next();

        out.push(RenderObject {
            object_type: ObjType::Banana, x, start_time: rhe(current_time as f64),
            color: banana_color(current_time as i64), scale_factor: BANANA_SCALE,
            event_time: Some(current_time as f64), hyper_dash: false,
        });
        current_time = to_float32(current_time as f64 + spacing as f64);
    }
}

#[allow(clippy::too_many_arguments)]
fn build_juice_stream_objects(
    hit_object: &CatchHitObject, combo_color: [u8; 3], slider_tick_rate: f64,
    slider_multiplier: f64, beatmap_format_version: i32, timing_points: &[TimingPoint],
    rng: &mut LegacyRandom, out: &mut Vec<RenderObject>,
) -> Result<()> {
    let slider_type = hit_object.slider_type.as_deref()
        .ok_or_else(|| PreviewError::new("catch slider is missing path type"))?;

    let path = build_catch_slider_path(
        hit_object.x, hit_object.y, &hit_object.slider_points, slider_type,
        hit_object.slider_pixel_length,
    );
    let events = build_slider_events(
        hit_object, slider_tick_rate, slider_multiplier, beatmap_format_version, timing_points,
    )?;

    let mut nested_objects: Vec<RenderObject> = Vec::new();
    let mut previous_event: Option<SliderEvent> = None;

    for event in &events {
        if let Some(prev) = previous_event {
            build_tiny_droplets_between(&path, &prev, event, combo_color, &mut nested_objects);
        }

        let x = path_position_at(&path, event.path_progress).0;
        match event.event_type {
            EventType::Tick => {
                let st = rhe(event.time);
                nested_objects.push(RenderObject {
                    object_type: ObjType::Droplet, x, start_time: st, color: combo_color,
                    scale_factor: DROPLET_SCALE, event_time: Some(event.time), hyper_dash: false,
                });
            }
            EventType::LegacyLastTick => {}
            _ => {
                nested_objects.push(build_fruit_object(x, rhe(event.time), combo_color, Some(event.time)));
            }
        }
        previous_event = Some(*event);
    }

    for mut obj in nested_objects {
        match obj.object_type {
            ObjType::TinyDroplet => {
                // Python: offset = rng.next(-20, 20)
                // which is: int(-20 + rng.next_double() * 40)
                let offset = (-20.0 + rng.next_double() * 40.0) as i32 as f64;
                obj.x = (obj.x + offset).max(0.0).min(PLAYFIELD_WIDTH);
            }
            ObjType::Droplet => {
                rng.next();
            }
            _ => {}
        }
        out.push(obj);
    }
    Ok(())
}

fn build_slider_events(
    hit_object: &CatchHitObject, slider_tick_rate: f64, slider_multiplier: f64,
    beatmap_format_version: i32, timing_points: &[TimingPoint],
) -> Result<Vec<SliderEvent>> {
    if slider_tick_rate <= 0.0 {
        return Err(PreviewError::new("SliderTickRate must be positive"));
    }

    let (beat_length, slider_velocity) = catch_resolve_slider_timing(hit_object.start_time, timing_points);
    let span_count = hit_object.slider_repeats.max(1);

    let adjusted_beat_length = precision_adjusted_beat_length(beat_length, slider_velocity);
    let velocity = 100.0 * slider_multiplier / adjusted_beat_length;

    if hit_object.slider_pixel_length <= 0.0 || velocity <= 0.0 {
        return Ok(vec![
            SliderEvent { event_type: EventType::Head, time: hit_object.start_time as f64, path_progress: 0.0 },
            SliderEvent { event_type: EventType::Tail, time: hit_object.end_time as f64, path_progress: if span_count % 2 == 1 { 1.0 } else { 0.0 } },
        ]);
    }

    let span_duration = hit_object.slider_pixel_length / velocity;
    let scoring_distance = velocity * beat_length;
    let scoring_distance = if beatmap_format_version < 8 {
        scoring_distance / slider_velocity
    } else {
        scoring_distance
    };
    let total_distance = hit_object.slider_pixel_length.min(100000.0);
    let tick_distance = (scoring_distance / slider_tick_rate).max(0.0).min(total_distance);
    let min_distance_from_end = velocity * 10.0;

    let mut events: Vec<SliderEvent> = Vec::new();
    events.push(SliderEvent {
        event_type: EventType::Head,
        time: hit_object.start_time as f64,
        path_progress: 0.0,
    });

    for span_index in 0..span_count {
        let span_start_time = hit_object.start_time as f64 + span_index as f64 * span_duration;
        let reversed_span = span_index % 2 == 1;

        generate_span_ticks(
            span_index, span_start_time, span_duration, reversed_span,
            total_distance, tick_distance, min_distance_from_end, &mut events,
        );

        let is_last_span = span_index == span_count - 1;
        let event_type = if is_last_span { EventType::Tail } else { EventType::Repeat };
        let path_progress = if span_index % 2 == 0 { 1.0 } else { 0.0 };

        events.push(SliderEvent {
            event_type,
            time: span_start_time + span_duration,
            path_progress,
        });
    }

    // Always generate legacy last tick, regardless of format version
    if let Some(legacy_tick) = build_legacy_last_tick(
        hit_object.start_time as i64, span_duration, span_count,
    ) {
        events.push(legacy_tick);
    }

    events.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
    Ok(events)
}

fn generate_span_ticks(
    _span_index: i32, span_start_time: f64, span_duration: f64,
    reversed_span: bool, total_distance: f64, tick_distance: f64,
    min_distance_from_end: f64, events: &mut Vec<SliderEvent>,
) {
    if tick_distance <= 0.0 {
        return;
    }

    let mut ticks: Vec<SliderEvent> = Vec::new();
    let mut distance = tick_distance;

    while distance <= total_distance + 0.001 {
        if distance >= total_distance - min_distance_from_end {
            break;
        }

        let path_progress = distance / total_distance;
        let time_progress = if reversed_span { 1.0 - path_progress } else { path_progress };

        ticks.push(SliderEvent {
            event_type: EventType::Tick,
            time: span_start_time + time_progress * span_duration,
            path_progress,
        });
        distance += tick_distance;
    }

    if reversed_span {
        ticks.reverse();
    }

    events.extend(ticks);
}

fn build_legacy_last_tick(
    start_time: i64, span_duration: f64, span_count: i32,
) -> Option<SliderEvent> {
    if span_count <= 0 {
        return None;
    }

    let total_duration = span_count as f64 * span_duration;
    let final_span_index = span_count - 1;
    let final_span_start_time = start_time as f64 + final_span_index as f64 * span_duration;
    let legacy_last_tick_time = (start_time as f64 + total_duration / 2.0)
        .max(final_span_start_time + span_duration - 36.0);

    let mut path_progress = (legacy_last_tick_time - final_span_start_time) / span_duration;
    if span_count % 2 == 0 {
        path_progress = 1.0 - path_progress;
    }

    Some(SliderEvent {
        event_type: EventType::LegacyLastTick,
        time: legacy_last_tick_time,
        path_progress,
    })
}

fn precision_adjusted_beat_length(beat_length: f64, slider_velocity: f64) -> f64 {
    if slider_velocity <= 0.0 {
        return beat_length;
    }
    let bpm_multiplier = to_float32(100.0 / slider_velocity).max(10.0).min(1000.0) / 100.0;
    beat_length * bpm_multiplier as f64
}

fn build_tiny_droplets_between(
    path: &SliderPath, prev: &SliderEvent, next: &SliderEvent,
    combo_color: [u8; 3], out: &mut Vec<RenderObject>,
) {
    let since_last_event = next.time as i64 - prev.time as i64;
    if since_last_event <= 80 {
        return;
    }

    let mut time_between_tiny = since_last_event as f64;
    while time_between_tiny > 100.0 {
        time_between_tiny /= 2.0;
    }

    let mut offset = time_between_tiny;
    while offset < since_last_event as f64 - 0.001 {
        let ratio = offset / since_last_event as f64;
        let progress = prev.path_progress + (next.path_progress - prev.path_progress) * ratio;
        let x = path_position_at(path, progress).0;
        let time = prev.time + offset;
        out.push(RenderObject {
            object_type: ObjType::TinyDroplet, x, start_time: rhe(time), color: combo_color,
            scale_factor: TINY_DROPLET_SCALE, event_time: Some(time), hyper_dash: false,
        });
        offset += time_between_tiny;
    }
}

fn apply_hard_rock_fruit_offset(
    fruit: &mut RenderObject, last_position: &mut Option<f64>,
    last_start_time: &mut f64, rng: &mut LegacyRandom,
) {
    let time_diff = fruit.start_time as f64 - *last_start_time;
    if time_diff < 500.0 && last_position.is_some() {
        let offset = if time_diff < 250.0 { 22.0 } else { 0.0 };
        fruit.x = apply_offset(fruit.x, offset);
    } else {
        fruit.x = apply_random_offset(fruit.x, 20.0, rng);
    }
    *last_position = Some(fruit.x);
    *last_start_time = fruit.start_time as f64;
}

fn apply_random_offset(position: f64, max_offset: f64, rng: &mut LegacyRandom) -> f64 {
    let offset = rng.next_double() * max_offset * 2.0 - max_offset;
    (position + offset).clamp(0.0, PLAYFIELD_WIDTH)
}

fn apply_offset(position: f64, amount: f64) -> f64 {
    (position + amount).min(PLAYFIELD_WIDTH)
}

fn apply_hyper_dash(render_objects: &mut [RenderObject], circle_size: f64) {
    let catcher_width = CATCHER_BASE_SIZE * circle_scale(circle_size);
    let half_catcher_width = catcher_width / 2.0;
    let mut last_direction = 0i32;
    let mut last_excess = half_catcher_width;

    for current_index in 0..render_objects.len().saturating_sub(1) {
        if render_objects[current_index].object_type == ObjType::Banana
            || render_objects[current_index].object_type == ObjType::TinyDroplet
        {
            continue;
        }
        let mut next_index = current_index + 1;
        while next_index < render_objects.len()
            && matches!(
                render_objects[next_index].object_type,
                ObjType::Banana | ObjType::TinyDroplet
            )
        {
            next_index += 1;
        }
        if next_index >= render_objects.len() {
            break;
        }

        let current_x = render_objects[current_index].x;
        let next_x = render_objects[next_index].x;
        let direction = if next_x > current_x { 1 } else { -1 };
        let time_to_next = render_objects[next_index].event_time_or_start().trunc()
            - render_objects[current_index].event_time_or_start().trunc()
            - 1000.0 / 60.0 / 4.0;
        let distance_to_next = (next_x - current_x).abs()
            - if last_direction == direction { last_excess } else { half_catcher_width };
        let distance_to_hyper = time_to_next - distance_to_next;

        if distance_to_hyper < 0.0 {
            render_objects[current_index].hyper_dash = true;
            last_excess = half_catcher_width;
        } else {
            last_excess = distance_to_hyper.min(half_catcher_width).max(0.0);
        }

        last_direction = direction;
    }
}

// ─── slider timing ───

fn catch_resolve_slider_timing(start_time: i64, timing_points: &[TimingPoint]) -> (f64, f64) {
    let mut beat_length = DEFAULT_BEAT_LENGTH;
    let mut slider_velocity = 1.0;

    for point in timing_points {
        if point.time > 0.0 { break; }
        apply_timing_state(point, &mut beat_length, &mut slider_velocity);
    }
    for point in timing_points {
        if point.time > start_time as f64 { break; }
        apply_timing_state(point, &mut beat_length, &mut slider_velocity);
    }
    (beat_length, slider_velocity)
}

fn apply_timing_state(point: &TimingPoint, beat_length: &mut f64, slider_velocity: &mut f64) {
    if point.uninherited {
        *beat_length = point.beat_length;
    } else if point.beat_length >= 0.0 {
        *slider_velocity = 1.0;
    } else {
        *slider_velocity = -100.0 / point.beat_length;
    }
}
