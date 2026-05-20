from __future__ import annotations

from bisect import bisect_right
from dataclasses import dataclass

from PIL import Image, ImageDraw, ImageFont

from ..errors import PreviewError
from ..models import Beatmap, ManiaHitObject, TimingPoint
from ..mods import ModSettings
from ..time_selection import PreviewTimeSelector, times_to_milliseconds
from .config import (
    GIF_DEFAULT_HIT_POSITION,
    GIF_DURATION_MS,
    GIF_FPS,
    GIF_FRAME_HEIGHT,
    GIF_GRID_GAP,
    GIF_HIT_TARGET_FROM_BOTTOM,
    GIF_JUDGEMENT_LINE,
    GIF_LOOP,
    GIF_MAX_TIME_RANGE,
    GIF_PREVIEW_TIME_LABEL_COLOR,
    GIF_SCROLL_SPEED,
    GIF_SEGMENT_COUNT,
    GIF_SEPARATOR_BACKGROUND,
    GIF_SEPARATOR_WIDTH,
    GIF_STAGE_TOP_PADDING,
    GIF_TIME_LABEL_COLOR,
    GIF_TIME_LABEL_FONT_SIZE,
    GIF_TIME_LABEL_HEIGHT,
    GIF_TIME_LABEL_NOTE_COLOR,
    GIF_TIME_LABEL_NOTE_FONT_SIZE,
    GIF_TIME_LABEL_TOP_GAP,
    IMAGE_BACKGROUND,
    LANE_BACKGROUND,
    LANE_WIDTH,
    LEFT_PANEL_BACKGROUND,
    LEFT_PANEL_WIDTH,
    NOTE_HEAD_HEIGHT,
    NOTE_SIDE_PADDING,
    PAGE_MARGIN_X,
    PAGE_MARGIN_Y,
    SV_TEXT_COLOR,
    SV_TEXT_FONT_SIZE,
)
from .renderer import LANE_COLOR_PALETTES, _apply_hold_off_mod, _apply_inverse_mod, _darken_hex
from .convert import SOURCE_MODE_KEY
from .skin import ManiaSkinConfig, load_mania_skin_config


@dataclass(frozen=True)
class GifLayout:
    """GIF 专用布局；不要复用 PNG 的 RenderLayout，避免影响静态图显示。"""

    segment_count: int
    segment_width: int
    playfield_height: int
    lane_area_width: int
    image_width: int
    image_height: int
    hit_position_y: int
    scroll_length: int
    note_head_height: int
    column_left_offsets: tuple[int, ...]
    column_widths: tuple[int, ...]
    column_colours: tuple[tuple[int, int, int, int], ...]


@dataclass(frozen=True)
class ScrollMap:
    """把谱面时间映射为顺序滚动距离，用于处理 BPM 和 SV 变化。"""

    starts: tuple[float, ...]
    positions: tuple[float, ...]
    multipliers: tuple[float, ...]

    def position_at(self, time: float) -> float:
        # starts/positions 是分段前缀表，二分后只需计算当前段内的增量。
        index = bisect_right(self.starts, time) - 1
        if index < 0:
            index = 0
        return self.positions[index] + (time - self.starts[index]) * self.multipliers[index]


