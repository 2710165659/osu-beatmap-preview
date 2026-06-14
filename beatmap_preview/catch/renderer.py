from __future__ import annotations

import math
from dataclasses import dataclass

from PIL import Image, ImageDraw, ImageFont

from pathlib import Path

from ..errors import PreviewError
from ..models import Beatmap, CatchHitObject, TimingPoint
from ..mods import ModSettings
from .config import (
    BEAT_LINE,
    COLUMN_GAP,
    COLUMN_WIDTH,
    DRAW_CATCHER_EACH_COLUMN,
    FIXED_COLUMN_COUNT_6_TO_10_MIN,
    FRUIT_HYPER_GLOW_ALPHA,
    FRUIT_HYPER_GLOW_SCALE,
    IMAGE_BACKGROUND,
    LEFT_PANEL_BACKGROUND,
    LEFT_PANEL_WIDTH,
    KIAI_TIME_LABEL_COLOR,
    MAX_AREA_HEIGHT_0_TO_1_MIN,
    MAX_AREA_HEIGHT_1_TO_2_MIN,
    MAX_AREA_HEIGHT_2_TO_3_MIN,
    MAX_AREA_HEIGHT_3_TO_4_MIN,
    MAX_AREA_HEIGHT_4_TO_5_MIN,
    MAX_AREA_HEIGHT_5_TO_6_MIN,
    MAX_SUPPORTED_DURATION_MS,
    MEASURE_LINE,
    OBJECT_BOTTOM_PADDING,
    OBJECT_RADIUS,
    PAGE_MARGIN_X,
    PAGE_MARGIN_Y,
    PLAYFIELD_BACKGROUND,
    PLAYFIELD_BORDER,
    PLAYFIELD_SIDE_PADDING,
    PLAYFIELD_WIDTH,
    RULER_TEXT,
    STABLE_CATCHER_Y,
    STABLE_FRUIT_START_Y,
    TIME_LABEL_FONT_SIZE,
    TIME_LABEL_NOTE_GAP,
    TOP_BUFFER,
    LEGACY_CATCHER_ORIGIN_Y,
    LEGACY_CATCHER_VISUAL_SCALE,
)
from .objects import CatchRenderObject, build_catch_render_objects
from .skin import CatchSkin, load_catch_skin
from .slider_path import _build_slider_path_cached


@dataclass(frozen=True)
class TimingLine:
    time: int
    color: tuple[int, int, int, int]
    label_color: tuple[int, int, int, int]
    note: str | None = None


@dataclass(frozen=True)
class CatcherMetrics:
    width: int
    height: int
    origin_y: int
    bottom_extent: int


@dataclass(frozen=True)
class RenderLayout:
    column_count: int
    time_per_column: int
    column_height: int
    total_column_height: int
    image_width: int
    image_height: int
    playfield_scale: float
    object_scale: float
    side_padding: int
    visible_playfield_width: int
    bottom_area_height: int
    catcher_metrics: CatcherMetrics
    pixels_per_ms: float


def render_catch_grid(
    beatmap: Beatmap,
    output_path: Path,
    mods: ModSettings | None = None,
) -> Path:
    """把 osu!catch 谱面渲染为纵向多列预览图。"""
    _build_slider_path_cached.cache_clear()
    try:
        hit_objects = [ho for ho in beatmap.hit_objects if isinstance(ho, CatchHitObject)]
        if not hit_objects:
            raise PreviewError("catch beatmap has no hit objects")

        skin = load_catch_skin()
        render_cache: dict = {}
        effective_difficulty = _effective_difficulty(beatmap, mods)
        render_objects = build_catch_render_objects(beatmap, hit_objects, skin.combo_colors, mods=mods, difficulty=effective_difficulty)
        chart_end_time = max(1, max(hit_object.end_time for hit_object in hit_objects))
        timing_lines = _build_timing_lines(beatmap.timing_points, chart_end_time)
        layout = _build_layout(chart_end_time, effective_difficulty["CircleSize"], effective_difficulty["ApproachRate"], skin)
        font_regular = ImageFont.load_default(size=TIME_LABEL_FONT_SIZE)

        image = Image.new("RGB", (layout.image_width, layout.image_height), IMAGE_BACKGROUND[:3])
        draw = ImageDraw.Draw(image)

        for column_index in range(layout.column_count):
            _draw_column_background(draw, layout, column_index)
            if DRAW_CATCHER_EACH_COLUMN or column_index == 0:
                _draw_catcher(image, skin, layout, column_index, render_cache)

        for timing_line in timing_lines:
            _draw_timing_line(draw, timing_line, layout, font_regular)

        for catch_object in sorted(render_objects, key=lambda item: (-item.start_time, _object_order(item.object_type))):
            _draw_catch_object(image, skin, catch_object, layout, render_cache)

        image.save(output_path, optimize=True)
        return output_path
    finally:
        _build_slider_path_cached.cache_clear()


