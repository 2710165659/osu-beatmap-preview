from __future__ import annotations

import math
from dataclasses import dataclass
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

from ..errors import PreviewError
from ..models import Beatmap, ManiaHitObject, TimingPoint
from ..mods import ModSettings
from .convert import SOURCE_MODE_KEY
from .config import (
    BEAT_LINE,
    BOTTOM_PADDING_MS,
    COLUMN_GAP,
    FIXED_COLUMN_COUNT_6_TO_10_MIN,
    IMAGE_BACKGROUND,
    LANE_BACKGROUND,
    LANE_GAP,
    LANE_SEPARATOR,
    LANE_WIDTH,
    LEFT_PANEL_BACKGROUND,
    LEFT_PANEL_WIDTH,
    MAX_AREA_HEIGHT_0_TO_1_MIN,
    MAX_AREA_HEIGHT_1_TO_2_MIN,
    MAX_AREA_HEIGHT_2_TO_3_MIN,
    MAX_AREA_HEIGHT_3_TO_4_MIN,
    MAX_AREA_HEIGHT_4_TO_5_MIN,
    MAX_AREA_HEIGHT_5_TO_6_MIN,
    MAX_SUPPORTED_DURATION_MS,
    MEASURE_LINE,
    NOTE_HEAD_HEIGHT,
    NOTE_SIDE_PADDING,
    PAGE_MARGIN_X,
    PAGE_MARGIN_Y,
    PIXELS_PER_MS,
    RULER_TEXT,
    SUBDIVISION_LINE,
    SV_TEXT_COLOR,
    SV_TEXT_FONT_SIZE,
    TIME_LABEL_FONT_SIZE,
    TOP_BUFFER,
)


LANE_COLOR_PALETTES = {
    1: ["#e9eef4"],
    2: ["#e9eef4", "#e9eef4"],
    3: ["#e9eef4", "#bcdbf1", "#e9eef4"],
    4: ["#e9eef4", "#bcdbf1", "#bcdbf1", "#e9eef4"],
    5: ["#e9eef4", "#ccfcb2", "#ffe274", "#ccfcb2", "#e9eef4"],
    6: ["#e9eef4", "#ccfcb2", "#e9eef4", "#e9eef4", "#ccfcb2", "#e9eef4"],
    7: ["#e9eef4", "#ccfcb2", "#e9eef4", "#ffe274", "#e9eef4", "#ccfcb2", "#e9eef4"],
    8: ["#bcdbf1", "#e9eef4", "#ccfcb2", "#e9eef4", "#e9eef4", "#ccfcb2", "#e9eef4", "#bcdbf1"],
    9: ["#bcdbf1", "#e9eef4", "#ccfcb2", "#e9eef4", "#ffe274", "#e9eef4", "#ccfcb2", "#e9eef4", "#bcdbf1"],
    10: ["#bcdbf1", "#e9eef4", "#ccfcb2", "#e9eef4", "#ffe274", "#ffe274", "#e9eef4", "#ccfcb2", "#e9eef4", "#bcdbf1"],
    11: ["#bcdbf1", "#e9eef4", "#ccfcb2", "#e9eef4", "#ffe274", "#ff7a5c", "#ffe274", "#e9eef4", "#ccfcb2", "#e9eef4", "#bcdbf1"],
    12: ["#ffe274", "#bcdbf1", "#e9eef4", "#ccfcb2", "#e9eef4", "#ffe274", "#ffe274", "#e9eef4", "#ccfcb2", "#e9eef4", "#bcdbf1", "#ffe274"],
    13: ["#ffe274", "#bcdbf1", "#e9eef4", "#ccfcb2", "#e9eef4", "#ffe274", "#ff7a5c", "#ffe274", "#e9eef4", "#ccfcb2", "#e9eef4", "#bcdbf1", "#ffe274"],
    14: ["#e9eef4", "#ffe274", "#bcdbf1", "#e9eef4", "#ccfcb2", "#e9eef4", "#ffe274", "#ffe274", "#e9eef4", "#ccfcb2", "#e9eef4", "#bcdbf1", "#ffe274", "#e9eef4"],
    15: ["#e9eef4", "#ffe274", "#bcdbf1", "#e9eef4", "#ccfcb2", "#e9eef4", "#ffe274", "#ff7a5c", "#ffe274", "#e9eef4", "#ccfcb2", "#e9eef4", "#bcdbf1", "#ffe274", "#e9eef4"],
    16: ["#ccfcb2", "#e9eef4", "#ffe274", "#bcdbf1", "#e9eef4", "#ccfcb2", "#e9eef4", "#ffe274", "#ffe274", "#e9eef4", "#ccfcb2", "#e9eef4", "#bcdbf1", "#ffe274", "#e9eef4", "#ccfcb2"],
    17: ["#ccfcb2", "#e9eef4", "#ffe274", "#bcdbf1", "#e9eef4", "#ccfcb2", "#e9eef4", "#ffe274", "#ff7a5c", "#ffe274", "#e9eef4", "#ccfcb2", "#e9eef4", "#bcdbf1", "#ffe274", "#e9eef4", "#ccfcb2"],
    18: ["#bcdbf1", "#ccfcb2", "#e9eef4", "#ffe274", "#bcdbf1", "#e9eef4", "#ccfcb2", "#e9eef4", "#ffe274", "#ffe274", "#e9eef4", "#ccfcb2", "#e9eef4", "#bcdbf1", "#ffe274", "#e9eef4", "#ccfcb2", "#bcdbf1"],
}


