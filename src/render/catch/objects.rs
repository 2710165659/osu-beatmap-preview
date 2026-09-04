//! osu!catch 渲染对象展开：水果、果汁流、香蕉雨、HR 偏移和 hyperdash。
//! RNG 调用顺序严格匹配 Python/stable。

use crate::common::legacy_random::{stateless_next_int, LegacyRandom};
use crate::common::slider_path::{build_catch_slider_path, path_position_at, SliderPath};
use crate::core::errors::{PreviewError, Result};
use crate::core::models::{Beatmap, CatchHitObject, TimingPoint};
use crate::core::mods::ModSettings;
use crate::parser::round_half_even;

use super::constants::*;

#[inline]
pub(crate) fn rhe(v: f64) -> i64 {
    round_half_even(v)
}

#[inline]
fn to_float32(v: f64) -> f32 {
    v as f32
}

// ─── 渲染对象 ───

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
    /// 接近 hyperdash 极限、需要引导线提示的大跨度移动。
    pub(crate) edge: bool,
    /// 所属香蕉雨编号；仅香蕉物件设置。
    pub(crate) banana_shower_id: Option<usize>,
    /// 推荐路线在该香蕉时刻采用的实际接盘中心横坐标。
    pub(crate) banana_route_x: Option<f64>,
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

// ─── 难度 ───

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

// ─── 无状态颜色 ───

pub(crate) fn banana_color(seed: i64) -> [u8; 3] {
    crate::render::catch::constants::BANANA_COLORS[stateless_next_int(3, seed, 0) as usize]
}

// ─── 对象展开 ───

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
    let mut banana_shower_ranges = Vec::new();
    let mut rng = LegacyRandom::new(crate::render::catch::constants::RNG_SEED as u32);
    let hard_rock_offsets = mods.is_some_and(|m| m.hard_rock);
    let mut last_position: Option<f64> = None;
    let mut last_start_time = 0.0f64;

    // 谱面自带 [Colours] 优先；其次统一 skin 配置；最后 lazer 默认配色
    let skin_combo_colors = &crate::config::current().skin.COMBO_COLORS;
    let combo_colors: &[[u8; 3]] = if !beatmap.combo_colors.is_empty() {
        &beatmap.combo_colors
    } else if !skin_combo_colors.is_empty() {
        skin_combo_colors
    } else {
        &crate::render::catch::constants::LAZER_COMBO_COLORS
    };

    // combo 颜色追踪：首个对象固定取第 0 组色，之后 new_combo 时前进 1 + combo_offset
    let mut color_index: usize = 0;
    let mut seen_first_combo_object = false;

    for hit_object in hit_objects.iter() {
        if hit_object.hit_type & 8 != 0 {
            // 香蕉雨：颜色由香蕉自身随机决定，不影响 combo 颜色推进
            let range = build_banana_shower_objects(
                hit_object,
                banana_shower_ranges.len(),
                &mut rng,
                &mut render_objects,
            );
            if !range.is_empty() {
                banana_shower_ranges.push(range);
            }
            continue;
        }

        if seen_first_combo_object {
            if hit_object.new_combo {
                color_index =
                    (color_index + 1 + hit_object.combo_offset as usize) % combo_colors.len();
            }
        } else {
            seen_first_combo_object = true;
        }
        let combo_color = combo_colors[color_index];

        if hit_object.hit_type & 2 != 0 {
            last_position = Some(stable_slider_end_x(hit_object));
            last_start_time = hit_object.start_time as f64;
            build_juice_stream_objects(
                hit_object,
                combo_color,
                difficulty.slider_tick_rate,
                difficulty.slider_multiplier,
                beatmap_format_version,
                &beatmap.timing_points,
                &mut rng,
                &mut render_objects,
            )?;
            continue;
        }
        let mut fruit = build_fruit_object(
            hit_object.x as f64,
            hit_object.start_time,
            combo_color,
            None,
        );
        if hard_rock_offsets {
            apply_hard_rock_fruit_offset(
                &mut fruit,
                &mut last_position,
                &mut last_start_time,
                &mut rng,
            );
        }
        render_objects.push(fruit);
    }

    highlight_banana_shower_routes(&mut render_objects, &banana_shower_ranges, difficulty.cs);
    apply_hyper_dash(&mut render_objects, difficulty.cs);
    Ok(render_objects)
}

