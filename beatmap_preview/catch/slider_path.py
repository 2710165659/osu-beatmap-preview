from __future__ import annotations

import math
from bisect import bisect_right
from dataclasses import dataclass
from functools import lru_cache

from ..errors import PreviewError
from ..models import CatchHitObject

BEZIER_TOLERANCE = 0.25  # osu! PathApproximator.BEZIER_TOLERANCE
CATMULL_DETAIL = 50  # osu! PathApproximator.catmull_detail
CATMULL_MIN_DISTANCE = 6.0  # osu!stable Catmull 优化


@dataclass(frozen=True)
class SliderPath:
    points: tuple[tuple[float, float], ...]
    cumulative_lengths: tuple[float, ...]
    total_length: float


def build_slider_path(hit_object: CatchHitObject) -> SliderPath:
    """构建 catch 转谱专用 slider path。

    这里按 osu!lazer SliderPath 的点数语义和 ExpectedDistance 拟合方式实现。
    不做 RDP 简化，避免 tiny droplet / hyperdash 的判定落点和游戏内不一致。
    """
    return _build_slider_path_cached(hit_object)


def path_position_at(path: SliderPath, progress: float) -> tuple[float, float]:
    if not path.points:
        raise PreviewError("slider path has no points")
    if len(path.points) < 2 or path.total_length <= 0:
        return path.points[0]

    target = path.total_length * max(0.0, min(1.0, progress))
    return _path_position_at_distance(path, target)


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


@lru_cache(maxsize=None)
def _build_slider_path_cached(hit_object: CatchHitObject) -> SliderPath:
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

    return build_path(_fit_path_to_length(path, hit_object.slider_pixel_length))


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
    for index in range(1, len(points) - 1):
        dx = points[index - 1][0] - 2 * points[index][0] + points[index + 1][0]
        dy = points[index - 1][1] - 2 * points[index][1] + points[index + 1][1]
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
    for index in range(1, count - 1):
        p0, p1, p2 = left[index - 1], left[index], left[index + 1]
        output.append((0.25 * (p0[0] + 2 * p1[0] + p2[0]), 0.25 * (p0[1] + 2 * p1[1] + p2[1])))
    return output


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

    # lazer 的 subPoints 表示“点数”而不是“分段数”，最少 2 点就是起点 + 终点。
    step_angle = 2 * math.acos(1 - 0.1 / radius) if radius > 0.1 else math.tau
    point_count = max(2, math.ceil(abs(theta_range) / step_angle))
    if point_count >= 1000:
        return _approximate_bezier_segments(points)

    return [
        (
            centre[0] + math.cos(start_angle + theta_range * index / (point_count - 1)) * radius,
            centre[1] + math.sin(start_angle + theta_range * index / (point_count - 1)) * radius,
        )
        for index in range(point_count)
    ]


def _circle_centre(
    first: tuple[float, float],
    second: tuple[float, float],
    third: tuple[float, float],
) -> tuple[float, float]:
    ax, ay = first
    bx, by = second
    cx, cy = third
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
    p0: tuple[float, float],
    p1: tuple[float, float],
    p2: tuple[float, float],
    p3: tuple[float, float],
    t: float,
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
    for index in range(1, len(path)):
        prev = result[-1]
        curr = path[index]
        if math.dist(prev, curr) >= CATMULL_MIN_DISTANCE or curr in knot_set or index == len(path) - 1:
            result.append(curr)
    return result


def _fit_path_to_length(
    path: list[tuple[float, float]],
    expected_length: float,
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

    fitted = list(path)
    fitted_lengths = list(cumulative)

    if travelled > expected_length:
        while len(fitted_lengths) > 0 and fitted_lengths[-1] >= expected_length:
            fitted_lengths.pop()
            fitted.pop()

        if len(fitted) <= 0:
            return [path[0]]

        path_end_index = len(fitted)
        if path_end_index <= 0:
            return [path[0]]

        fitted.append(path[path_end_index])
        fitted_lengths.append(expected_length)

    if fitted[-1] == fitted[-2]:
        return fitted

    # 与 lazer SliderPath.calculateLength() 一致：按当前最后一段方向，把最后一点移动到 ExpectedDistance。
    remaining = expected_length - fitted_lengths[-2]
    direction = (fitted[-1][0] - fitted[-2][0], fitted[-1][1] - fitted[-2][1])
    direction_length = math.hypot(direction[0], direction[1])

    if direction_length > 0:
        fitted[-1] = (
            fitted[-2][0] + direction[0] / direction_length * remaining,
            fitted[-2][1] + direction[1] / direction_length * remaining,
        )

    return fitted


def _dedupe_points(points: list[tuple[float, float]]) -> list[tuple[float, float]]:
    deduped: list[tuple[float, float]] = []
    for point in points:
        if not deduped or point != deduped[-1]:
            deduped.append(point)
    return deduped


def _are_collinear(
    first: tuple[float, float],
    second: tuple[float, float],
    third: tuple[float, float],
) -> bool:
    return abs((second[1] - first[1]) * (third[0] - first[0]) - (second[0] - first[0]) * (third[1] - first[1])) < 0.001