def _build_layout(
    beatmap_duration: int,
    circle_size: float,
    approach_rate: float,
    skin: CatchSkin,
) -> RenderLayout:
    if beatmap_duration >= MAX_SUPPORTED_DURATION_MS:
        raise PreviewError("songs longer than 10 minutes are not supported")

    playfield_scale = COLUMN_WIDTH / PLAYFIELD_WIDTH
    object_scale = _circle_scale(circle_size)
    side_padding = round(PLAYFIELD_SIDE_PADDING * playfield_scale)
    visible_playfield_width = COLUMN_WIDTH + side_padding * 2
    pixels_per_ms = _pixels_per_ms_for_ar(approach_rate, playfield_scale)
    total_chart_height = max(1, math.ceil(beatmap_duration * pixels_per_ms))
    column_count = _calculate_column_count(beatmap_duration, total_chart_height)
    time_per_column = max(1, math.ceil(beatmap_duration / column_count))
    column_height = max(1, math.ceil(time_per_column * pixels_per_ms))
    catcher_metrics = _build_catcher_metrics(circle_size, playfield_scale, skin)
    bottom_area_height = max(catcher_metrics.bottom_extent, OBJECT_BOTTOM_PADDING)
    total_column_height = TOP_BUFFER + column_height + bottom_area_height
    image_width = (
        PAGE_MARGIN_X * 2
        + column_count * (LEFT_PANEL_WIDTH + visible_playfield_width)
        + (column_count - 1) * COLUMN_GAP
    )
    image_height = PAGE_MARGIN_Y * 2 + total_column_height
    return RenderLayout(
        column_count=column_count,
        time_per_column=time_per_column,
        column_height=column_height,
        total_column_height=total_column_height,
        image_width=image_width,
        image_height=image_height,
        playfield_scale=playfield_scale,
        object_scale=object_scale,
        side_padding=side_padding,
        visible_playfield_width=visible_playfield_width,
        bottom_area_height=bottom_area_height,
        catcher_metrics=catcher_metrics,
        pixels_per_ms=pixels_per_ms,
    )


def _pixels_per_ms_for_ar(approach_rate: float, playfield_scale: float) -> float:
    time_range = _catch_time_range(approach_rate)
    # osu!catch 游玩内使用 stable 兼容的下落段：水果从 -100 落到判定线 340。
    # 预览图使用同一段距离和 AR TimeRange 定标，EZ/HR 后纵向密度才会跟游戏内一致。
    visible_fall_height = (STABLE_CATCHER_Y - STABLE_FRUIT_START_Y) * playfield_scale
    return visible_fall_height / time_range


def _catch_time_range(approach_rate: float) -> float:
    return _difficulty_range(approach_rate, 1800.0, 1200.0, 450.0)


def _difficulty_range(difficulty: float, minimum: float, middle: float, maximum: float) -> float:
    scaled = (difficulty - 5.0) / 5.0
    if difficulty > 5.0:
        return middle + (maximum - middle) * scaled
    if difficulty < 5.0:
        return middle + (middle - minimum) * scaled
    return middle


def _calculate_column_count(beatmap_duration: int, total_chart_height: int) -> int:
    if beatmap_duration >= 6 * 60 * 1000:
        return FIXED_COLUMN_COUNT_6_TO_10_MIN
    return max(1, math.ceil(total_chart_height / _resolve_max_area_height(beatmap_duration)))


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


