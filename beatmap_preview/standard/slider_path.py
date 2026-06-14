from __future__ import annotations

import math
from bisect import bisect_right
from dataclasses import dataclass
from functools import lru_cache

from ..errors import PreviewError
from ..models import StandardHitObject

BEZIER_TOLERANCE = 0.25  # osu! PathApproximator.BEZIER_TOLERANCE
CATMULL_DETAIL = 50  # osu! PathApproximator.catmull_detail
CATMULL_MIN_DISTANCE = 6.0  # osu!stable Catmull 优化


@dataclass(frozen=True)
class SliderPath:
    points: tuple[tuple[float, float], ...]
    cumulative_lengths: tuple[float, ...]
    total_length: float


def build_slider_path(hit_object: StandardHitObject) -> SliderPath:
    return _build_slider_path_cached(hit_object)


def simplify_path(points: list[tuple[float, float]], tolerance: float = 1.0) -> list[tuple[float, float]]:
    # Ramer–Douglas–Peucker 算法简化路径，减少渲染点数
    if len(points) < 3:
        return points
    # 找到距离首尾连线最远的点
    sx, sy = points[0]
    ex, ey = points[-1]
    dx, dy = ex - sx, ey - sy
    line_len_sq = dx * dx + dy * dy
    max_dist_sq = 0.0
    max_idx = 0
    for i in range(1, len(points) - 1):
        if line_len_sq < 0.0001:
            dist_sq = (points[i][0] - sx) ** 2 + (points[i][1] - sy) ** 2
        else:
            t = ((points[i][0] - sx) * dx + (points[i][1] - sy) * dy) / line_len_sq
            t = max(0.0, min(1.0, t))
            px = sx + t * dx
            py = sy + t * dy
            dist_sq = (points[i][0] - px) ** 2 + (points[i][1] - py) ** 2
        if dist_sq > max_dist_sq:
            max_dist_sq = dist_sq
            max_idx = i
    if max_dist_sq <= tolerance * tolerance:
        return [points[0], points[-1]]
    left = simplify_path(points[:max_idx + 1], tolerance)
    right = simplify_path(points[max_idx:], tolerance)
    return left[:-1] + right


def build_path(points: list[tuple[float, float]] | tuple[tuple[float, float], ...]) -> SliderPath:
    deduped = _dedupe_points(list(points))
    if not deduped:
        return SliderPath(points=(), cumulative_lengths=(), total_length=0.0)

    cumulative_lengths = [0.0]
    travelled = 0.0
    for index in range(1, len(deduped)):
        travelled += math.dist(deduped[index - 1], deduped[index])
        cumulative_lengths.append(travelled)

    return SliderPath(
        points=tuple(deduped),
        cumulative_lengths=tuple(cumulative_lengths),
        total_length=travelled,
    )


def path_position_at(path: SliderPath, progress: float) -> tuple[float, float]:
    if not path.points:
        raise PreviewError("slider path has no points")
    if len(path.points) < 2 or path.total_length <= 0:
        return path.points[0]

    target = path.total_length * max(0.0, min(1.0, progress))
    return _path_position_at_distance(path, target)


def slice_path(
    path: SliderPath, start_progress: float, end_progress: float,
) -> list[tuple[float, float]]:
    if len(path.points) < 2 or path.total_length <= 0:
        return list(path.points)
    if start_progress > end_progress:
        start_progress, end_progress = end_progress, start_progress

    start_progress = max(0.0, min(1.0, start_progress))
    end_progress = max(0.0, min(1.0, end_progress))
    start_distance = path.total_length * start_progress
    end_distance = path.total_length * end_progress
    sliced = [_path_position_at_distance(path, start_distance)]

    for index, distance in enumerate(path.cumulative_lengths[1:-1], start=1):
        if start_distance < distance < end_distance:
            sliced.append(path.points[index])

    sliced.append(_path_position_at_distance(path, end_distance))
    return _dedupe_points(sliced)


@lru_cache(maxsize=None)
def _build_slider_path_cached(hit_object: StandardHitObject) -> SliderPath:
    if hit_object.slider_type is None:
        raise PreviewError("slider is missing path type")

    points = [(float(hit_object.x), float(hit_object.y))]
    points.extend((float(x), float(y)) for x, y in hit_object.slider_points)

    if hit_object.slider_type == "L":
        path = points
    elif hit_object.slider_type == "P":
        path = _approximate_perfect_curve(points)
    elif hit_object.slider_type == "C":
        path = _approximate_catmull(points)
    else:
        path = _approximate_bezier_segments(points)

    fitted = _fit_path_to_length(path, hit_object.slider_pixel_length)
    return build_path(simplify_path(fitted))


