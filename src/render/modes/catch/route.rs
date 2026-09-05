//! osu!catch 香蕉雨推荐路线规划。

use super::constants::*;
use super::objects::{rhe, ObjType, RenderObject};

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

    // 回溯完成后不再需要整段路线的状态和轨迹，及时释放大数组，避免长香蕉雨占用内存到整张图渲染结束。
    drop(traces);
    drop(states);

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

pub(super) fn highlight_banana_shower_routes(
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