def _build_catcher_metrics(circle_size: float, playfield_scale: float, skin: CatchSkin) -> CatcherMetrics:
    circle_scale = _circle_scale(circle_size)
    catcher_scale = circle_scale * 2
    logical_width = skin.catcher_idle.width / 2
    logical_height = skin.catcher_idle.height / 2
    target_width = max(1, round(logical_width * LEGACY_CATCHER_VISUAL_SCALE * catcher_scale * playfield_scale))
    target_height = max(1, round(logical_height * LEGACY_CATCHER_VISUAL_SCALE * catcher_scale * playfield_scale))
    origin_y = max(1, round(LEGACY_CATCHER_ORIGIN_Y * LEGACY_CATCHER_VISUAL_SCALE * catcher_scale * playfield_scale))
    return CatcherMetrics(
        width=target_width,
        height=target_height,
        origin_y=origin_y,
        bottom_extent=max(0, target_height - origin_y),
    )


def _draw_column_background(
    draw: ImageDraw.ImageDraw,
    layout: RenderLayout,
    column_index: int,
) -> None:
    column_left = _column_left(column_index, layout)
    chart_top = PAGE_MARGIN_Y
    chart_bottom = PAGE_MARGIN_Y + layout.total_column_height
    visible_left = column_left + LEFT_PANEL_WIDTH
    visible_right = visible_left + layout.visible_playfield_width

    draw.rectangle((column_left, chart_top, visible_left, chart_bottom), fill=LEFT_PANEL_BACKGROUND)
    draw.rectangle((visible_left, chart_top, visible_right, chart_bottom), fill=PLAYFIELD_BACKGROUND)
    draw.line((visible_left, chart_top, visible_left, chart_bottom), fill=PLAYFIELD_BORDER, width=1)
    draw.line((visible_right - 1, chart_top, visible_right - 1, chart_bottom), fill=PLAYFIELD_BORDER, width=1)