pub(crate) fn build_fruit_object(
    x: f64,
    start_time: i64,
    combo_color: [u8; 3],
    event_time: Option<f64>,
) -> RenderObject {
    RenderObject {
        object_type: ObjType::Fruit,
        x,
        start_time,
        color: combo_color,
        scale_factor: 1.0,
        event_time,
        hyper_dash: false,
        edge: false,
        banana_shower_id: None,
        banana_route_x: None,
    }
}

fn stable_slider_end_x(hit_object: &CatchHitObject) -> f64 {
    if let Some(&(px, _)) = hit_object.slider_points.last() {
        px as f64
    } else {
        hit_object.x as f64
    }
}

fn build_banana_shower_objects(
    hit_object: &CatchHitObject,
    shower_id: usize,
    rng: &mut LegacyRandom,
    out: &mut Vec<RenderObject>,
) -> std::ops::Range<usize> {
    let start_time = hit_object.start_time;
    let end_time = hit_object.end_time;
    let mut spacing = to_float32((hit_object.end_time - hit_object.start_time) as f64);

    while spacing > 100.0 {
        spacing = to_float32(spacing as f64 / 2.0);
    }
    if spacing <= 0.0 {
        return out.len()..out.len();
    }

    let first_banana = out.len();
    let mut current_time = to_float32(start_time as f64);
    while current_time <= end_time as f32 {
        let x = rng.next_double() * crate::render::catch::constants::PLAYFIELD_WIDTH;
        rng.next();
        rng.next();
        rng.next();

        out.push(RenderObject {
            object_type: ObjType::Banana,
            x,
            start_time: rhe(current_time as f64),
            color: banana_color(current_time as i64),
            scale_factor: crate::render::catch::constants::BANANA_SCALE,
            event_time: Some(current_time as f64),
            hyper_dash: false,
            edge: false,
            banana_shower_id: Some(shower_id),
            banana_route_x: None,
        });
        current_time = to_float32(current_time as f64 + spacing as f64);
    }
    first_banana..out.len()
}

const BANANA_ROUTE_DIRECTION_COUNT: usize = 3;
const BANANA_ROUTE_STILL: usize = 0;
const BANANA_ROUTE_LEFT: usize = 1;
const BANANA_ROUTE_RIGHT: usize = 2;
const BANANA_ROUTE_SCORE_SCALE: f64 = 1_000.0;

#[derive(Clone, Copy)]
struct BananaRouteAnchor {
    x: f64,
    time: f64,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct BananaRouteScore {
    caught: usize,
    dash_time: i64,
    turns: usize,
    travel_distance: i64,
    catch_margin: i64,
}

impl BananaRouteScore {
    /// 推荐路线先保证香蕉数量，再依次降低冲刺、折返和位移，最后提高接取余量。
    fn is_better_than(self, other: Self) -> bool {
        self.caught > other.caught
            || (self.caught == other.caught
                && (self.dash_time < other.dash_time
                    || (self.dash_time == other.dash_time
                        && (self.turns < other.turns
                            || (self.turns == other.turns
                                && (self.travel_distance < other.travel_distance
                                    || (self.travel_distance == other.travel_distance
                                        && self.catch_margin > other.catch_margin)))))))
    }

