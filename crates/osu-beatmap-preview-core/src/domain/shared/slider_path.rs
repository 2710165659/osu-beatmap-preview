//! standard 与 catch 共用的滑条路径近似算法。
//! standard 将 RDP 显示简化与完整弧长计时分离；catch 保留完整点集，
//! 使用 lazer calculateLength 风格的拟合。

pub const BEZIER_TOLERANCE: f64 = 0.25;
pub const CATMULL_DETAIL: usize = 50;
pub const CATMULL_MIN_DISTANCE: f64 = 6.0;

pub type P = (f64, f64);

#[derive(Debug, Clone, Default)]
pub struct SliderPath {
    pub points: Vec<P>,
    pub cumulative_lengths: Vec<f64>,
    pub total_length: f64,
}

fn dist(a: P, b: P) -> f64 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    (dx * dx + dy * dy).sqrt()
}

pub fn build_path(points: &[P]) -> SliderPath {
    let deduped = dedupe_points(points);
    if deduped.is_empty() {
        return SliderPath::default();
    }
    let mut cumulative = Vec::with_capacity(deduped.len());
    cumulative.push(0.0);
    let mut travelled = 0.0;
    for i in 1..deduped.len() {
        travelled += dist(deduped[i - 1], deduped[i]);
        cumulative.push(travelled);
    }
    SliderPath {
        points: deduped,
        cumulative_lengths: cumulative,
        total_length: travelled,
    }
}

pub fn path_position_at(path: &SliderPath, progress: f64) -> P {
    if path.points.is_empty() {
        return (0.0, 0.0);
    }
    if path.points.len() < 2 || path.total_length <= 0.0 {
        return path.points[0];
    }
    let target = path.total_length * progress.clamp(0.0, 1.0);
    path_position_at_distance(path, target)
}

pub fn path_position_at_distance(path: &SliderPath, target: f64) -> P {
    if target <= 0.0 {
        return path.points[0];
    }
    if target >= path.total_length {
        return *path.points.last().unwrap();
    }
    // 等价于 bisect_right。
    let index = path.cumulative_lengths.partition_point(|&v| v <= target);
    let previous_index = index.saturating_sub(1);
    let next_index = index.min(path.points.len() - 1);
    let previous = path.points[previous_index];
    let current = path.points[next_index];
    let segment_length =
        path.cumulative_lengths[next_index] - path.cumulative_lengths[previous_index];
    if segment_length <= 0.0 {
        return current;
    }
    let ratio = (target - path.cumulative_lengths[previous_index]) / segment_length;
    (
        previous.0 + (current.0 - previous.0) * ratio,
        previous.1 + (current.1 - previous.1) * ratio,
    )
}

pub fn slice_path(path: &SliderPath, start_progress: f64, end_progress: f64) -> Vec<P> {
    if path.points.len() < 2 || path.total_length <= 0.0 {
        return path.points.clone();
    }
    let (mut s, mut e) = (start_progress, end_progress);
    if s > e {
        std::mem::swap(&mut s, &mut e);
    }
    let s = s.clamp(0.0, 1.0);
    let e = e.clamp(0.0, 1.0);
    let start_distance = path.total_length * s;
    let end_distance = path.total_length * e;
    // 只遍历切片内部节点；长滑条蛇入初期不应每帧扫描整条路径。
    let first = path
        .cumulative_lengths
        .partition_point(|&distance| distance <= start_distance)
        .max(1);
    let last = path
        .cumulative_lengths
        .partition_point(|&distance| distance < end_distance)
        .min(path.points.len() - 1);
    let mut sliced = Vec::with_capacity(last.saturating_sub(first) + 2);
    sliced.push(path_position_at_distance(path, start_distance));
    if first < last {
        sliced.extend_from_slice(&path.points[first..last]);
    }
    sliced.push(path_position_at_distance(path, end_distance));
    dedupe_points(&sliced)
}

/// 根据滑条类型构建原始曲线（长度拟合前）。
fn approximate_curve(slider_type: &str, points: &[P], perfect_lazer_semantics: bool) -> Vec<P> {
    match slider_type {
        "L" => points.to_vec(),
        "P" => approximate_perfect_curve(points, perfect_lazer_semantics),
        "C" => approximate_catmull(points),
        _ => approximate_bezier_segments(points),
    }
}

