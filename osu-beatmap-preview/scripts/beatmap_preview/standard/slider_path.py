from __future__ import annotations

import math
from bisect import bisect_right
from dataclasses import dataclass
from functools import lru_cache

from ..errors import PreviewError
from ..models import StandardHitObject


@dataclass(frozen=True)
class SliderPath:
    points: tuple[tuple[float, float], ...]
    cumulative_lengths: tuple[float, ...]
    total_length: float


def build_slider_path(hit_object: StandardHitObject) -> SliderPath:
    return _build_slider_path_cached(hit_object)


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
    """返回近似路径上指定进度对应的坐标点。"""
    if not path.points:
        raise PreviewError("slider path has no points")
    if len(path.points) < 2 or path.total_length <= 0:
        return path.points[0]

    target = path.total_length * max(0.0, min(1.0, progress))
    return _path_position_at_distance(path, target)


def slice_path(
    path: SliderPath,
    start_progress: float,
    end_progress: float,
) -> list[tuple[float, float]]:
    """截取近似路径在两个进度值之间的片段。"""
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
    """在 osu! 游玩坐标系中近似 standard slider 路径。"""
    if hit_object.slider_type is None:
        raise PreviewError("slider is missing path type")

    points = [(float(hit_object.x), float(hit_object.y))]
    points.extend((float(x), float(y)) for x, y in hit_object.slider_points)

    # osu! 的 slider 类型：L=直线，P=三点圆弧，C=Catmull，其余按 Bezier 处理。
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
    # C# SliderPath.calculateSubPath：先去除连续重复控制点，再将整段作为一条 Bezier 逼近。
    deduped = [points[0]]
    for point in points[1:]:
        if point != deduped[-1]:
            deduped.append(point)

    if len(deduped) < 2:
        return deduped

    return _approximate_bezier(deduped)


def _approximate_bezier(points: list[tuple[float, float]]) -> list[tuple[float, float]]:
    steps = max(64, len(points) * 24)
    return [_bezier_at(points, index / steps) for index in range(steps + 1)]


def _bezier_at(points: list[tuple[float, float]], t: float) -> tuple[float, float]:
    working = points[:]
    while len(working) > 1:
        working = [
            (
                working[index][0] * (1 - t) + working[index + 1][0] * t,
                working[index][1] * (1 - t) + working[index + 1][1] * t,
            )
            for index in range(len(working) - 1)
        ]
    return working[0]


def _approximate_perfect_curve(points: list[tuple[float, float]]) -> list[tuple[float, float]]:
    # Perfect curve 只有三个非共线点时才是圆弧；其它情况按 Bezier 兼容处理。
    if len(points) != 3 or _are_collinear(points[0], points[1], points[2]):
        return _approximate_bezier_segments(points)

    centre = _circle_centre(points[0], points[1], points[2])
    radius = math.dist(centre, points[0])
    start_angle = math.atan2(points[0][1] - centre[1], points[0][0] - centre[0])
    middle_angle = math.atan2(points[1][1] - centre[1], points[1][0] - centre[0])
    end_angle = math.atan2(points[2][1] - centre[1], points[2][0] - centre[0])
    end_angle = _normalise_arc_end(start_angle, middle_angle, end_angle)
    steps = max(64, round(abs(end_angle - start_angle) * radius / 4))
    return [
        (
            centre[0] + math.cos(start_angle + (end_angle - start_angle) * index / steps) * radius,
            centre[1] + math.sin(start_angle + (end_angle - start_angle) * index / steps) * radius,
        )
        for index in range(steps + 1)
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
    # 选择穿过中间控制点的那一段圆弧，避免走到另一侧的长弧。
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
    # 端点复制一次，保证首尾两段也能按 Catmull-Rom 样条计算切线。
    extended = [points[0], *points, points[-1]]
    for index in range(1, len(extended) - 2):
        p0, p1, p2, p3 = extended[index - 1], extended[index], extended[index + 1], extended[index + 2]
        for step in range(50):
            path.append(_catmull_at(p0, p1, p2, p3, step / 50))
    path.append(points[-1])
    return _dedupe_points(path)


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


def _fit_path_to_length(
    path: list[tuple[float, float]],
    expected_length: float,
) -> list[tuple[float, float]]:
    """按 osu! C# SliderPath.calculateLength 算法调整路径长度。

    C# 的做法（与 stable 行为一致）：
    1. 计算全部累积距离
    2. 从末尾移除超过 expected_length 的点
    3. 将最后一个保留点沿其到来方向（P[n-2]→P[n-1]）调整到 expected_length
    """
    if len(path) < 2 or expected_length <= 0:
        return path

    cumulative = [0.0]
    travelled = 0.0
    for index in range(1, len(path)):
        travelled += math.dist(path[index - 1], path[index])
        cumulative.append(travelled)

    actual_length = travelled
    if actual_length <= 0:
        return path

    fitted = list(path)
    while len(cumulative) > 1 and cumulative[-1] > expected_length + 0.001:
        fitted.pop()
        cumulative.pop()

    if len(fitted) < 2:
        return path[:2] if len(path) >= 2 else path

    # C#：若最后两个路径点重合，不扩展（匹配 stable）
    if fitted[-1] == fitted[-2]:
        return fitted

    last_cumulative = cumulative[-1]
    remaining = expected_length - last_cumulative
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
    first: tuple[float, float],
    second: tuple[float, float],
    third: tuple[float, float],
) -> bool:
    return abs((second[1] - first[1]) * (third[0] - first[0]) - (second[0] - first[0]) * (third[1] - first[1])) < 0.001