    fn after_movement(
        mut self,
        from_x: f64,
        to_x: f64,
        duration: f64,
        previous_direction: usize,
    ) -> Option<(Self, usize)> {
        let distance = (to_x - from_x).abs();
        if distance > duration * BASE_DASH_SPEED + f64::EPSILON {
            return None;
        }

        let direction = if to_x < from_x {
            BANANA_ROUTE_LEFT
        } else if to_x > from_x {
            BANANA_ROUTE_RIGHT
        } else {
            previous_direction
        };
        if previous_direction != BANANA_ROUTE_STILL
            && direction != BANANA_ROUTE_STILL
            && direction != previous_direction
        {
            self.turns += 1;
        }

        // 先以普通速度移动，剩余距离才需要冲刺；该值等价于最短冲刺持续时间。
        let dash_time = ((distance - duration * BASE_WALK_SPEED).max(0.0)
            / (BASE_DASH_SPEED - BASE_WALK_SPEED))
            * BANANA_ROUTE_SCORE_SCALE;
        self.dash_time += rhe(dash_time);
        self.travel_distance += rhe(distance * BANANA_ROUTE_SCORE_SCALE);
        Some((self, direction))
    }

    fn after_catch(mut self, catcher_x: f64, banana_x: f64, half_catch_width: f64) -> Self {
        self.caught += 1;
        self.catch_margin +=
            rhe((half_catch_width - (catcher_x - banana_x).abs()) * BANANA_ROUTE_SCORE_SCALE);
        self
    }
}

#[derive(Clone, Copy)]
struct BananaRouteTrace {
    previous: u16,
    caught: bool,
}

fn banana_route_grid_step(banana_count: usize) -> usize {
    if banana_count <= 512 {
        1
    } else if banana_count <= 2_048 {
        2
    } else {
        4
    }
}

fn banana_route_state_index(position: usize, direction: usize) -> usize {
    position * BANANA_ROUTE_DIRECTION_COUNT + direction
}

fn banana_route_position(state: usize, grid_step: usize) -> f64 {
    (state / BANANA_ROUTE_DIRECTION_COUNT * grid_step) as f64
}

fn banana_route_anchor(object: &RenderObject) -> Option<BananaRouteAnchor> {
    matches!(object.object_type, ObjType::Fruit | ObjType::Droplet).then_some(BananaRouteAnchor {
        x: object.x,
        time: object.event_time_or_start(),
    })
}

fn anchor_contains(anchor: BananaRouteAnchor, x: f64, half_catch_width: f64) -> bool {
    (anchor.x - x).abs() <= half_catch_width
}

fn choose_banana_route_terminal(
    states: &[Option<BananaRouteScore>],
    last_time: f64,
    exit: Option<BananaRouteAnchor>,
    half_catch_width: f64,
    grid_step: usize,
) -> Option<usize> {
    let grid_size = PLAYFIELD_WIDTH as usize / grid_step + 1;
    let mut best: Option<(usize, BananaRouteScore)> = None;

    for (state_index, state) in states.iter().enumerate() {
        let Some(score) = state else {
            continue;
        };
        let from_x = banana_route_position(state_index, grid_step);
        let previous_direction = state_index % BANANA_ROUTE_DIRECTION_COUNT;
        let candidate = if let Some(exit) = exit {
            let duration = exit.time - last_time;
            if duration < 0.0 {
                continue;
            }
            let mut best_exit = None;
            for position in 0..grid_size {
                let to_x = (position * grid_step) as f64;
                if !anchor_contains(exit, to_x, half_catch_width) {
                    continue;
                }
                let Some((candidate, _)) =
                    score.after_movement(from_x, to_x, duration, previous_direction)
                else {
                    continue;
                };
                if best_exit.is_none_or(|current| candidate.is_better_than(current)) {
                    best_exit = Some(candidate);
                }
            }
            let Some(best_exit) = best_exit else {
                continue;
            };
            best_exit
        } else {
            *score
        };

        if best.is_none_or(|(_, current)| candidate.is_better_than(current)) {
            best = Some((state_index, candidate));
        }
    }
    best.map(|(state_index, _)| state_index)
}

/// 用接盘实际横坐标作为状态求整条香蕉雨的可执行路线。
/// 白色表示普通移动可达，粉色表示从上一个推荐香蕉到这里需要冲刺。
fn highlight_recommended_banana_route(
    bananas: &mut [RenderObject],
    circle_size: f64,
    entry: Option<BananaRouteAnchor>,
    exit: Option<BananaRouteAnchor>,
) {
    if bananas.is_empty() {
        return;
    }

    let catcher_scale = (1.0 - 0.7 * ((circle_size - 5.0) / 5.0)).abs();
    let half_catch_width = CATCHER_BASE_SIZE * catcher_scale * ALLOWED_CATCH_RANGE / 2.0;
    let grid_step = banana_route_grid_step(bananas.len());
    let grid_size = PLAYFIELD_WIDTH as usize / grid_step + 1;
    let state_count = grid_size * BANANA_ROUTE_DIRECTION_COUNT;
    let first_time = bananas[0].event_time_or_start();
    let initial_time = entry.map_or(0.0f64.min(first_time), |anchor| anchor.time);
    let initial_anchor = entry.unwrap_or(BananaRouteAnchor {
        x: PLAYFIELD_WIDTH / 2.0,
        time: initial_time,
    });
    let initial_half_width = if entry.is_some() {
        half_catch_width
    } else {
        grid_step as f64 / 2.0
    };
    let mut states = vec![None; state_count];
    for position in 0..grid_size {
        let x = (position * grid_step) as f64;
        if anchor_contains(initial_anchor, x, initial_half_width) {
            states[banana_route_state_index(position, BANANA_ROUTE_STILL)] =
                Some(BananaRouteScore::default());
        }
    }

    let mut traces = Vec::with_capacity(bananas.len());
    let mut previous_time = initial_time;
    for banana in bananas.iter() {
        let current_time = banana.event_time_or_start();
        let duration = (current_time - previous_time).max(0.0);
        let maximum_grid_distance = (duration * BASE_DASH_SPEED / grid_step as f64).ceil() as usize;
        let mut next_states = vec![None; state_count];
        let mut event_trace = vec![None; state_count];

        for (previous_index, previous_score) in states.iter().enumerate() {
            let Some(previous_score) = previous_score else {
                continue;
            };
            let previous_position = previous_index / BANANA_ROUTE_DIRECTION_COUNT;
            let previous_direction = previous_index % BANANA_ROUTE_DIRECTION_COUNT;
            let minimum_position = previous_position.saturating_sub(maximum_grid_distance);
            let maximum_position = (previous_position + maximum_grid_distance).min(grid_size - 1);

            for position in minimum_position..=maximum_position {
                let from_x = (previous_position * grid_step) as f64;
                let to_x = (position * grid_step) as f64;
                let Some((mut candidate, direction)) =
                    previous_score.after_movement(from_x, to_x, duration, previous_direction)
                else {
                    continue;
                };
                // 状态记录的是接盘中心而不是香蕉中心；只要香蕉落在盘宽内就能接取，
                // 因此可以贴左/右边缘接住并从该实际位置继续规划下一步。
                let caught = (banana.x - to_x).abs() <= half_catch_width;
                if caught {
                    candidate = candidate.after_catch(to_x, banana.x, half_catch_width);
                }
                let candidate_index = banana_route_state_index(position, direction);
                if next_states[candidate_index]
                    .is_none_or(|current| candidate.is_better_than(current))
                {
                    next_states[candidate_index] = Some(candidate);
                    event_trace[candidate_index] = Some(BananaRouteTrace {
                        previous: previous_index as u16,
                        caught,
                    });
                }
            }
        }

        states = next_states;
        traces.push(event_trace);
        previous_time = current_time;
    }

    let valid_exit = exit.filter(|anchor| anchor.time >= previous_time);
    let terminal = choose_banana_route_terminal(
        &states,
        previous_time,
        valid_exit,
        half_catch_width,
        grid_step,
    )
    .or_else(|| {
        choose_banana_route_terminal(&states, previous_time, None, half_catch_width, grid_step)
    });
    let Some(mut state_index) = terminal else {
        return;
    };

    let mut caught = vec![false; bananas.len()];
    let mut catcher_positions = vec![0.0; bananas.len()];
    for event_index in (0..bananas.len()).rev() {
        let Some(trace) = traces[event_index][state_index] else {
            return;
        };
        caught[event_index] = trace.caught;
        catcher_positions[event_index] = banana_route_position(state_index, grid_step);
        state_index = trace.previous as usize;
    }

    let mut last_x = banana_route_position(state_index, grid_step);
    let mut last_time = initial_time;
    for (index, banana) in bananas.iter_mut().enumerate() {
        banana.banana_route_x = Some(catcher_positions[index]);
        if !caught[index] {
            continue;
        }
        let current_time = banana.event_time_or_start();
        let needs_dash = (catcher_positions[index] - last_x).abs()
            > (current_time - last_time).max(0.0) * BASE_WALK_SPEED + f64::EPSILON;
        banana.color = if needs_dash {
            RECOMMENDED_DASH_BANANA_COLOR
        } else {
            RECOMMENDED_BANANA_COLOR
        };
        last_x = catcher_positions[index];
        last_time = current_time;
    }
}

fn merge_adjacent_banana_shower_ranges(
    ranges: &[std::ops::Range<usize>],
) -> Vec<std::ops::Range<usize>> {
    let mut merged: Vec<std::ops::Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(previous) = merged.last_mut() {
            if previous.end == range.start {
                previous.end = range.end;
                continue;
            }
        }
        merged.push(range.clone());
    }
    merged
}

