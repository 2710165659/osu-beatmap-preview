from __future__ import annotations

import math
from dataclasses import dataclass
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

from .errors import PreviewError
from .models import Beatmap, ManiaHitObject, TimingPoint

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

PIXELS_PER_MS = 0.4  # 小节线间基础长度
MAX_AREA_HEIGHT_0_TO_1_MIN = 4000  # [0, 1) 分钟最大区域高度
MAX_AREA_HEIGHT_1_TO_2_MIN = 5500  # [1, 2) 分钟最大区域高度
MAX_AREA_HEIGHT_2_TO_3_MIN = 7000  # [2, 3) 分钟最大区域高度
MAX_AREA_HEIGHT_3_TO_4_MIN = 8500  # [3, 4) 分钟最大区域高度
MAX_AREA_HEIGHT_4_TO_5_MIN = 10000  # [4, 5) 分钟最大区域高度
MAX_AREA_HEIGHT_5_TO_6_MIN = 11500  # [5, 6) 分钟最大区域高度
FIXED_COLUMN_COUNT_6_TO_10_MIN = 30  # [6, 10) 分钟固定列数
MAX_SUPPORTED_DURATION_MS = 10 * 60 * 1000  # 支持渲染的最大谱面时长
PAGE_MARGIN_X = 20  # 图片左右外边距
PAGE_MARGIN_Y = 20  # 图片上下外边距
LANE_WIDTH = 38  # 单个轨道宽度
LANE_GAP = 0  # 轨道之间间距
COLUMN_GAP = 100  # 列与列之间间距
NOTE_HEAD_HEIGHT = 15  # note 头部高度
BOTTOM_PADDING_MS = 2000  # 谱面底部额外预留时间
TOP_BUFFER = NOTE_HEAD_HEIGHT  # 顶部额外缓冲高度
LEFT_PANEL_WIDTH = 12  # 轨道左侧区域宽度
LEFT_PANEL_BACKGROUND = (112, 112, 112, 255)  # 轨道左侧区域背景色
IMAGE_BACKGROUND = (0, 0, 0, 255)  # 整体背景色
LANE_BACKGROUND = (0, 0, 0, 255)  # 轨道背景色
RULER_TEXT = (232, 232, 232, 255)  # 时间文字颜色
MEASURE_LINE = (220, 220, 220, 96)  # 小节线颜色
BEAT_LINE = (200, 200, 200, 72)  # 拍线颜色
SUBDIVISION_LINE = (180, 180, 180, 48)  # 细分节拍线颜色
LANE_SEPARATOR = (32, 32, 32, 255)  # 轨道分隔线颜色
NOTE_SIDE_PADDING = 2  # note 左右内边距


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


def render_mania_preview(
    beatmap: Beatmap,
    output_path: Path,
) -> Path:
    # 将完整 mania 谱面渲染为单张图片，长谱面按列切分。
    try:
        key_count = int(float(beatmap.difficulty["CircleSize"]))
        palette = LANE_COLOR_PALETTES[key_count]
        font_regular = ImageFont.load_default()
        beatmap_duration = max(hit_object.end_time for hit_object in beatmap.hit_objects)
        chart_end_time = beatmap_duration + BOTTOM_PADDING_MS
        timing_lines = _build_timing_lines(beatmap.timing_points, chart_end_time)
        layout = _build_layout(key_count, beatmap_duration, chart_end_time)

        image = Image.new("RGBA", (layout.image_width, layout.image_height), IMAGE_BACKGROUND)
        draw = ImageDraw.Draw(image)

        for column_index in range(layout.column_count):
            _draw_column_background(draw, key_count, column_index, layout)

        for timing_line in timing_lines:
            _draw_timing_line(draw, timing_line, layout, font_regular)

        for hit_object in beatmap.hit_objects:
            _draw_hit_object(draw, hit_object, palette, layout)

        output_path.parent.mkdir(parents=True, exist_ok=True)
        image.save(output_path)
        return output_path
    except PreviewError:
        raise
    except (OSError, KeyError, ValueError, IndexError, ZeroDivisionError) as exc:
        raise PreviewError("Failed to render preview.") from exc


def _darken_hex(color: str, ratio: float) -> str:
    channels = [int(color[index : index + 2], 16) for index in (1, 3, 5)]
    darkened = [int(channel * (1 - ratio)) for channel in channels]
    return "#" + "".join(f"{value:02x}" for value in darkened)


def _build_layout(key_count: int, beatmap_duration: int, chart_end_time: int) -> RenderLayout:
    total_chart_height = max(1, math.ceil(chart_end_time * PIXELS_PER_MS))
    column_count = _calculate_column_count(beatmap_duration, total_chart_height)
    # 按列数反推每列时间跨度，保证最终图片高度被限制在可读范围内。
    time_per_column = math.ceil(chart_end_time / column_count)
    column_height = math.ceil(time_per_column * PIXELS_PER_MS)
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
    )