@dataclass(frozen=True)
class TimingLine:
    time: int
    color: str
    show_label: bool


@dataclass(frozen=True)
class RenderLayout:
    column_count: int
    time_per_column: int
    column_height: int
    total_column_height: int
    lane_area_width: int
    column_width: int
    image_width: int
    image_height: int
    pixels_per_ms: float = PIXELS_PER_MS


def render_mania_grid(
    beatmap: Beatmap,
    output_path: Path,
    mods: ModSettings | None = None,
) -> Path:
    # ── key count 直接取自谱面 CS（mod 不改变原生 mania 列数）──
    key_count = int(float(beatmap.difficulty["CircleSize"]))
    key_count = max(1, min(key_count, max(LANE_COLOR_PALETTES.keys())))
    palette = LANE_COLOR_PALETTES[key_count]

    # ── IN / HO：修改 hit objects ──
    hit_objects = list(beatmap.hit_objects)
    if mods and mods.inverse:
        hit_objects = _apply_inverse_mod(hit_objects, beatmap.timing_points)
    if mods and mods.hold_off:
        hit_objects = _apply_hold_off_mod(hit_objects)

    # ── CS：关闭 SV 变化 ──
    cs_mode = mods is not None and mods.cs_override
    source_mode = beatmap.general.get(SOURCE_MODE_KEY, beatmap.general.get("Mode", "3"))
    is_native_mania = source_mode == "3"

    font_regular = ImageFont.load_default(size=TIME_LABEL_FONT_SIZE)
    font_sv = ImageFont.load_default(size=SV_TEXT_FONT_SIZE)
    beatmap_duration = max(ho.end_time for ho in hit_objects) if hit_objects else 0
    chart_end_time = beatmap_duration + BOTTOM_PADDING_MS
    timing_lines = _build_timing_lines(beatmap.timing_points, chart_end_time)
    sv_changes = [] if cs_mode or not is_native_mania else _build_sv_changes(beatmap.timing_points, chart_end_time)
    layout = _build_layout(key_count, beatmap_duration, chart_end_time)
    render_cache: dict[str, str] = {}

    image = Image.new("RGB", (layout.image_width, layout.image_height), IMAGE_BACKGROUND[:3])
    draw = ImageDraw.Draw(image)

    for column_index in range(layout.column_count):
        _draw_column_background(draw, key_count, column_index, layout)

    for timing_line in timing_lines:
        _draw_timing_line(draw, timing_line, layout, font_regular)

    for sv_change in sv_changes:
        _draw_sv_indicator(draw, sv_change, layout, font_sv)

    for hit_object in hit_objects:
        _draw_hit_object(draw, hit_object, palette, layout, render_cache)

    image.save(output_path, optimize=True)
    return output_path