/// 先拟合完整曲线长度；显示简化必须在拟合之后进行，避免改变末端位置。
fn build_standard_fitted_path(
    x: i32,
    y: i32,
    slider_points: &[(i32, i32)],
    slider_type: &str,
    pixel_length: f64,
) -> Vec<P> {
    let mut points: Vec<P> = Vec::with_capacity(slider_points.len() + 1);
    points.push((x as f64, y as f64));
    points.extend(slider_points.iter().map(|&(px, py)| (px as f64, py as f64)));
    fit_path_truncate_extend(
        &approximate_curve(slider_type, &points, false),
        pixel_length,
    )
}

/// 返回显示路径和计时路径。显示路径允许 RDP 简化以控制光栅化成本；计时路径
/// 保留所有拟合点，确保密集回绕不会丢失弧长，滑条球与 tick 仍按游戏节奏移动。
pub fn build_standard_slider_paths(
    x: i32,
    y: i32,
    slider_points: &[(i32, i32)],
    slider_type: &str,
    pixel_length: f64,
) -> (SliderPath, SliderPath) {
    let fitted = build_standard_fitted_path(x, y, slider_points, slider_type, pixel_length);
    let timing = build_path(&fitted);
    let display = simplify_timed_path(&timing, 1.0);
    (display, timing)
}

pub fn build_standard_slider_path(
    x: i32,
    y: i32,
    slider_points: &[(i32, i32)],
    slider_type: &str,
    pixel_length: f64,
) -> SliderPath {
    let fitted = build_standard_fitted_path(x, y, slider_points, slider_type, pixel_length);
    // 堆叠与 follow point 只需要路径端点和形状，无需分配完整计时副本。
    build_path(&simplify_path(&fitted, 1.0))
}

/// catch 模式滑条路径：不做简化，使用 lazer calculateLength 拟合。
pub fn build_catch_slider_path(
    x: i32,
    y: i32,
    slider_points: &[(i32, i32)],
    slider_type: &str,
    pixel_length: f64,
) -> SliderPath {
    let mut points: Vec<P> = Vec::with_capacity(slider_points.len() + 1);
    points.push((x as f64, y as f64));
    points.extend(slider_points.iter().map(|&(px, py)| (px as f64, py as f64)));
    let path = approximate_curve(slider_type, &points, true);
    build_path(&fit_path_lazer(&path, pixel_length))
}

pub fn simplify_path(points: &[P], tolerance: f64) -> Vec<P> {
    simplify_indices(points, tolerance)
        .into_iter()
        .map(|index| points[index])
        .collect()
}

/// 显示折线仍使用原始累计弧长。删除微小回绕只影响画面采样，不能压缩其时间；
/// 例如同一小区域内的多次往返仍占用原来的进度区间，蛇入/蛇出不会提前跳过。
fn simplify_timed_path(path: &SliderPath, tolerance: f64) -> SliderPath {
    let indices = simplify_indices(&path.points, tolerance);
    SliderPath {
        points: indices.iter().map(|&index| path.points[index]).collect(),
        cumulative_lengths: indices
            .iter()
            .map(|&index| path.cumulative_lengths[index])
            .collect(),
        total_length: path.total_length,
    }
}

fn simplify_indices(points: &[P], tolerance: f64) -> Vec<usize> {
    if points.len() < 3 {
        return (0..points.len()).collect();
    }
    // 索引栈保留原有 RDP 的距离判定和点序，避免长路径的递归栈及反复拷贝。
    let mut retained = vec![false; points.len()];
    retained[0] = true;
    retained[points.len() - 1] = true;
    let mut ranges = vec![(0, points.len() - 1)];
    while let Some((start, end)) = ranges.pop() {
        if end <= start + 1 {
            continue;
        }
        let (sx, sy) = points[start];
        let (ex, ey) = points[end];
        let dx = ex - sx;
        let dy = ey - sy;
        let line_len_sq = dx * dx + dy * dy;
        let mut max_dist_sq = 0.0;
        let mut max_idx = 0usize;
        for (i, point) in points.iter().enumerate().take(end).skip(start + 1) {
            let dist_sq = if line_len_sq < 0.0001 {
                (point.0 - sx).powi(2) + (point.1 - sy).powi(2)
            } else {
                let t = (((point.0 - sx) * dx + (point.1 - sy) * dy) / line_len_sq).clamp(0.0, 1.0);
                let px = sx + t * dx;
                let py = sy + t * dy;
                (point.0 - px).powi(2) + (point.1 - py).powi(2)
            };
            if dist_sq > max_dist_sq {
                max_dist_sq = dist_sq;
                max_idx = i;
            }
        }
        if max_dist_sq > tolerance * tolerance {
            retained[max_idx] = true;
            ranges.push((max_idx, end));
            ranges.push((start, max_idx));
        }
    }
    retained
        .into_iter()
        .enumerate()
        .filter_map(|(index, keep)| keep.then_some(index))
        .collect()
}