def render_mania_gif(
    beatmap: Beatmap,
    mods: ModSettings | None = None,
    times: list[float] | None = None,
):
    key_count, palette, hit_objects, cs_mode = _prepare_render_data(beatmap, mods)
    if not hit_objects:
        raise PreviewError("mania beatmap has no hit objects")

    # 先选 4 个片段开始时间：PreviewTime 必选，其余随机并尽量避开 break。
    # DT/HT 只改变谱面时间推进；GIF 实际播放时长仍固定为 5 秒。
    speed_multiplier = mods.speed_multiplier if mods is not None else 1.0
    gameplay_segment_duration = round(GIF_DURATION_MS * speed_multiplier)
    segment_timings = PreviewTimeSelector(
        beatmap=beatmap,
        hit_objects=hit_objects,
        segment_count=GIF_SEGMENT_COUNT,
        segment_duration=gameplay_segment_duration,
        requested_start_times=times_to_milliseconds(times),
    ).choose()

    skin_config = load_mania_skin_config(key_count)
    layout = _build_layout(skin_config)
    source_mode = beatmap.general.get(SOURCE_MODE_KEY, beatmap.general.get("Mode", "3"))
    is_native_mania = source_mode == "3"
    # CS 是 Constant Scroll：保留 33 速时间窗，但不叠加 SV 变速。
    scroll_map = _build_scroll_map(beatmap, constant=cs_mode, allow_sv=is_native_mania)
    # time_range 表示 33 速下从判定线到顶部可容纳的谱面时间。
    time_range = _compute_time_range(speed_multiplier, skin_config.hit_position)
    pixels_per_scroll_unit = layout.scroll_length / time_range
    frame_count = max(1, round(GIF_DURATION_MS * GIF_FPS / 1000))
    frame_duration_ms = max(1, round(1000 / GIF_FPS))
    max_segment_end = max(timing.start_time + gameplay_segment_duration for timing in segment_timings)
    sv_changes = [] if cs_mode or not is_native_mania else _build_sv_changes(beatmap.timing_points, max_segment_end + round(time_range))

    font_regular = ImageFont.load_default(size=GIF_TIME_LABEL_FONT_SIZE)
    font_note = ImageFont.load_default(size=GIF_TIME_LABEL_NOTE_FONT_SIZE)
    font_sv = ImageFont.load_default(size=SV_TEXT_FONT_SIZE)
    segment_snapshot_times = [
        tuple(timing.start_time + round(frame_index * 1000 * speed_multiplier / GIF_FPS) for frame_index in range(frame_count))
        for timing in segment_timings
    ]

    def frame_generator():
        for frame_index in range(frame_count):
            canvas = Image.new("RGBA", (layout.image_width, layout.image_height), IMAGE_BACKGROUND)
            draw = ImageDraw.Draw(canvas)
            # 先画列间分割区域，再逐段覆盖轨道背景、SV 标记、note 和时间标签。
            _draw_segment_separators(draw, layout)

            for segment_index, segment_timing in enumerate(segment_timings):
                segment_left = _segment_left(segment_index, layout)
                snapshot_time = segment_snapshot_times[segment_index][frame_index]
                _draw_segment_background(draw, segment_left, layout)
                _draw_sv_indicators(
                    draw,
                    sv_changes,
                    segment_left,
                    snapshot_time,
                    layout,
                    scroll_map,
                    pixels_per_scroll_unit,
                    font_sv,
                )
                _draw_hit_objects(
                    draw,
                    hit_objects,
                    palette,
                    segment_left,
                    snapshot_time,
                    layout,
                    scroll_map,
                    pixels_per_scroll_unit,
                )
                _draw_time_label(
                    draw,
                    segment_timing.start_time,
                    gameplay_segment_duration,
                    segment_left,
                    layout,
                    font_regular,
                    font_note,
                    segment_timing.is_preview,
                )

            yield canvas

    return frame_generator(), frame_duration_ms, GIF_LOOP


def _prepare_render_data(
    beatmap: Beatmap,
    mods: ModSettings | None,
) -> tuple[int, list[str], list[ManiaHitObject], bool]:
    # GIF 仅处理 mania 物件；note 配色复用 PNG 的 palette，保持样式一致。
    key_count = int(float(beatmap.difficulty["CircleSize"]))
    key_count = max(1, min(key_count, max(LANE_COLOR_PALETTES.keys())))
    hit_objects = [ho for ho in beatmap.hit_objects if isinstance(ho, ManiaHitObject)]
    # IN/HO 改物件形态，和 PNG 保持同一套辅助逻辑。
    if mods and mods.inverse:
        hit_objects = _apply_inverse_mod(hit_objects, beatmap.timing_points)
    if mods and mods.hold_off:
        hit_objects = _apply_hold_off_mod(hit_objects)
    return key_count, LANE_COLOR_PALETTES[key_count], hit_objects, mods is not None and mods.cs_override


def _build_layout(skin_config: ManiaSkinConfig) -> GifLayout:
    # 列宽、列线宽、背景色只从 mania skin 读取；PNG 不经过这里。
    column_left_offsets = _build_column_left_offsets(skin_config.column_widths, skin_config.column_line_widths)
    lane_area_width = sum(skin_config.column_widths) + sum(skin_config.column_line_widths)
    segment_width = LEFT_PANEL_WIDTH * 2 + lane_area_width
    playfield_height = GIF_FRAME_HEIGHT
    hit_position_y = round(playfield_height - skin_config.hit_position)
    scroll_length = max(1, hit_position_y - GIF_STAGE_TOP_PADDING)
    average_column_width = sum(skin_config.column_widths) / len(skin_config.column_widths)
    # PNG 原比例是 38px 轨道配 15px note；GIF 按 skin 列宽等比放大，避免宽列 note 显得过扁。
    note_head_height = max(1, round(NOTE_HEAD_HEIGHT * average_column_width / LANE_WIDTH))
    image_width = PAGE_MARGIN_X * 2 + GIF_SEGMENT_COUNT * segment_width + (GIF_SEGMENT_COUNT - 1) * GIF_GRID_GAP
    image_height = PAGE_MARGIN_Y * 2 + playfield_height + GIF_TIME_LABEL_TOP_GAP + GIF_TIME_LABEL_HEIGHT
    return GifLayout(
        segment_count=GIF_SEGMENT_COUNT,
        segment_width=segment_width,
        playfield_height=playfield_height,
        lane_area_width=lane_area_width,
        image_width=image_width,
        image_height=image_height,
        hit_position_y=hit_position_y,
        scroll_length=scroll_length,
        note_head_height=note_head_height,
        column_left_offsets=column_left_offsets,
        column_widths=skin_config.column_widths,
        column_colours=skin_config.column_colours,
    )


