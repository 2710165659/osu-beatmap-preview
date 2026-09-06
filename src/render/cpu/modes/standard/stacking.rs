//! osu!standard hit object stacking, ported from `OsuBeatmapProcessor`.

use crate::domain::models::StandardHitObject;
use crate::domain::shared::slider_path::build_standard_slider_path;

const STACK_DISTANCE_SQUARED: f32 = 3.0 * 3.0;

pub(crate) fn calculate_preempt(approach_rate: f32) -> i64 {
    let approach_rate = approach_rate as f64;
    (if approach_rate > 5.0 {
        1200.0 + (450.0 - 1200.0) * ((approach_rate - 5.0) / 5.0)
    } else if approach_rate < 5.0 {
        1200.0 + (1200.0 - 1800.0) * ((approach_rate - 5.0) / 5.0)
    } else {
        1200.0
    }) as i64
}

#[cfg(test)]
pub(crate) fn calculate_stack_threshold(approach_rate: f32, stack_leniency: f32) -> f32 {
    calculate_preempt(approach_rate) as f32 * stack_leniency
}

pub(crate) fn apply_stacking(
    hit_objects: &mut [StandardHitObject],
    beatmap_version: i32,
    stack_threshold: f32,
) {
    for object in hit_objects.iter_mut() {
        object.stack_height = 0;
    }

    if hit_objects.len() < 2 {
        return;
    }

    let end_positions: Vec<(f64, f64)> = hit_objects.iter().map(end_position).collect();
    if beatmap_version >= 6 {
        apply_stacking_modern(hit_objects, &end_positions, stack_threshold);
    } else {
        apply_stacking_old(hit_objects, &end_positions, stack_threshold);
    }
}

fn apply_stacking_modern(
    hit_objects: &mut [StandardHitObject],
    end_positions: &[(f64, f64)],
    stack_threshold: f32,
) {
    for i in (1..hit_objects.len()).rev() {
        if hit_objects[i].stack_height != 0 || is_spinner(&hit_objects[i]) {
            continue;
        }

        let mut object_i = i;
        if is_hit_circle(&hit_objects[i]) {
            let mut n = i;
            while n > 0 {
                n -= 1;
                if is_spinner(&hit_objects[n]) {
                    continue;
                }

                if (hit_objects[object_i].start_time - hit_objects[n].end_time) as f32
                    > stack_threshold
                {
                    break;
                }

                if is_slider(&hit_objects[n])
                    && positions_overlap(end_positions[n], position(&hit_objects[object_i]))
                {
                    let offset =
                        hit_objects[object_i].stack_height - hit_objects[n].stack_height + 1;
                    for object_j in hit_objects.iter_mut().take(i + 1).skip(n + 1) {
                        if positions_overlap(end_positions[n], position(object_j)) {
                            object_j.stack_height -= offset;
                        }
                    }
                    break;
                }

                if positions_overlap(position(&hit_objects[n]), position(&hit_objects[object_i])) {
                    hit_objects[n].stack_height = hit_objects[object_i].stack_height + 1;
                    object_i = n;
                }
            }
        } else if is_slider(&hit_objects[i]) {
            let mut n = i;
            while n > 0 {
                n -= 1;
                if is_spinner(&hit_objects[n]) {
                    continue;
                }

                if (hit_objects[object_i].start_time - hit_objects[n].start_time) as f32
                    > stack_threshold
                {
                    break;
                }

                if positions_overlap(end_positions[n], position(&hit_objects[object_i])) {
                    hit_objects[n].stack_height = hit_objects[object_i].stack_height + 1;
                    object_i = n;
                }
            }
        }
    }
}

fn apply_stacking_old(
    hit_objects: &mut [StandardHitObject],
    end_positions: &[(f64, f64)],
    stack_threshold: f32,
) {
    for i in 0..hit_objects.len() {
        if hit_objects[i].stack_height != 0 && !is_slider(&hit_objects[i]) {
            continue;
        }

        let mut start_time = hit_objects[i].end_time;
        let mut slider_stack = 0;
        let position_i = position(&hit_objects[i]);
        let position_2 = if is_slider(&hit_objects[i]) {
            end_positions[i]
        } else {
            position_i
        };

        for j in i + 1..hit_objects.len() {
            if hit_objects[j].start_time as f64 - stack_threshold as f64 > start_time as f64 {
                break;
            }

            if positions_overlap(position(&hit_objects[j]), position_i) {
                hit_objects[i].stack_height += 1;
                start_time = hit_objects[j].start_time;
            } else if positions_overlap(position(&hit_objects[j]), position_2) {
                slider_stack += 1;
                hit_objects[j].stack_height -= slider_stack;
                start_time = hit_objects[j].start_time;
            }
        }
    }
}

fn position(hit_object: &StandardHitObject) -> (f64, f64) {
    (hit_object.x as f64, hit_object.y as f64)
}

fn end_position(hit_object: &StandardHitObject) -> (f64, f64) {
    if !is_slider(hit_object) {
        return position(hit_object);
    }

    let path = build_standard_slider_path(
        hit_object.x,
        hit_object.y,
        &hit_object.slider_points,
        hit_object.slider_type.as_deref().unwrap_or("B"),
        hit_object.slider_pixel_length,
    );
    path.points
        .last()
        .copied()
        .unwrap_or_else(|| position(hit_object))
}