def _draw_timing_line(
    draw: ImageDraw.ImageDraw,
    timing_line: TimingLine,
    layout: RenderLayout,
    font: ImageFont.ImageFont,
) -> None:
    column_index = min(layout.column_count - 1, timing_line.time // layout.time_per_column)
    local_time = timing_line.time - column_index * layout.time_per_column
    column_left = _column_left(column_index, layout)
    playfield_left = _playfield_left(column_index, layout)
    y = _chart_bottom(layout, column_index) - round(local_time * layout.pixels_per_ms)

    draw.line(
        (playfield_left, y, playfield_left + COLUMN_WIDTH - 1, y),
        fill=timing_line.color,
        width=1,
    )
    label = f"{timing_line.time / 1000:.1f}s"
    label_box = draw.textbbox((0, 0), label, font=font)
    label_width = label_box[2] - label_box[0]
    label_height = label_box[3] - label_box[1]

    note_width = 0
    note_height = 0
    if timing_line.note is not None:
        note_box = draw.textbbox((0, 0), timing_line.note, font=font)
        note_width = note_box[2] - note_box[0]
        note_height = note_box[3] - note_box[1]

    block_width = max(label_width, note_width)
    block_height = label_height
    if timing_line.note is not None:
        block_height += TIME_LABEL_NOTE_GAP + note_height
    label_x = column_left + LEFT_PANEL_WIDTH + layout.visible_playfield_width + 4
    if column_index < layout.column_count - 1:
        next_column_left = _column_left(column_index + 1, layout)
        label_x = min(label_x, next_column_left - block_width - 4)
    else:
        label_x = min(label_x, layout.image_width - PAGE_MARGIN_X - block_width)

    block_top = y - block_height / 2
    draw.text((label_x, block_top), label, fill=timing_line.label_color, font=font)
    if timing_line.note is not None:
        note_y = block_top + label_height + TIME_LABEL_NOTE_GAP
        draw.text((label_x, note_y), timing_line.note, fill=timing_line.label_color, font=font)


def _draw_catch_object(
    image: Image.Image,
    skin: CatchSkin,
    catch_object: CatchRenderObject,
    layout: RenderLayout,
    cache: dict,
) -> None:
    column_index = min(layout.column_count - 1, catch_object.start_time // layout.time_per_column)
    local_time = catch_object.start_time - column_index * layout.time_per_column
    center_x = round(_playfield_left(column_index, layout) + catch_object.x * layout.playfield_scale)
    center_y = round(_chart_bottom(layout, column_index) - local_time * layout.pixels_per_ms)
    # catch 物件显示尺寸需要乘上 CS 缩放。
    diameter = max(
        1,
        round(OBJECT_RADIUS * 2 * layout.object_scale * layout.playfield_scale * catch_object.scale_factor),
    )

    if catch_object.sprite_name == "banana":
        base_sprite = skin.banana_base
        overlay_sprite = skin.banana_overlay
    elif catch_object.sprite_name == "drop":
        base_sprite = skin.droplet_base
        overlay_sprite = skin.droplet_overlay
    else:
        base_sprite = skin.fruit_bases[catch_object.sprite_name]
        overlay_sprite = skin.fruit_overlays[catch_object.sprite_name]

    if catch_object.hyper_dash:
        glow_size = max(1, round(diameter * FRUIT_HYPER_GLOW_SCALE))
        glow = _tint_sprite(base_sprite, skin.hyper_dash_fruit_color, glow_size, FRUIT_HYPER_GLOW_ALPHA)
        image.paste(glow, (round(center_x - glow.width / 2), round(center_y - glow.height / 2)), glow)

    base_key = (id(base_sprite), catch_object.color, diameter)
    base = cache.get(base_key)
    if base is None:
        base = _tint_sprite(base_sprite, catch_object.color, diameter, 1.0)
        cache[base_key] = base

    overlay_key = (id(overlay_sprite), diameter)
    overlay = cache.get(overlay_key)
    if overlay is None:
        overlay = _resize_sprite(overlay_sprite, (diameter, diameter))
        cache[overlay_key] = overlay

    if catch_object.rotation:
        base = base.rotate(catch_object.rotation, resample=Image.Resampling.BICUBIC, expand=True)
        overlay = overlay.rotate(catch_object.rotation, resample=Image.Resampling.BICUBIC, expand=True)

    image.paste(base, (round(center_x - base.width / 2), round(center_y - base.height / 2)), base)
    image.paste(overlay, (round(center_x - overlay.width / 2), round(center_y - overlay.height / 2)), overlay)


def _draw_catcher(
    image: Image.Image,
    skin: CatchSkin,
    layout: RenderLayout,
    column_index: int,
    cache: dict,
) -> None:
    catcher_key = (id(skin.catcher_idle), layout.catcher_metrics.width, layout.catcher_metrics.height)
    catcher = cache.get(catcher_key)
    if catcher is None:
        catcher = _resize_sprite(skin.catcher_idle, (layout.catcher_metrics.width, layout.catcher_metrics.height))
        cache[catcher_key] = catcher
    catch_x = round(_playfield_left(column_index, layout) + PLAYFIELD_WIDTH * layout.playfield_scale / 2 - catcher.width / 2)
    catch_y = _chart_bottom(layout, column_index) - layout.catcher_metrics.origin_y
    image.paste(catcher, (catch_x, catch_y), catcher)


def _build_timing_lines(timing_points: list[TimingPoint], chart_end_time: int) -> list[TimingLine]:
    base_points = sorted((point for point in timing_points if point.uninherited), key=lambda point: point.time)
    if not base_points:
        return []

    all_points = sorted(timing_points, key=lambda point: point.time)
    timing_lines: dict[int, TimingLine] = {}

    for index, point in enumerate(base_points):
        segment_end = chart_end_time
        if index + 1 < len(base_points):
            segment_end = int(base_points[index + 1].time)

        beat_index = 0
        current = point.time

        while current <= segment_end + 0.001:
            if current >= 0:
                time = int(round(current))
                is_bar = beat_index % point.meter == 0
                if is_bar:
                    timing_lines[time] = TimingLine(
                        time=time,
                        color=_timing_line_color(is_bar),
                        label_color=_time_label_color(time, all_points),
                    )
            beat_index += 1
            current = point.time + beat_index * point.beat_length

    for kiai_start_time in _collect_kiai_start_times(all_points, chart_end_time):
        line_color = BEAT_LINE
        if kiai_start_time in timing_lines:
            line_color = timing_lines[kiai_start_time].color
        timing_lines[kiai_start_time] = TimingLine(
            time=kiai_start_time,
            color=line_color,
            label_color=KIAI_TIME_LABEL_COLOR,
            note="Kiai",
        )

    return [timing_lines[time] for time in sorted(timing_lines)]


def _timing_line_color(is_bar: bool) -> tuple[int, int, int, int]:
    if is_bar:
        return MEASURE_LINE
    return BEAT_LINE


def _time_label_color(time: int, timing_points: list[TimingPoint]) -> tuple[int, int, int, int]:
    if _kiai_mode_at(time, timing_points):
        return KIAI_TIME_LABEL_COLOR
    return RULER_TEXT


def _collect_kiai_start_times(timing_points: list[TimingPoint], chart_end_time: int) -> list[int]:
    kiai_start_times: list[int] = []
    previous_kiai_mode = False

    for point in timing_points:
        point_time = int(round(point.time))
        if point_time > chart_end_time:
            break
        if point_time >= 0 and point.kiai_mode and not previous_kiai_mode:
            kiai_start_times.append(point_time)
        previous_kiai_mode = point.kiai_mode

    return kiai_start_times


def _kiai_mode_at(time: int, timing_points: list[TimingPoint]) -> bool:
    current_kiai_mode = False
    for point in timing_points:
        if point.time > time:
            break
        current_kiai_mode = point.kiai_mode
    return current_kiai_mode


def _column_left(column_index: int, layout: RenderLayout) -> int:
    return PAGE_MARGIN_X + column_index * (LEFT_PANEL_WIDTH + layout.visible_playfield_width + COLUMN_GAP)


def _playfield_left(column_index: int, layout: RenderLayout) -> int:
    return _column_left(column_index, layout) + LEFT_PANEL_WIDTH + layout.side_padding


def _chart_bottom(layout: RenderLayout, column_index: int) -> int:
    if DRAW_CATCHER_EACH_COLUMN or column_index == 0:
        return PAGE_MARGIN_Y + TOP_BUFFER + layout.column_height
    return PAGE_MARGIN_Y + layout.total_column_height - OBJECT_BOTTOM_PADDING


def _resize_sprite(sprite: Image.Image, size: tuple[int, int]) -> Image.Image:
    return sprite.resize(size, Image.Resampling.LANCZOS)


def _tint_sprite(
    sprite: Image.Image,
    color: tuple[int, int, int],
    size: int,
    alpha: float,
) -> Image.Image:
    resized = _resize_sprite(sprite, (size, size))
    if alpha < 1:
        alpha_channel = resized.getchannel("A").point(lambda value: round(value * alpha))
        resized.putalpha(alpha_channel)
    mask = resized.getchannel("A")
    tinted = Image.new("RGBA", resized.size, (*color, 0))
    tinted.putalpha(mask)
    return tinted


def _circle_scale(circle_size: float) -> float:
    return (1.0 - 0.7 * ((circle_size - 5.0) / 5.0)) / 2.0


def _object_order(object_type: str) -> int:
    order = {
        "tiny_droplet": 0,
        "droplet": 1,
        "fruit": 2,
        "banana": 3,
    }
    return order[object_type]


def _effective_difficulty(beatmap: Beatmap, mods: ModSettings | None) -> dict[str, float]:
    difficulty = {
        "CircleSize": float(beatmap.difficulty.get("CircleSize", "5")),
        "OverallDifficulty": float(beatmap.difficulty.get("OverallDifficulty", "5")),
        "ApproachRate": float(beatmap.difficulty.get("ApproachRate", beatmap.difficulty.get("OverallDifficulty", "5"))),
        "HPDrainRate": float(beatmap.difficulty.get("HPDrainRate", "5")),
        "SliderMultiplier": float(beatmap.difficulty.get("SliderMultiplier", "1.4")),
        "SliderTickRate": float(beatmap.difficulty.get("SliderTickRate", "1")),
    }
    if mods is None:
        return difficulty

    if mods.easy:
        difficulty["CircleSize"] *= 0.5
        difficulty["ApproachRate"] *= 0.5
        difficulty["HPDrainRate"] *= 0.5
        difficulty["OverallDifficulty"] *= 0.5

    if mods.hard_rock:
        difficulty["HPDrainRate"] = min(difficulty["HPDrainRate"] * 1.4, 10.0)
        difficulty["OverallDifficulty"] = min(difficulty["OverallDifficulty"] * 1.4, 10.0)
        difficulty["CircleSize"] = min(difficulty["CircleSize"] * 1.3, 10.0)
        difficulty["ApproachRate"] = min(difficulty["ApproachRate"] * 1.4, 10.0)

    return difficulty