def _apply_inverse_mod(
    hit_objects: list[ManiaHitObject],
    timing_points: list[TimingPoint],
) -> list[ManiaHitObject]:
    """IN mod: 按 lane 将相邻物件之间改为 hold，末尾物件不保留。"""
    if not hit_objects:
        return []

    by_lane: dict[int, list[ManiaHitObject]] = {}
    for ho in hit_objects:
        by_lane.setdefault(ho.lane, []).append(ho)

    result: list[ManiaHitObject] = []
    for lane, lane_objects in by_lane.items():
        sorted_lane_objects = sorted(lane_objects, key=lambda ho: (ho.start_time, ho.end_time))
        for current, next_object in zip(sorted_lane_objects, sorted_lane_objects[1:]):
            gap = next_object.start_time - current.start_time
            beat_length = _beat_length_at(next_object.start_time, timing_points)
            duration = max(gap / 2, gap - beat_length / 4)
            end_time = max(current.start_time, round(current.start_time + duration))
            result.append(
                ManiaHitObject(
                    lane=lane,
                    start_time=current.start_time,
                    end_time=end_time,
                    is_long_note=end_time > current.start_time,
                )
            )
    return sorted(result, key=lambda ho: (ho.start_time, ho.end_time, ho.lane))


def _apply_hold_off_mod(
    hit_objects: list[ManiaHitObject],
) -> list[ManiaHitObject]:
    """HO mod: 长条只保留头部单点，普通 note 原样保留。"""
    result: list[ManiaHitObject] = []
    for ho in hit_objects:
        result.append(
            ManiaHitObject(
                lane=ho.lane,
                start_time=ho.start_time,
                end_time=ho.start_time,
                is_long_note=False,
            )
        )
    return sorted(result, key=lambda ho: (ho.start_time, ho.end_time, ho.lane))


def _beat_length_at(time: int, timing_points: list[TimingPoint]) -> float:
    beat_length = timing_points[0].beat_length if timing_points else 500.0
    for point in timing_points:
        if point.time > time:
            break
        if point.uninherited:
            beat_length = point.beat_length
    return beat_length


def _darken_hex(color: str, ratio: float, cache: dict[str, str]) -> str:
    key = f"{color}_{ratio}"
    cached = cache.get(key)
    if cached is not None:
        return cached
    channels = [int(color[index : index + 2], 16) for index in (1, 3, 5)]
    darkened = [int(channel * (1 - ratio)) for channel in channels]
    result = "#" + "".join(f"{value:02x}" for value in darkened)
    cache[key] = result
    return result


def _build_layout(
    key_count: int,
    beatmap_duration: int,
    chart_end_time: int,
    pixels_per_ms: float = PIXELS_PER_MS,
) -> RenderLayout:
    total_chart_height = max(1, math.ceil(chart_end_time * pixels_per_ms))
    column_count = _calculate_column_count(beatmap_duration, total_chart_height)
    time_per_column = math.ceil(chart_end_time / column_count)
    column_height = math.ceil(time_per_column * pixels_per_ms)
    total_column_height = TOP_BUFFER + column_height
    lane_area_width = key_count * LANE_WIDTH + (key_count - 1) * LANE_GAP
    column_width = LEFT_PANEL_WIDTH + lane_area_width
    image_width = PAGE_MARGIN_X * 2 + column_count * column_width + column_count * COLUMN_GAP
    image_height = PAGE_MARGIN_Y * 2 + total_column_height
    return RenderLayout(
        column_count=column_count,
        time_per_column=time_per_column,
        column_height=column_height,
        total_column_height=total_column_height,
        lane_area_width=lane_area_width,
        column_width=column_width,
        image_width=image_width,
        image_height=image_height,
        pixels_per_ms=pixels_per_ms,
    )


def _calculate_column_count(beatmap_duration: int, total_chart_height: int) -> int:
    if beatmap_duration >= MAX_SUPPORTED_DURATION_MS:
        raise PreviewError("songs longer than 10 minutes are not supported")

    if beatmap_duration >= 6 * 60 * 1000:
        return FIXED_COLUMN_COUNT_6_TO_10_MIN

    max_area_height = _resolve_max_area_height(beatmap_duration)
    return max(1, math.ceil(total_chart_height / max_area_height))


def _resolve_max_area_height(beatmap_duration: int) -> int:
    if beatmap_duration < 1 * 60 * 1000:
        return MAX_AREA_HEIGHT_0_TO_1_MIN
    if beatmap_duration < 2 * 60 * 1000:
        return MAX_AREA_HEIGHT_1_TO_2_MIN
    if beatmap_duration < 3 * 60 * 1000:
        return MAX_AREA_HEIGHT_2_TO_3_MIN
    if beatmap_duration < 4 * 60 * 1000:
        return MAX_AREA_HEIGHT_3_TO_4_MIN
    if beatmap_duration < 5 * 60 * 1000:
        return MAX_AREA_HEIGHT_4_TO_5_MIN
    return MAX_AREA_HEIGHT_5_TO_6_MIN