def _build_column_left_offsets(
    column_widths: tuple[int, ...],
    column_line_widths: tuple[int, ...],
) -> tuple[int, ...]:
    # ColumnLineWidth 有 keys + 1 个值，分别表示最左、列间、最右的线宽。
    offsets: list[int] = []
    cursor = column_line_widths[0] if column_line_widths else 0
    for index, width in enumerate(column_widths):
        offsets.append(cursor)
        cursor += width
        if index + 1 < len(column_line_widths):
            cursor += column_line_widths[index + 1]
    return tuple(offsets)


def _compute_time_range(speed_multiplier: float, hit_position: float) -> float:
    # 对齐 DrawableManiaRuleset.updateTimeRange()：33 速基础时间窗按 HitPosition 修正。
    hit_position_scale = (GIF_FRAME_HEIGHT - hit_position) / (GIF_FRAME_HEIGHT - GIF_DEFAULT_HIT_POSITION)
    return max(1.0, GIF_MAX_TIME_RANGE / GIF_SCROLL_SPEED * hit_position_scale * speed_multiplier)


def _build_scroll_map(beatmap: Beatmap, constant: bool, allow_sv: bool) -> ScrollMap:
    if constant:
        # CS mod 下所有时间按匀速滚动，不使用谱面的绿线 SV。
        return ScrollMap(starts=(0.0,), positions=(0.0,), multipliers=(1.0,))

    # 顺序滚动距离需要累积红线 BPM 和绿线 SV，绘制时再从距离差换成像素。
    timing_points = beatmap.timing_points
    starts: list[float] = []
    multipliers: list[float] = []
    current_beat_length = _most_common_beat_length(timing_points, beatmap.hit_objects)
    current_scroll_speed = 1.0
    base_beat_length = current_beat_length

    for point in timing_points:
        if point.uninherited:
            # 红线切换 BPM，同时重置当前分段使用的 scroll speed。
            current_beat_length = point.beat_length
            current_scroll_speed = 1.0
        elif allow_sv and point.beat_length < 0:
            # 绿线 beat_length 为负数，osu! 用 -100 / beat_length 表示 SV 倍率。
            current_scroll_speed = -100.0 / point.beat_length
        else:
            continue
        starts.append(float(point.time))
        multipliers.append(current_scroll_speed * base_beat_length / current_beat_length)

    if not starts:
        starts = [0.0]
        multipliers = [1.0]
    elif starts[0] > 0:
        starts.insert(0, 0.0)
        multipliers.insert(0, multipliers[0])

    positions = [0.0]
    for index in range(1, len(starts)):
        positions.append(positions[-1] + (starts[index] - starts[index - 1]) * multipliers[index - 1])
    return ScrollMap(starts=tuple(starts), positions=tuple(positions), multipliers=tuple(multipliers))


def _most_common_beat_length(timing_points: list[TimingPoint], hit_objects: list[ManiaHitObject]) -> float:
    red_lines = [point for point in timing_points if point.uninherited and point.beat_length > 0]
    if not red_lines:
        return 500.0

    if hit_objects:
        last_time = max(hit_object.end_time for hit_object in hit_objects)
    else:
        last_time = red_lines[-1].time

    buckets: dict[float, float] = {}
    for index, point in enumerate(red_lines):
        if point.time > last_time:
            duration = 0.0
        else:
            current_time = 0.0 if index == 0 else point.time
            next_time = last_time if index == len(red_lines) - 1 else red_lines[index + 1].time
            duration = max(0.0, next_time - current_time)

        key = round(point.beat_length * 1000.0) / 1000.0
        buckets[key] = buckets.get(key, 0.0) + duration

    most_common = max(buckets.items(), key=lambda item: item[1])[0]
    min_beat_length = min(point.beat_length for point in red_lines)
    max_beat_length = max(point.beat_length for point in red_lines)
    return max(min_beat_length, min(max_beat_length, most_common))