def _calculate_column_count(beatmap_duration: int, total_chart_height: int) -> int:
    if beatmap_duration >= MAX_SUPPORTED_DURATION_MS:
        raise PreviewError("songs longer than 10 minutes are not supported")

    if beatmap_duration >= 6 * 60 * 1000:
        # 长谱固定列数，避免图片高度继续线性增长。
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
    # mania 视图时间从下往上推进，所以时间越晚 y 越小。
    y = chart_top + layout.column_height - round(local_time * PIXELS_PER_MS)

    draw.line(
        (lane_area_left, y, lane_area_left + layout.lane_area_width - 1, y),
        fill=timing_line.color,
        width=1,
    )
    if timing_line.show_label:
        label = f"{timing_line.time / 1000:.1f}s"
        label_box = draw.textbbox((0, 0), label, font=font)
        label_width = label_box[2] - label_box[0]
        label_x = column_left + layout.column_width + 4
        if column_index < layout.column_count - 1:
            next_column_left = column_left + layout.column_width + COLUMN_GAP
            label_x = min(label_x, next_column_left - label_width - 4)
        else:
            label_x = min(label_x, layout.image_width - PAGE_MARGIN_X - label_width)
        draw.text(
            (label_x, max(chart_top, y - 6)),
            label,
            fill=RULER_TEXT,
            font=font,
        )


def _draw_hit_object(
    draw: ImageDraw.ImageDraw,
    hit_object: ManiaHitObject,
    palette: list[str],
    layout: RenderLayout,
) -> None:
    start_column = min(layout.column_count - 1, hit_object.start_time // layout.time_per_column)
    end_column = min(layout.column_count - 1, hit_object.end_time // layout.time_per_column)
    lane_color = palette[hit_object.lane]
    hold_color = _darken_hex(lane_color, 0.5)

    for column_index in range(start_column, end_column + 1):
        column_left = PAGE_MARGIN_X + column_index * (layout.column_width + COLUMN_GAP)
        lane_area_left = column_left + LEFT_PANEL_WIDTH
        chart_top = PAGE_MARGIN_Y
        chart_axis_top = chart_top + TOP_BUFFER
        chart_bottom = chart_axis_top + layout.column_height
        lane_left = lane_area_left + hit_object.lane * (LANE_WIDTH + LANE_GAP) + NOTE_SIDE_PADDING
        lane_right = lane_left + LANE_WIDTH - NOTE_SIDE_PADDING * 2
        # 长条可能跨列，逐列裁剪当前列实际需要绘制的时间片段。
        segment_start = max(hit_object.start_time, column_index * layout.time_per_column)
        segment_end = min(hit_object.end_time, (column_index + 1) * layout.time_per_column)
        y_start = chart_axis_top + layout.column_height - round(
            (segment_start - column_index * layout.time_per_column) * PIXELS_PER_MS
        )
        y_end = chart_axis_top + layout.column_height - round(
            (segment_end - column_index * layout.time_per_column) * PIXELS_PER_MS
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


def _build_timing_lines(timing_points: list[TimingPoint], chart_end_time: int) -> list[TimingLine]:
    # 只有红线 timing point 会生成可见的小节线和拍线。
    base_points = [point for point in timing_points if point.uninherited]
    if not base_points:
        return []

    timing_lines: list[TimingLine] = []
    for index, point in enumerate(base_points):
        segment_end = chart_end_time
        if index + 1 < len(base_points):
            segment_end = int(base_points[index + 1].time)

        subdivision = _choose_subdivision(point.beat_length)
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
                        color=_timing_line_color(is_bar, is_beat),
                        show_label=is_bar or is_beat,
                    )
                )
            step_index += 1
            current = point.time + step_index * step

    ordered_unique: dict[int, TimingLine] = {}
    for timing_line in timing_lines:
        # 多个 timing section 边界可能生成同一毫秒的线，后写入的保留即可。
        ordered_unique[timing_line.time] = timing_line
    return [ordered_unique[time] for time in sorted(ordered_unique)]


def _choose_subdivision(beat_length: float) -> int:
    beat_pixels = beat_length * PIXELS_PER_MS
    # BPM 越快，拍线越密；减少细分能防止画面被网格线淹没。
    if beat_pixels >= 72:
        return 4
    if beat_pixels >= 28:
        return 2
    return 1


def _timing_line_color(is_bar: bool, is_beat: bool) -> str:
    if is_bar:
        return MEASURE_LINE
    if is_beat:
        return BEAT_LINE
    return SUBDIVISION_LINE