def _draw_column_background(
    draw: ImageDraw.ImageDraw,
    key_count: int,
    column_index: int,
    layout: RenderLayout,
) -> None:
    column_left = PAGE_MARGIN_X + column_index * (layout.column_width + COLUMN_GAP)
    chart_top = PAGE_MARGIN_Y
    lane_area_left = column_left + LEFT_PANEL_WIDTH

    draw.rectangle(
        (column_left, chart_top, lane_area_left, chart_top + layout.total_column_height),
        fill=LEFT_PANEL_BACKGROUND,
    )

    for lane_index in range(key_count):
        lane_left = lane_area_left + lane_index * (LANE_WIDTH + LANE_GAP)
        lane_right = lane_left + LANE_WIDTH
        draw.rectangle(
            (lane_left, chart_top, lane_right, chart_top + layout.total_column_height),
            fill=LANE_BACKGROUND,
        )
        if lane_index > 0:
            separator_x = lane_left
            draw.line(
                (separator_x, chart_top, separator_x, chart_top + layout.total_column_height),
                fill=LANE_SEPARATOR,
                width=1,
            )


def _draw_timing_line(
    draw: ImageDraw.ImageDraw,
    timing_line: TimingLine,
    layout: RenderLayout,
    font: ImageFont.ImageFont,
) -> None:
    column_index = min(layout.column_count - 1, timing_line.time // layout.time_per_column)
    local_time = timing_line.time - column_index * layout.time_per_column
    column_left = PAGE_MARGIN_X + column_index * (layout.column_width + COLUMN_GAP)
    lane_area_left = column_left + LEFT_PANEL_WIDTH
    chart_top = PAGE_MARGIN_Y + TOP_BUFFER
    y = chart_top + layout.column_height - round(local_time * layout.pixels_per_ms)

    draw.line(
        (lane_area_left, y, lane_area_left + layout.lane_area_width - 1, y),
        fill=timing_line.color,
        width=1,
    )
    if timing_line.show_label:
        label = f"{timing_line.time / 1000:.1f}s"
        label_box = draw.textbbox((0, 0), label, font=font)
        label_width = label_box[2] - label_box[0]
        text_mid_y = (label_box[1] + label_box[3]) / 2
        label_x = column_left + layout.column_width + 4
        if column_index < layout.column_count - 1:
            next_column_left = column_left + layout.column_width + COLUMN_GAP
            label_x = min(label_x, next_column_left - label_width - 4)
        else:
            label_x = min(label_x, layout.image_width - PAGE_MARGIN_X - label_width)
        label_y = max(chart_top, y - text_mid_y)
        draw.text(
            (label_x, label_y),
            label,
            fill=RULER_TEXT,
            font=font,
        )


def _draw_hit_object(
    draw: ImageDraw.ImageDraw,
    hit_object: ManiaHitObject,
    palette: list[str],
    layout: RenderLayout,
    cache: dict[str, str],
) -> None:
    start_column = min(layout.column_count - 1, hit_object.start_time // layout.time_per_column)
    end_column = min(layout.column_count - 1, hit_object.end_time // layout.time_per_column)
    lane_color = palette[hit_object.lane]
    hold_color = _darken_hex(lane_color, 0.5, cache)

    for column_index in range(start_column, end_column + 1):
        column_left = PAGE_MARGIN_X + column_index * (layout.column_width + COLUMN_GAP)
        lane_area_left = column_left + LEFT_PANEL_WIDTH
        chart_top = PAGE_MARGIN_Y
        chart_axis_top = chart_top + TOP_BUFFER
        chart_bottom = chart_axis_top + layout.column_height
        lane_left = lane_area_left + hit_object.lane * (LANE_WIDTH + LANE_GAP) + NOTE_SIDE_PADDING
        lane_right = lane_left + LANE_WIDTH - NOTE_SIDE_PADDING * 2
        segment_start = max(hit_object.start_time, column_index * layout.time_per_column)
        segment_end = min(hit_object.end_time, (column_index + 1) * layout.time_per_column)
        y_start = chart_axis_top + layout.column_height - round(
            (segment_start - column_index * layout.time_per_column) * layout.pixels_per_ms
        )
        y_end = chart_axis_top + layout.column_height - round(
            (segment_end - column_index * layout.time_per_column) * layout.pixels_per_ms
        )

        if hit_object.is_long_note:
            body_top = max(chart_top, min(y_end, y_start - NOTE_HEAD_HEIGHT))
            body_bottom = min(chart_bottom, y_start)
            if body_top < body_bottom:
                draw.rectangle(
                    (lane_left, body_top, lane_right, body_bottom),
                    fill=hold_color,
                )
            if column_index == start_column:
                head_top = max(chart_top, y_start - NOTE_HEAD_HEIGHT)
                head_bottom = min(chart_bottom, y_start)
                if head_top < head_bottom:
                    draw.rectangle(
                        (lane_left, head_top, lane_right, head_bottom),
                        fill=lane_color,
                    )
        else:
            head_top = max(chart_top, y_start - NOTE_HEAD_HEIGHT)
            head_bottom = min(chart_bottom, y_start)
            if head_top < head_bottom:
                draw.rectangle(
                    (lane_left, head_top, lane_right, head_bottom),
                    fill=lane_color,
                )


def _build_timing_lines(
    timing_points: list[TimingPoint],
    chart_end_time: int,
    pixels_per_ms: float = PIXELS_PER_MS,
) -> list[TimingLine]:
    base_points = [point for point in timing_points if point.uninherited]
    if not base_points:
        return []

    timing_lines: list[TimingLine] = []
    for index, point in enumerate(base_points):
        segment_end = chart_end_time
        if index + 1 < len(base_points):
            segment_end = int(base_points[index + 1].time)

        beat_pixels = point.beat_length * pixels_per_ms
        if beat_pixels >= 72:
            subdivision = 4
        elif beat_pixels >= 28:
            subdivision = 2
        else:
            subdivision = 1
        step = point.beat_length / subdivision
        step_index = 0
        current = point.time

        while current <= segment_end + 0.001:
            if current >= 0:
                is_bar = step_index % (subdivision * point.meter) == 0
                is_beat = step_index % subdivision == 0
                timing_lines.append(
                    TimingLine(
                        time=int(round(current)),
                        color=MEASURE_LINE if is_bar else (BEAT_LINE if is_beat else SUBDIVISION_LINE),
                        show_label=is_bar or is_beat,
                    )
                )
            step_index += 1
            current = point.time + step_index * step

    ordered_unique: dict[int, TimingLine] = {}
    for timing_line in timing_lines:
        ordered_unique[timing_line.time] = timing_line
    return [ordered_unique[time] for time in sorted(ordered_unique)]


def _build_sv_changes(timing_points: list[TimingPoint], chart_end_time: int) -> list[tuple[int, float]]:
    inherited = [
        point for point in timing_points
        if not point.uninherited and point.beat_length < 0 and 0 <= point.time <= chart_end_time
    ]
    if not inherited:
        return []

    changes: list[tuple[int, float]] = []
    prev_sv: float | None = None
    for point in inherited:
        sv = -100.0 / point.beat_length
        if prev_sv is None or abs(sv - prev_sv) > 0.001:
            changes.append((int(point.time), sv))
            prev_sv = sv

    return changes


def _draw_sv_indicator(
    draw: ImageDraw.ImageDraw,
    sv_change: tuple[int, float],
    layout: RenderLayout,
    font: ImageFont.ImageFont,
) -> None:
    time, sv = sv_change
    column_index = min(layout.column_count - 1, time // layout.time_per_column)
    local_time = time - column_index * layout.time_per_column
    column_left = PAGE_MARGIN_X + column_index * (layout.column_width + COLUMN_GAP)
    chart_top = PAGE_MARGIN_Y + TOP_BUFFER
    y = chart_top + layout.column_height - round(local_time * layout.pixels_per_ms)

    if sv == round(sv, 1):
        label = f"{sv:.1f}x"
    else:
        label = f"{sv:.2f}x"
    label_box = draw.textbbox((0, 0), label, font=font)
    label_width = label_box[2] - label_box[0]
    text_mid_y = (label_box[1] + label_box[3]) / 2

    label_x = max(0, column_left - 1 - label_width)
    label_y = max(chart_top, y - text_mid_y)
    draw.text(
        (label_x, label_y),
        label,
        fill=SV_TEXT_COLOR,
        font=font,
    )