def _segment_left(segment_index: int, layout: GifLayout) -> int:
    return PAGE_MARGIN_X + segment_index * (layout.segment_width + GIF_GRID_GAP)


def _draw_segment_separators(draw: ImageDraw.ImageDraw, layout: GifLayout) -> None:
    # 用户要求列与列之间有更宽间隔，并在间隔中央保留一条分割区域。
    playfield_top = PAGE_MARGIN_Y
    playfield_bottom = playfield_top + layout.playfield_height
    for segment_index in range(layout.segment_count - 1):
        left_segment_right = _segment_left(segment_index, layout) + layout.segment_width
        separator_left = left_segment_right + (GIF_GRID_GAP - GIF_SEPARATOR_WIDTH) // 2
        draw.rectangle(
            (separator_left, playfield_top, separator_left + GIF_SEPARATOR_WIDTH, playfield_bottom),
            fill=GIF_SEPARATOR_BACKGROUND,
        )


def _draw_segment_background(
    draw: ImageDraw.ImageDraw,
    segment_left: int,
    layout: GifLayout,
) -> None:
    # GIF 不画小节线、拍线、轨道分隔线，只保留左右灰色侧栏和判定线。
    playfield_top = PAGE_MARGIN_Y
    playfield_bottom = playfield_top + layout.playfield_height
    lane_area_left = segment_left + LEFT_PANEL_WIDTH
    lane_area_right = lane_area_left + layout.lane_area_width

    draw.rectangle((segment_left, playfield_top, segment_left + layout.segment_width, playfield_bottom), fill=LANE_BACKGROUND)
    draw.rectangle((segment_left, playfield_top, lane_area_left, playfield_bottom), fill=LEFT_PANEL_BACKGROUND)
    draw.rectangle((lane_area_right, playfield_top, segment_left + layout.segment_width, playfield_bottom), fill=LEFT_PANEL_BACKGROUND)

    for lane_index, lane_width in enumerate(layout.column_widths):
        lane_left = lane_area_left + layout.column_left_offsets[lane_index]
        draw.rectangle((lane_left, playfield_top, lane_left + lane_width, playfield_bottom), fill=layout.column_colours[lane_index])

    judgement_y = playfield_top + layout.hit_position_y
    draw.line((segment_left, judgement_y, segment_left + layout.segment_width, judgement_y), fill=GIF_JUDGEMENT_LINE, width=2)


def _draw_sv_indicators(
    draw: ImageDraw.ImageDraw,
    sv_changes: list[tuple[int, float]],
    segment_left: int,
    snapshot_time: int,
    layout: GifLayout,
    scroll_map: ScrollMap,
    pixels_per_scroll_unit: float,
    font: ImageFont.ImageFont,
) -> None:
    # SV 文本画在左侧灰色区域附近，只提示变速点，不画对应横线。
    for time, sv in sv_changes:
        y = _y_at_time(time, snapshot_time, layout, scroll_map, pixels_per_scroll_unit)
        if y < PAGE_MARGIN_Y or y > PAGE_MARGIN_Y + layout.playfield_height:
            continue
        label = f"{sv:.1f}x" if sv == round(sv, 1) else f"{sv:.2f}x"
        box = draw.textbbox((0, 0), label, font=font)
        x = max(0, segment_left + LEFT_PANEL_WIDTH - (box[2] - box[0]) - 3)
        draw.text((x, y - (box[3] - box[1]) / 2), label, fill=SV_TEXT_COLOR, font=font)


def _draw_hit_objects(
    draw: ImageDraw.ImageDraw,
    hit_objects: list[ManiaHitObject],
    palette: list[str],
    segment_left: int,
    snapshot_time: int,
    layout: GifLayout,
    scroll_map: ScrollMap,
    pixels_per_scroll_unit: float,
) -> None:
    # 每帧缓存 LN 暗色，效果与 PNG 的 _darken_hex(lane_color, 0.5) 一致。
    color_cache: dict[str, str] = {}
    for hit_object in hit_objects:
        _draw_hit_object(
            draw,
            hit_object,
            palette,
            segment_left,
            snapshot_time,
            layout,
            scroll_map,
            pixels_per_scroll_unit,
            color_cache,
        )