fn highlight_banana_shower_routes(
    render_objects: &mut [RenderObject],
    ranges: &[std::ops::Range<usize>],
    circle_size: f64,
) {
    for range in merge_adjacent_banana_shower_ranges(ranges) {
        let first_time = render_objects[range.start].event_time_or_start();
        let last_time = render_objects[range.end - 1].event_time_or_start();
        let entry = render_objects[..range.start]
            .iter()
            .rev()
            .find_map(banana_route_anchor)
            .filter(|anchor| anchor.time <= first_time);
        let exit = render_objects[range.end..]
            .iter()
            .find_map(banana_route_anchor)
            .filter(|anchor| anchor.time >= last_time);
        let merged_shower_id = render_objects[range.start]
            .banana_shower_id
            .unwrap_or(range.start);
        for banana in &mut render_objects[range.clone()] {
            banana.banana_shower_id = Some(merged_shower_id);
        }
        highlight_recommended_banana_route(&mut render_objects[range], circle_size, entry, exit);
    }
}

#[allow(clippy::too_many_arguments)]
fn build_juice_stream_objects(
    hit_object: &CatchHitObject,
    combo_color: [u8; 3],
    slider_tick_rate: f64,
    slider_multiplier: f64,
    beatmap_format_version: i32,
    timing_points: &[TimingPoint],
    rng: &mut LegacyRandom,
    out: &mut Vec<RenderObject>,
) -> Result<()> {
    let slider_type = hit_object
        .slider_type
        .as_deref()
        .ok_or_else(|| PreviewError::new("catch slider is missing path type"))?;

    let path = build_catch_slider_path(
        hit_object.x,
        hit_object.y,
        &hit_object.slider_points,
        slider_type,
        hit_object.slider_pixel_length,
    );
    let events = build_slider_events(
        hit_object,
        slider_tick_rate,
        slider_multiplier,
        beatmap_format_version,
        timing_points,
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
                    object_type: ObjType::Droplet,
                    x,
                    start_time: st,
                    color: combo_color,
                    scale_factor: crate::render::catch::constants::DROPLET_SCALE,
                    event_time: Some(event.time),
                    hyper_dash: false,
                    edge: false,
                    banana_shower_id: None,
                    banana_route_x: None,
                });
            }
            EventType::LegacyLastTick => {}
            _ => {
                nested_objects.push(build_fruit_object(
                    x,
                    rhe(event.time),
                    combo_color,
                    Some(event.time),
                ));
            }
        }
        previous_event = Some(*event);
    }

    for mut obj in nested_objects {
        match obj.object_type {
            ObjType::TinyDroplet => {
                // Python：offset = rng.next(-20, 20)。
                // 等价于：int(-20 + rng.next_double() * 40)。
                let offset = (-20.0 + rng.next_double() * 40.0) as i32 as f64;
                let shifted_x = obj.x + offset;
                // 原 max/min 链在 NaN 时回退到下界，显式保留该行为。
                obj.x = if shifted_x.is_nan() {
                    0.0
                } else {
                    shifted_x.clamp(0.0, crate::render::catch::constants::PLAYFIELD_WIDTH)
                };
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
    hit_object: &CatchHitObject,
    slider_tick_rate: f64,
    slider_multiplier: f64,
    beatmap_format_version: i32,
    timing_points: &[TimingPoint],
) -> Result<Vec<SliderEvent>> {
    if slider_tick_rate <= 0.0 {
        return Err(PreviewError::new("SliderTickRate must be positive"));
    }

    let (beat_length, slider_velocity) =
        catch_resolve_slider_timing(hit_object.start_time, timing_points);
    let span_count = hit_object.slider_repeats.max(1);

    let adjusted_beat_length = precision_adjusted_beat_length(beat_length, slider_velocity);
    let velocity = 100.0 * slider_multiplier / adjusted_beat_length;

    if hit_object.slider_pixel_length <= 0.0 || velocity <= 0.0 {
        return Ok(vec![
            SliderEvent {
                event_type: EventType::Head,
                time: hit_object.start_time as f64,
                path_progress: 0.0,
            },
            SliderEvent {
                event_type: EventType::Tail,
                time: hit_object.end_time as f64,
                path_progress: if span_count % 2 == 1 { 1.0 } else { 0.0 },
            },
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
    let tick_distance = (scoring_distance / slider_tick_rate)
        .max(0.0)
        .min(total_distance);
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
            span_index,
            span_start_time,
            span_duration,
            reversed_span,
            total_distance,
            tick_distance,
            min_distance_from_end,
            &mut events,
        );

        let is_last_span = span_index == span_count - 1;
        let event_type = if is_last_span {
            EventType::Tail
        } else {
            EventType::Repeat
        };
        let path_progress = if span_index % 2 == 0 { 1.0 } else { 0.0 };

        events.push(SliderEvent {
            event_type,
            time: span_start_time + span_duration,
            path_progress,
        });
    }

    // 无论格式版本如何，始终生成旧版末尾 tick。
    if let Some(legacy_tick) =
        build_legacy_last_tick(hit_object.start_time, span_duration, span_count)
    {
        events.push(legacy_tick);
    }

    events.sort_by(|a, b| {
        a.time
            .partial_cmp(&b.time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(events)
}

fn generate_span_ticks(
    _span_index: i32,
    span_start_time: f64,
    span_duration: f64,
    reversed_span: bool,
    total_distance: f64,
    tick_distance: f64,
    min_distance_from_end: f64,
    events: &mut Vec<SliderEvent>,
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
        let time_progress = if reversed_span {
            1.0 - path_progress
        } else {
            path_progress
        };

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
    start_time: i64,
    span_duration: f64,
    span_count: i32,
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
    let raw_multiplier = to_float32(100.0 / slider_velocity);
    // 原 max/min 链在 NaN 时回退到下界 10.0。
    let bpm_multiplier = if raw_multiplier.is_nan() {
        10.0
    } else {
        raw_multiplier.clamp(10.0, 1000.0)
    } / 100.0;
    beat_length * bpm_multiplier as f64
}

fn build_tiny_droplets_between(
    path: &SliderPath,
    prev: &SliderEvent,
    next: &SliderEvent,
    combo_color: [u8; 3],
    out: &mut Vec<RenderObject>,
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
            object_type: ObjType::TinyDroplet,
            x,
            start_time: rhe(time),
            color: combo_color,
            scale_factor: crate::render::catch::constants::TINY_DROPLET_SCALE,
            event_time: Some(time),
            hyper_dash: false,
            edge: false,
            banana_shower_id: None,
            banana_route_x: None,
        });
        offset += time_between_tiny;
    }
}

fn apply_hard_rock_fruit_offset(
    fruit: &mut RenderObject,
    last_position: &mut Option<f64>,
    last_start_time: &mut f64,
    rng: &mut LegacyRandom,
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
    (position + offset).clamp(0.0, crate::render::catch::constants::PLAYFIELD_WIDTH)
}

fn apply_offset(position: f64, amount: f64) -> f64 {
    (position + amount).min(crate::render::catch::constants::PLAYFIELD_WIDTH)
}

fn apply_hyper_dash(render_objects: &mut [RenderObject], circle_size: f64) {
    let catcher_width =
        crate::render::catch::constants::CATCHER_BASE_SIZE * circle_scale(circle_size);
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
            - if last_direction == direction {
                last_excess
            } else {
                half_catcher_width
            };
        let distance_to_hyper = time_to_next - distance_to_next;

        if distance_to_hyper < 0.0 {
            render_objects[current_index].hyper_dash = true;
            last_excess = half_catcher_width;
        } else {
            last_excess = distance_to_hyper.min(half_catcher_width).max(0.0);
            // 与旧版 osubot Catch 预览一致：距离 hyperdash 阈值不足 20px 的
            // 大跨度移动标为 edge，供静态图用白线连接前后两个可接物件。
            if distance_to_next > 2.0 * half_catcher_width && distance_to_hyper < 20.0 {
                render_objects[current_index].edge = true;
            }
        }

        last_direction = direction;
    }
}

// ─── 滑条时间 ───

fn catch_resolve_slider_timing(start_time: i64, timing_points: &[TimingPoint]) -> (f64, f64) {
    let mut beat_length = DEFAULT_BEAT_LENGTH;
    let mut slider_velocity = 1.0;

    for point in timing_points {
        if point.time > 0.0 {
            break;
        }
        apply_timing_state(point, &mut beat_length, &mut slider_velocity);
    }
    for point in timing_points {
        if point.time > start_time as f64 {
            break;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fruit(x: f64, time: i64) -> RenderObject {
        RenderObject {
            object_type: ObjType::Fruit,
            x,
            start_time: time,
            color: LAZER_COMBO_COLORS[0],
            scale_factor: 1.0,
            event_time: Some(time as f64),
            hyper_dash: false,
            edge: false,
            banana_shower_id: None,
            banana_route_x: None,
        }
    }

    fn banana(x: f64, time: i64) -> RenderObject {
        RenderObject {
            object_type: ObjType::Banana,
            x,
            start_time: time,
            color: BANANA_COLORS[0],
            scale_factor: BANANA_SCALE,
            event_time: Some(time as f64),
            hyper_dash: false,
            edge: false,
            banana_shower_id: None,
            banana_route_x: None,
        }
    }

    fn anchor(x: f64, time: i64) -> BananaRouteAnchor {
        BananaRouteAnchor {
            x,
            time: time as f64,
        }
    }

    #[test]
    fn recommended_banana_route_skips_unreachable_decoys() {
        let mut bananas = vec![banana(0.0, 0), banana(512.0, 100), banana(20.0, 200)];

        highlight_recommended_banana_route(&mut bananas, 5.0, Some(anchor(0.0, 0)), None);

        assert_eq!(bananas[0].color, RECOMMENDED_BANANA_COLOR);
        assert_eq!(bananas[1].color, BANANA_COLORS[0]);
        assert_eq!(bananas[2].color, RECOMMENDED_BANANA_COLOR);
    }

    #[test]
    fn recommended_banana_route_marks_dash_transitions_pink() {
        let mut bananas = vec![banana(0.0, 0), banana(140.0, 100)];

        highlight_recommended_banana_route(&mut bananas, 5.0, Some(anchor(0.0, 0)), None);

        assert_eq!(bananas[0].color, RECOMMENDED_BANANA_COLOR);
        assert_eq!(bananas[1].color, RECOMMENDED_DASH_BANANA_COLOR);
    }

    #[test]
    fn recommended_banana_route_uses_catcher_edges_without_moving() {
        // 两颗香蕉中心相距 80px，但 CS5 接取区间有重叠，可以让接盘停在约 140px。
        let mut bananas = vec![banana(100.0, 0), banana(180.0, 20)];

        highlight_recommended_banana_route(&mut bananas, 5.0, Some(anchor(100.0, 0)), None);

        assert_eq!(bananas[0].color, RECOMMENDED_BANANA_COLOR);
        assert_eq!(bananas[1].color, RECOMMENDED_BANANA_COLOR);
        let first_x = bananas[0].banana_route_x.unwrap();
        let second_x = bananas[1].banana_route_x.unwrap();
        assert_eq!(first_x, second_x);
        assert!((137.3..=142.7).contains(&first_x));
    }

    #[test]
    fn recommended_banana_route_is_globally_reachable() {
        // 相邻两段分别贴边可达，但中间香蕉无法同时站在左右两个接取边缘。
        let mut bananas = vec![banana(0.0, 0), banana(178.0, 100), banana(356.0, 200)];

        highlight_recommended_banana_route(&mut bananas, 5.0, None, None);

        let selected = bananas
            .iter()
            .filter(|banana| {
                matches!(
                    banana.color,
                    RECOMMENDED_BANANA_COLOR | RECOMMENDED_DASH_BANANA_COLOR
                )
            })
            .count();
        assert_eq!(selected, 2);
    }

    #[test]
    fn recommended_banana_route_reserves_position_for_exit_fruit() {
        let mut bananas = vec![banana(150.0, 100), banana(362.0, 100)];

        highlight_recommended_banana_route(
            &mut bananas,
            5.0,
            Some(anchor(256.0, 0)),
            Some(anchor(512.0, 200)),
        );

        assert_eq!(bananas[0].color, BANANA_COLORS[0]);
        assert!(matches!(
            bananas[1].color,
            RECOMMENDED_BANANA_COLOR | RECOMMENDED_DASH_BANANA_COLOR
        ));
    }

    #[test]
    fn consecutive_single_banana_showers_share_one_reachable_route() {
        let mut first = banana(0.0, 1_000);
        first.banana_shower_id = Some(0);
        let mut second = banana(512.0, 1_000);
        second.banana_shower_id = Some(1);
        let mut objects = vec![first, second];

        highlight_banana_shower_routes(&mut objects, &[0..1, 1..2], 5.0);

        let first_x = objects[0].banana_route_x.unwrap();
        let second_x = objects[1].banana_route_x.unwrap();
        assert_eq!(first_x, second_x);
        assert_eq!(objects[0].banana_shower_id, objects[1].banana_shower_id);
    }

    #[test]
    fn large_near_hyper_transition_is_marked_as_edge() {
        let mut fruits = vec![fruit(0.0, 0), fruit(200.0, 190)];

        apply_hyper_dash(&mut fruits, 5.0);

        assert!(!fruits[0].hyper_dash);
        assert!(fruits[0].edge);
    }
}