// ——— 贝塞尔曲线 ———

fn approximate_bezier_segments(points: &[P]) -> Vec<P> {
    let mut path: Vec<P> = Vec::new();
    let mut segment = vec![points[0]];
    for &point in &points[1..] {
        segment.push(point);
        if segment.len() > 2 && point == segment[segment.len() - 2] {
            segment.pop();
            path.extend(approximate_bezier(&segment));
            segment = vec![point];
        }
    }
    if segment.len() > 1 {
        path.extend(approximate_bezier(&segment));
    }
    dedupe_points(&path)
}

fn approximate_bezier(points: &[P]) -> Vec<P> {
    if points.len() < 2 {
        return points.to_vec();
    }
    if points.len() == 2 {
        return vec![points[0], points[1]];
    }
    let mut result = Vec::new();
    let mut stack: Vec<Vec<P>> = vec![points.to_vec()];
    while let Some(parent) = stack.pop() {
        if bezier_is_flat_enough(&parent) {
            result.extend(bezier_approximate(&parent));
        } else {
            let (left, right) = bezier_subdivide(&parent);
            stack.push(right);
            stack.push(left);
        }
    }
    result.push(*points.last().unwrap());
    result
}

fn bezier_is_flat_enough(points: &[P]) -> bool {
    let threshold = BEZIER_TOLERANCE * BEZIER_TOLERANCE * 4.0;
    for i in 1..points.len() - 1 {
        let dx = points[i - 1].0 - 2.0 * points[i].0 + points[i + 1].0;
        let dy = points[i - 1].1 - 2.0 * points[i].1 + points[i + 1].1;
        if dx * dx + dy * dy > threshold {
            return false;
        }
    }
    true
}

fn bezier_subdivide(points: &[P]) -> (Vec<P>, Vec<P>) {
    let count = points.len();
    let mut midpoints = points.to_vec();
    let mut left = vec![points[0]; count];
    let mut right = vec![*points.last().unwrap(); count];
    for i in 0..count {
        left[i] = midpoints[0];
        right[count - i - 1] = midpoints[count - i - 1];
        for j in 0..count - i - 1 {
            midpoints[j] = (
                (midpoints[j].0 + midpoints[j + 1].0) / 2.0,
                (midpoints[j].1 + midpoints[j + 1].1) / 2.0,
            );
        }
    }
    (left, right)
}

fn bezier_approximate(points: &[P]) -> Vec<P> {
    let count = points.len();
    let (mut left, right) = bezier_subdivide(points);
    // osu-framework 将左右子曲线拼接后以 2*i 采样；只取左半边会漏掉
    // 每个细分段的后半段及连接点，密集小曲线的弧长和形状都会失真。
    left.extend_from_slice(&right[1..]);
    let mut output = Vec::with_capacity(count - 1);
    output.push(points[0]);
    for i in 1..count - 1 {
        let index = 2 * i;
        let p0 = left[index - 1];
        let p1 = left[index];
        let p2 = left[index + 1];
        output.push((
            0.25 * (p0.0 + 2.0 * p1.0 + p2.0),
            0.25 * (p0.1 + 2.0 * p1.1 + p2.1),
        ));
    }
    output
}

// ——— 完美圆 ———