fn positions_overlap(a: (f64, f64), b: (f64, f64)) -> bool {
    let dx = a.0 as f32 - b.0 as f32;
    let dy = a.1 as f32 - b.1 as f32;
    dx * dx + dy * dy < STACK_DISTANCE_SQUARED
}

fn is_hit_circle(hit_object: &StandardHitObject) -> bool {
    !is_slider(hit_object) && !is_spinner(hit_object)
}

fn is_slider(hit_object: &StandardHitObject) -> bool {
    hit_object.hit_type & 2 != 0
}

fn is_spinner(hit_object: &StandardHitObject) -> bool {
    hit_object.hit_type & 8 != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circle(x: i32, y: i32, start_time: i64) -> StandardHitObject {
        StandardHitObject {
            x,
            y,
            start_time,
            end_time: start_time,
            hit_type: 1,
            ..Default::default()
        }
    }

    fn slider(
        x: i32,
        y: i32,
        end_x: i32,
        end_y: i32,
        start_time: i64,
        end_time: i64,
    ) -> StandardHitObject {
        StandardHitObject {
            x,
            y,
            start_time,
            end_time,
            hit_type: 2,
            slider_type: Some("L".to_string()),
            slider_points: vec![(end_x, end_y)],
            slider_pixel_length: (((end_x - x).pow(2) + (end_y - y).pow(2)) as f64).sqrt(),
            ..Default::default()
        }
    }

    #[test]
    fn modern_stacks_circle_chain_backwards() {
        let mut objects = vec![
            circle(100, 100, 1000),
            circle(100, 100, 1050),
            circle(100, 100, 1100),
        ];
        apply_stacking(&mut objects, 14, 100.0);
        assert_eq!(
            objects.iter().map(|o| o.stack_height).collect::<Vec<_>>(),
            vec![2, 1, 0]
        );
    }

    #[test]
    fn threshold_and_distance_boundaries_are_strict() {
        let mut at_threshold = vec![circle(100, 100, 1000), circle(100, 100, 1100)];
        apply_stacking(&mut at_threshold, 14, 100.0);
        assert_eq!(at_threshold[0].stack_height, 1);

        let mut past_threshold = vec![circle(100, 100, 1000), circle(100, 100, 1101)];
        apply_stacking(&mut past_threshold, 14, 100.0);
        assert_eq!(past_threshold[0].stack_height, 0);

        let mut at_distance = vec![circle(100, 100, 1000), circle(103, 100, 1050)];
        apply_stacking(&mut at_distance, 14, 100.0);
        assert_eq!(at_distance[0].stack_height, 0);
    }

    #[test]
    fn zero_leniency_only_stacks_simultaneous_objects() {
        let mut objects = vec![circle(100, 100, 1000), circle(100, 100, 1001)];
        apply_stacking(&mut objects, 14, 0.0);
        assert_eq!(objects[0].stack_height, 0);
    }

    #[test]
    fn spinner_is_ignored_between_circles() {
        let mut spinner = circle(100, 100, 1025);
        spinner.hit_type = 8;
        spinner.end_time = 1075;
        let mut objects = vec![circle(100, 100, 1000), spinner, circle(100, 100, 1100)];
        apply_stacking(&mut objects, 14, 150.0);
        assert_eq!(
            objects.iter().map(|o| o.stack_height).collect::<Vec<_>>(),
            vec![1, 0, 0]
        );
    }

    #[test]
    fn circle_after_slider_end_uses_negative_stacking() {
        let mut objects = vec![
            slider(100, 100, 200, 100, 1000, 1200),
            circle(200, 100, 1250),
            circle(200, 100, 1300),
        ];
        apply_stacking(&mut objects, 14, 150.0);
        assert_eq!(
            objects.iter().map(|o| o.stack_height).collect::<Vec<_>>(),
            vec![0, -1, -2]
        );
    }

    #[test]
    fn old_stacking_keeps_slider_tail_special_case() {
        let mut objects = vec![
            slider(100, 100, 200, 100, 1000, 1200),
            circle(200, 100, 1250),
            circle(200, 100, 1300),
        ];
        apply_stacking(&mut objects, 5, 150.0);
        assert_eq!(
            objects.iter().map(|o| o.stack_height).collect::<Vec<_>>(),
            vec![0, -1, -2]
        );
    }

    #[test]
    fn matches_official_stacking_edge_case_one() {
        let mut objects = vec![
            slider(311, 185, 318, 158, 217_871, 230_671),
            slider(311, 185, 335, 170, 218_071, 230_671),
            slider(311, 185, 338, 192, 218_271, 230_671),
            slider(311, 185, 325, 209, 218_471, 230_671),
            slider(311, 185, 304, 212, 218_671, 230_671),
            circle(311, 185, 240_271),
        ];
        let threshold = calculate_stack_threshold(9.2, 0.2);
        apply_stacking(&mut objects, 14, threshold);
        assert!(objects[..5].iter().all(|object| object.stack_height == 0));
    }

    #[test]
    fn matches_official_stacking_edge_case_two() {
        let mut objects = vec![
            circle(427, 124, 84_226),
            circle(427, 124, 84_337),
            circle(427, 124, 84_449),
        ];
        let threshold = calculate_stack_threshold(9.3, 0.2);
        assert!(threshold < 111.0);
        apply_stacking(&mut objects, 14, threshold);
        assert!(objects[..2].iter().all(|object| object.stack_height == 0));
    }
}