def _draw_hit_object(
    draw: ImageDraw.ImageDraw,
    hit_object: ManiaHitObject,
    palette: list[str],
    segment_left: int,
    snapshot_time: int,
    layout: GifLayout,
    scroll_map: ScrollMap,
    pixels_per_scroll_unit: float,
    color_cache: dict[str, str],
) -> None:
    y_start = _y_at_time(hit_object.start_time, snapshot_time, layout, scroll_map, pixels_per_scroll_unit)
    y_end = _y_at_time(hit_object.end_time, snapshot_time, layout, scroll_map, pixels_per_scroll_unit)
    playfield_top = PAGE_MARGIN_Y
    playfield_bottom = playfield_top + layout.playfield_height
    if max(y_start, y_end) < playfield_top - layout.note_head_height or min(y_start, y_end) > playfield_bottom + layout.note_head_height:
        return

    lane_color = palette[hit_object.lane]
    # LN 身体颜色沿用 PNG 当前效果，不读取 skin.ini 的 ColourHold。
    hold_color = _darken_hex(lane_color, 0.5, color_cache)
    lane_left = segment_left + LEFT_PANEL_WIDTH + layout.column_left_offsets[hit_object.lane] + NOTE_SIDE_PADDING
    lane_right = lane_left + layout.column_widths[hit_object.lane] - NOTE_SIDE_PADDING * 2

    if hit_object.is_long_note:
        # 长条身体从尾部延伸到头部下沿，头部仍单独按 note 颜色绘制。
        body_top = max(playfield_top, min(y_end, y_start - layout.note_head_height))
        body_bottom = min(playfield_bottom, y_start)
        if body_top < body_bottom:
            draw.rectangle((lane_left, body_top, lane_right, body_bottom), fill=hold_color)

    head_top = max(playfield_top, y_start - layout.note_head_height)
    head_bottom = min(playfield_bottom, y_start)
    if head_top < head_bottom:
        draw.rectangle((lane_left, head_top, lane_right, head_bottom), fill=lane_color)


def _y_at_time(
    time: int | float,
    snapshot_time: int,
    layout: GifLayout,
    scroll_map: ScrollMap,
    pixels_per_scroll_unit: float,
) -> int:
    # 下落模式中，未来物件在判定线上方，过去物件在判定线下方。
    distance = scroll_map.position_at(time) - scroll_map.position_at(snapshot_time)
    return PAGE_MARGIN_Y + layout.hit_position_y - round(distance * pixels_per_scroll_unit)


def _draw_time_label(
    draw: ImageDraw.ImageDraw,
    start_time: int,
    duration_ms: int,
    segment_left: int,
    layout: GifLayout,
    font_regular: ImageFont.ImageFont,
    font_note: ImageFont.ImageFont,
    is_preview: bool,
) -> None:
    # 时间标签位于轨道下方，PreviewTime 使用高亮颜色。
    y = PAGE_MARGIN_Y + layout.playfield_height + GIF_TIME_LABEL_TOP_GAP
    label = f"{_format_time(start_time)} - {_format_time(start_time + duration_ms)}"
    color = GIF_PREVIEW_TIME_LABEL_COLOR if is_preview else GIF_TIME_LABEL_COLOR
    note_color = GIF_PREVIEW_TIME_LABEL_COLOR if is_preview else GIF_TIME_LABEL_NOTE_COLOR
    box = draw.textbbox((0, 0), label, font=font_regular)
    x = segment_left + (layout.segment_width - (box[2] - box[0])) / 2
    draw.text((x, y), label, fill=color, font=font_regular)

    if is_preview:
        note = "Preview Time"
        note_box = draw.textbbox((0, 0), note, font=font_note)
        note_x = segment_left + (layout.segment_width - (note_box[2] - note_box[0])) / 2
        draw.text((note_x, y + (box[3] - box[1]) + 4), note, fill=note_color, font=font_note)


def _format_time(ms: int) -> str:
    total_seconds = max(0, ms) // 1000
    return f"{total_seconds // 60}:{total_seconds % 60:02d}"


def _build_sv_changes(timing_points: list[TimingPoint], chart_end_time: int) -> list[tuple[int, float]]:
    # 只记录 SV 变化点，连续相同倍率不重复显示。
    inherited = [
        point for point in timing_points
        if not point.uninherited and point.beat_length < 0 and 0 <= point.time <= chart_end_time
    ]
    changes: list[tuple[int, float]] = []
    prev_sv: float | None = None
    for point in inherited:
        sv = -100.0 / point.beat_length
        if prev_sv is None or abs(sv - prev_sv) > 0.001:
            changes.append((int(point.time), sv))
            prev_sv = sv
    return changes