fn approximate_perfect_curve(points: &[P], lazer_semantics: bool) -> Vec<P> {
    if points.len() != 3 || are_collinear(points[0], points[1], points[2]) {
        return approximate_bezier_segments(points);
    }
    let centre = circle_centre(points[0], points[1], points[2]);
    let radius = dist(centre, points[0]);
    let start_angle = (points[0].1 - centre.1).atan2(points[0].0 - centre.0);
    let middle_angle = (points[1].1 - centre.1).atan2(points[1].0 - centre.0);
    let end_angle = (points[2].1 - centre.1).atan2(points[2].0 - centre.0);
    let end_angle = normalise_arc_end(start_angle, middle_angle, end_angle);
    let theta_range = end_angle - start_angle;

    let tau = std::f64::consts::TAU;
    let step_angle = if radius > 0.1 {
        2.0 * (1.0 - 0.1 / radius).acos()
    } else {
        tau
    };
    let n = ((theta_range.abs() / step_angle).ceil() as i64).max(2);
    if n >= 1000 {
        return approximate_bezier_segments(points);
    }

    if lazer_semantics {
        // n 为点数，除以 n - 1。
        let point_count = n;
        (0..point_count)
            .map(|index| {
                let angle = start_angle + theta_range * index as f64 / (point_count - 1) as f64;
                (
                    centre.0 + angle.cos() * radius,
                    centre.1 + angle.sin() * radius,
                )
            })
            .collect()
    } else {
        // n 为线段数，按 n 等分生成 n+1 个点。
        let steps = n;
        (0..=steps)
            .map(|index| {
                let angle = start_angle + theta_range * index as f64 / steps as f64;
                (
                    centre.0 + angle.cos() * radius,
                    centre.1 + angle.sin() * radius,
                )
            })
            .collect()
    }
}

fn circle_centre(first: P, second: P, third: P) -> P {
    let (ax, ay) = first;
    let (bx, by) = second;
    let (cx, cy) = third;
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    let ux = ((ax * ax + ay * ay) * (by - cy)
        + (bx * bx + by * by) * (cy - ay)
        + (cx * cx + cy * cy) * (ay - by))
        / d;
    let uy = ((ax * ax + ay * ay) * (cx - bx)
        + (bx * bx + by * by) * (ax - cx)
        + (cx * cx + cy * cy) * (bx - ax))
        / d;
    (ux, uy)
}

fn normalise_arc_end(start: f64, middle: f64, end: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut end = end;
    while end < start {
        end += tau;
    }
    let mut middle_forward = middle;
    while middle_forward < start {
        middle_forward += tau;
    }
    if middle_forward <= end {
        return end;
    }
    while end > start {
        end -= tau;
    }
    end
}

fn are_collinear(first: P, second: P, third: P) -> bool {
    ((second.1 - first.1) * (third.0 - first.0) - (second.0 - first.0) * (third.1 - first.1)).abs()
        < 0.001
}

// ——— Catmull-Rom 曲线 ———

fn approximate_catmull(points: &[P]) -> Vec<P> {
    if points.len() < 2 {
        return points.to_vec();
    }
    let mut path: Vec<P> = Vec::new();
    let mut extended = Vec::with_capacity(points.len() + 2);
    extended.push(points[0]);
    extended.extend_from_slice(points);
    extended.push(*points.last().unwrap());
    for index in 1..extended.len() - 2 {
        let p0 = extended[index - 1];
        let p1 = extended[index];
        let p2 = extended[index + 1];
        let p3 = extended[index + 2];
        for step in 0..CATMULL_DETAIL {
            path.push(catmull_at(
                p0,
                p1,
                p2,
                p3,
                step as f64 / CATMULL_DETAIL as f64,
            ));
        }
    }
    path.push(*points.last().unwrap());
    catmull_optimise(&path, points)
}

fn catmull_at(p0: P, p1: P, p2: P, p3: P, t: f64) -> P {
    let t2 = t * t;
    let t3 = t2 * t;
    let x = 0.5
        * ((2.0 * p1.0)
            + (-p0.0 + p2.0) * t
            + (2.0 * p0.0 - 5.0 * p1.0 + 4.0 * p2.0 - p3.0) * t2
            + (-p0.0 + 3.0 * p1.0 - 3.0 * p2.0 + p3.0) * t3);
    let y = 0.5
        * ((2.0 * p1.1)
            + (-p0.1 + p2.1) * t
            + (2.0 * p0.1 - 5.0 * p1.1 + 4.0 * p2.1 - p3.1) * t2
            + (-p0.1 + 3.0 * p1.1 - 3.0 * p2.1 + p3.1) * t3);
    (x, y)
}

fn catmull_optimise(path: &[P], knots: &[P]) -> Vec<P> {
    let is_knot = |p: P| knots.contains(&p);
    let mut result = vec![path[0]];
    for i in 1..path.len() {
        let prev = *result.last().unwrap();
        let curr = path[i];
        if dist(prev, curr) >= CATMULL_MIN_DISTANCE || is_knot(curr) || i == path.len() - 1 {
            result.push(curr);
        }
    }
    result
}

// ——— 长度拟合 ———