def _path_position_at_distance(path: SliderPath, target: float) -> tuple[float, float]:
    if target <= 0:
        return path.points[0]
    if target >= path.total_length:
        return path.points[-1]

    index = bisect_right(path.cumulative_lengths, target)
    previous_index = max(0, index - 1)
    next_index = min(len(path.points) - 1, index)
    previous = path.points[previous_index]
    current = path.points[next_index]
    segment_length = path.cumulative_lengths[next_index] - path.cumulative_lengths[previous_index]
    if segment_length <= 0:
        return current

    ratio = (target - path.cumulative_lengths[previous_index]) / segment_length
    return (
        previous[0] + (current[0] - previous[0]) * ratio,
        previous[1] + (current[1] - previous[1]) * ratio,
    )


# ——— Bezier (分段，重复控制点 = 段边界，产生尖角) ———

def _approximate_bezier_segments(points: list[tuple[float, float]]) -> list[tuple[float, float]]:
    path: list[tuple[float, float]] = []
    segment = [points[0]]

    for point in points[1:]:
        segment.append(point)
        if len(segment) > 2 and point == segment[-2]:
            segment.pop()
            path.extend(_approximate_bezier(segment))
            segment = [point]

    if len(segment) > 1:
        path.extend(_approximate_bezier(segment))
    return _dedupe_points(path)


def _approximate_bezier(points: list[tuple[float, float]]) -> list[tuple[float, float]]:
    if len(points) < 2:
        return points
    if len(points) == 2:
        return [points[0], points[1]]

    result: list[tuple[float, float]] = [points[0]]
    stack: list[list[tuple[float, float]]] = [list(points)]
    while stack:
        parent = stack.pop()
        if _bezier_is_flat_enough(parent):
            result.extend(_bezier_approximate(parent))
        else:
            left, right = _bezier_subdivide(parent)
            stack.append(right)
            stack.append(left)
    result.append(points[-1])
    return result


def _bezier_is_flat_enough(points: list[tuple[float, float]]) -> bool:
    threshold = BEZIER_TOLERANCE * BEZIER_TOLERANCE * 4
    for i in range(1, len(points) - 1):
        dx = points[i - 1][0] - 2 * points[i][0] + points[i + 1][0]
        dy = points[i - 1][1] - 2 * points[i][1] + points[i + 1][1]
        if dx * dx + dy * dy > threshold:
            return False
    return True


def _bezier_subdivide(points: list[tuple[float, float]]) -> tuple[list[tuple[float, float]], list[tuple[float, float]]]:
    count = len(points)
    midpoints = list(points)
    left: list[tuple[float, float]] = [points[0]] * count
    right: list[tuple[float, float]] = [points[-1]] * count
    for i in range(count):
        left[i] = midpoints[0]
        right[count - i - 1] = midpoints[count - i - 1]
        for j in range(count - i - 1):
            midpoints[j] = (
                (midpoints[j][0] + midpoints[j + 1][0]) / 2,
                (midpoints[j][1] + midpoints[j + 1][1]) / 2,
            )
    return left, right


def _bezier_approximate(points: list[tuple[float, float]]) -> list[tuple[float, float]]:
    count = len(points)
    left, _ = _bezier_subdivide(points)
    output: list[tuple[float, float]] = []
    for i in range(1, count - 1):
        p0, p1, p2 = left[i - 1], left[i], left[i + 1]
        output.append((0.25 * (p0[0] + 2 * p1[0] + p2[0]), 0.25 * (p0[1] + 2 * p1[1] + p2[1])))
    return output


# ——— Perfect / Circular arc ———

def _approximate_perfect_curve(points: list[tuple[float, float]]) -> list[tuple[float, float]]:
    if len(points) != 3 or _are_collinear(points[0], points[1], points[2]):
        return _approximate_bezier_segments(points)

    centre = _circle_centre(points[0], points[1], points[2])
    radius = math.dist(centre, points[0])
    start_angle = math.atan2(points[0][1] - centre[1], points[0][0] - centre[0])
    middle_angle = math.atan2(points[1][1] - centre[1], points[1][0] - centre[0])
    end_angle = math.atan2(points[2][1] - centre[1], points[2][0] - centre[0])
    end_angle = _normalise_arc_end(start_angle, middle_angle, end_angle)
    theta_range = end_angle - start_angle

    # osu! 公式: max(2, ceil(thetaRange / (2 * acos(1 - 0.1/Radius))))
    step_angle = 2 * math.acos(1 - 0.1 / radius) if radius > 0.1 else math.tau
    steps = max(2, math.ceil(abs(theta_range) / step_angle))
    if steps >= 1000:
        return _approximate_bezier_segments(points)

    return [
        (
            centre[0] + math.cos(start_angle + theta_range * index / steps) * radius,
            centre[1] + math.sin(start_angle + theta_range * index / steps) * radius,
        )
        for index in range(steps + 1)
    ]