/// standard 变体：按期望长度截断，或向外延伸最后一个点。
fn fit_path_truncate_extend(path: &[P], expected_length: f64) -> Vec<P> {
    if path.len() < 2 || expected_length <= 0.0 {
        return path.to_vec();
    }
    let mut cumulative = Vec::with_capacity(path.len());
    cumulative.push(0.0);
    let mut travelled = 0.0;
    for i in 1..path.len() {
        travelled += dist(path[i - 1], path[i]);
        cumulative.push(travelled);
    }
    if travelled <= 0.0 {
        return path.to_vec();
    }

    if travelled > expected_length {
        let mut fitted = vec![path[0]];
        let mut previous_distance = 0.0;
        for i in 1..path.len() {
            let current_distance = cumulative[i];
            let previous = path[i - 1];
            let current = path[i];
            if current_distance >= expected_length {
                let segment_length = current_distance - previous_distance;
                if segment_length <= 0.0 {
                    fitted.push(current);
                } else {
                    let ratio = (expected_length - previous_distance) / segment_length;
                    fitted.push((
                        previous.0 + (current.0 - previous.0) * ratio,
                        previous.1 + (current.1 - previous.1) * ratio,
                    ));
                }
                return fitted;
            }
            fitted.push(current);
            previous_distance = current_distance;
        }
        return fitted;
    }

    let mut fitted = path.to_vec();
    let n = fitted.len();
    if fitted[n - 1] == fitted[n - 2] {
        return fitted;
    }
    let remaining = expected_length - travelled;
    let direction = (
        fitted[n - 1].0 - fitted[n - 2].0,
        fitted[n - 1].1 - fitted[n - 2].1,
    );
    let direction_length = (direction.0 * direction.0 + direction.1 * direction.1).sqrt();
    if direction_length > 0.0 {
        fitted[n - 1] = (
            fitted[n - 1].0 + direction.0 / direction_length * remaining,
            fitted[n - 1].1 + direction.1 / direction_length * remaining,
        );
    }
    fitted
}

/// catch 变体：遵循 lazer SliderPath.calculateLength 语义。
fn fit_path_lazer(path: &[P], expected_length: f64) -> Vec<P> {
    if path.len() < 2 || expected_length <= 0.0 {
        return path.to_vec();
    }
    let mut cumulative = Vec::with_capacity(path.len());
    cumulative.push(0.0);
    let mut travelled = 0.0;
    for i in 1..path.len() {
        travelled += dist(path[i - 1], path[i]);
        cumulative.push(travelled);
    }
    if travelled <= 0.0 {
        return path.to_vec();
    }

    let mut fitted = path.to_vec();
    let mut fitted_lengths = cumulative;

    if travelled > expected_length {
        while !fitted_lengths.is_empty() && *fitted_lengths.last().unwrap() >= expected_length {
            fitted_lengths.pop();
            fitted.pop();
        }
        if fitted.is_empty() {
            return vec![path[0]];
        }
        let path_end_index = fitted.len();
        fitted.push(path[path_end_index]);
        fitted_lengths.push(expected_length);
    }

    let n = fitted.len();
    if n < 2 || fitted[n - 1] == fitted[n - 2] {
        return fitted;
    }
    let remaining = expected_length - fitted_lengths[fitted_lengths.len() - 2];
    let direction = (
        fitted[n - 1].0 - fitted[n - 2].0,
        fitted[n - 1].1 - fitted[n - 2].1,
    );
    let direction_length = (direction.0 * direction.0 + direction.1 * direction.1).sqrt();
    if direction_length > 0.0 {
        fitted[n - 1] = (
            fitted[n - 2].0 + direction.0 / direction_length * remaining,
            fitted[n - 2].1 + direction.1 / direction_length * remaining,
        );
    }
    fitted
}

pub fn dedupe_points(points: &[P]) -> Vec<P> {
    let mut deduped: Vec<P> = Vec::with_capacity(points.len());
    for &point in points {
        if deduped.last() != Some(&point) {
            deduped.push(point);
        }
    }
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_bezier_samples_both_halves() {
        let points = [(0.0, 0.0), (50.0, 0.1), (100.0, 0.0)];
        let path = approximate_bezier(&points);
        assert_eq!(path, vec![(0.0, 0.0), (50.0, 0.05), (100.0, 0.0)]);
    }

    #[test]
    fn dense_backtracking_keeps_length_and_local_dwell() {
        // 在一像素内往返会被显示简化删除，但必须保留累计路程与局部停留时间。
        let mut points = vec![(100, 0)];
        points.extend((0..10_000).map(|i| (100 + i % 2, 0)));
        points.push((200, 0));
        let (display, timing) = build_standard_slider_paths(0, 0, &points, "L", 10_298.0);
        assert!(display.points.len() < timing.points.len());
        assert_eq!(display.total_length, timing.total_length);
        assert!(timing.total_length > 10_000.0);
        for progress in [0.1, 0.3, 0.5, 0.7, 0.9] {
            let position = path_position_at(&timing, progress);
            assert!((100.0..=101.0).contains(&position.0));
        }
        assert_eq!(display.points.last(), timing.points.last());
    }

    #[test]
    fn straight_and_segmented_sliders_keep_shape() {
        for kind in ["L", "B"] {
            let points = [(50, 0), (50, 0), (50, 100), (50, 100), (150, 100)];
            let (display, timing) = build_standard_slider_paths(0, 0, &points, kind, 250.0);
            for index in 0..=100 {
                let progress = index as f64 / 100.0;
                assert_eq!(
                    path_position_at(&display, progress),
                    path_position_at(&timing, progress)
                );
            }
        }
    }

    #[test]
    fn ordinary_bezier_matches_analytic_curve() {
        // 用解析二次曲线验证主体，而不是用另一份相同的细分实现作为期望值。
        let path = build_path(&approximate_bezier(&[
            (0.0, 0.0),
            (50.0, 100.0),
            (100.0, 0.0),
        ]));
        for i in 0..=100 {
            let x = i as f64;
            let t = x / 100.0;
            let expected_y = 200.0 * t * (1.0 - t);
            let segment = path
                .points
                .windows(2)
                .find(|pair| pair[0].0 <= x && x <= pair[1].0)
                .unwrap();
            let ratio = (x - segment[0].0) / (segment[1].0 - segment[0].0);
            let actual_y = segment[0].1 + ratio * (segment[1].1 - segment[0].1);
            assert!((actual_y - expected_y).abs() <= BEZIER_TOLERANCE);
        }
    }

    #[test]
    fn extreme_and_ordinary_beatmaps_preserve_path_invariants() {
        use crate::domain::models::HitObjects;
        use crate::domain::parser::parse_beatmap_bytes;

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let files = [
            "testdata_slider/4858443.osu",
            "testdata_slider/5467386.osu",
            "testdata_conversion/1946909.osu",
            "testdata_conversion/5051189.osu",
        ];
        for file in files {
            let beatmap = parse_beatmap_bytes(&std::fs::read(root.join(file)).unwrap()).unwrap();
            let HitObjects::Standard(objects) = beatmap.hit_objects else {
                panic!("需要 standard 谱面")
            };
            for object in objects.iter().filter(|object| object.hit_type & 2 != 0) {
                let (display, timing) = build_standard_slider_paths(
                    object.x,
                    object.y,
                    &object.slider_points,
                    object.slider_type.as_deref().unwrap_or("B"),
                    object.slider_pixel_length,
                );
                assert!(
                    (timing.total_length - object.slider_pixel_length).abs() < 1e-6,
                    "{file} @ {}",
                    object.start_time
                );
                assert_eq!(display.total_length, timing.total_length);
                assert_eq!(display.points.first(), timing.points.first());
                assert_eq!(display.points.last(), timing.points.last());
                assert!(display
                    .cumulative_lengths
                    .windows(2)
                    .all(|pair| pair[0] <= pair[1]));
                // 显示简化只能在一世界像素的误差内取直线捷径，计时位置必须保留原始回绕。
                for i in 0..=100 {
                    let progress = i as f64 / 100.0;
                    let position = path_position_at(&timing, progress);
                    let index = display
                        .cumulative_lengths
                        .partition_point(|&distance| distance <= timing.total_length * progress);
                    let a = display.points[index.saturating_sub(1)];
                    let b = display.points[index.min(display.points.len() - 1)];
                    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                    let length = dx * dx + dy * dy;
                    let t = if length <= 1e-12 {
                        0.0
                    } else {
                        (((position.0 - a.0) * dx + (position.1 - a.1) * dy) / length)
                            .clamp(0.0, 1.0)
                    };
                    assert!(
                        dist(position, (a.0 + t * dx, a.1 + t * dy)) <= 1.000001,
                        "{file} @ {}",
                        object.start_time
                    );
                }
            }
        }
    }
}