def _circle_centre(
    first: tuple[float, float], second: tuple[float, float], third: tuple[float, float],
) -> tuple[float, float]:
    ax, ay = first; bx, by = second; cx, cy = third
    d = 2 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by))
    ux = ((ax * ax + ay * ay) * (by - cy) + (bx * bx + by * by) * (cy - ay) + (cx * cx + cy * cy) * (ay - by)) / d
    uy = ((ax * ax + ay * ay) * (cx - bx) + (bx * bx + by * by) * (ax - cx) + (cx * cx + cy * cy) * (bx - ax)) / d
    return ux, uy


def _normalise_arc_end(start: float, middle: float, end: float) -> float:
    while end < start:
        end += math.tau
    middle_forward = middle
    while middle_forward < start:
        middle_forward += math.tau
    if middle_forward <= end:
        return end
    while end > start:
        end -= math.tau
    return end


# ——— Catmull ———

def _approximate_catmull(points: list[tuple[float, float]]) -> list[tuple[float, float]]:
    if len(points) < 2:
        return points

    path: list[tuple[float, float]] = []
    extended = [points[0], *points, points[-1]]
    for index in range(1, len(extended) - 2):
        p0, p1, p2, p3 = extended[index - 1], extended[index], extended[index + 1], extended[index + 2]
        for step in range(CATMULL_DETAIL):
            path.append(_catmull_at(p0, p1, p2, p3, step / CATMULL_DETAIL))
    path.append(points[-1])

    return _catmull_optimise(path, points)


def _catmull_at(
    p0: tuple[float, float], p1: tuple[float, float],
    p2: tuple[float, float], p3: tuple[float, float], t: float,
) -> tuple[float, float]:
    t2 = t * t
    t3 = t2 * t
    x = 0.5 * (
        (2 * p1[0])
        + (-p0[0] + p2[0]) * t
        + (2 * p0[0] - 5 * p1[0] + 4 * p2[0] - p3[0]) * t2
        + (-p0[0] + 3 * p1[0] - 3 * p2[0] + p3[0]) * t3
    )
    y = 0.5 * (
        (2 * p1[1])
        + (-p0[1] + p2[1]) * t
        + (2 * p0[1] - 5 * p1[1] + 4 * p2[1] - p3[1]) * t2
        + (-p0[1] + 3 * p1[1] - 3 * p2[1] + p3[1]) * t3
    )
    return x, y


def _catmull_optimise(path: list[tuple[float, float]], knots: list[tuple[float, float]]) -> list[tuple[float, float]]:
    knot_set = set(knots)
    result: list[tuple[float, float]] = [path[0]]
    for i in range(1, len(path)):
        prev = result[-1]
        curr = path[i]
        if math.dist(prev, curr) >= CATMULL_MIN_DISTANCE or curr in knot_set or i == len(path) - 1:
            result.append(curr)
    return result


# ——— fit path to expected pixel length ———

def _fit_path_to_length(
    path: list[tuple[float, float]], expected_length: float,
) -> list[tuple[float, float]]:
    if len(path) < 2 or expected_length <= 0:
        return path

    cumulative = [0.0]
    travelled = 0.0
    for index in range(1, len(path)):
        travelled += math.dist(path[index - 1], path[index])
        cumulative.append(travelled)

    if travelled <= 0:
        return path

    if travelled > expected_length:
        fitted = [path[0]]
        previous_distance = 0.0

        for index in range(1, len(path)):
            current_distance = cumulative[index]
            previous = path[index - 1]
            current = path[index]

            if current_distance >= expected_length:
                segment_length = current_distance - previous_distance
                if segment_length <= 0:
                    fitted.append(current)
                else:
                    ratio = (expected_length - previous_distance) / segment_length
                    fitted.append((
                        previous[0] + (current[0] - previous[0]) * ratio,
                        previous[1] + (current[1] - previous[1]) * ratio,
                    ))
                return fitted

            fitted.append(current)
            previous_distance = current_distance

        return fitted

    fitted = list(path)
    if fitted[-1] == fitted[-2]:
        return fitted

    remaining = expected_length - travelled
    direction = (fitted[-1][0] - fitted[-2][0], fitted[-1][1] - fitted[-2][1])
    direction_length = math.hypot(direction[0], direction[1])

    if direction_length > 0:
        fitted[-1] = (
            fitted[-1][0] + direction[0] / direction_length * remaining,
            fitted[-1][1] + direction[1] / direction_length * remaining,
        )

    return fitted


def _dedupe_points(points: list[tuple[float, float]]) -> list[tuple[float, float]]:
    deduped: list[tuple[float, float]] = []
    for point in points:
        if not deduped or point != deduped[-1]:
            deduped.append(point)
    return deduped


def _are_collinear(
    first: tuple[float, float], second: tuple[float, float], third: tuple[float, float],
) -> bool:
    return abs((second[1] - first[1]) * (third[0] - first[0]) - (second[0] - first[0]) * (third[1] - first[1])) < 0.001
