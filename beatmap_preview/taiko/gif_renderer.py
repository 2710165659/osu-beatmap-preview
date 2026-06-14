from __future__ import annotations

import math
from bisect import bisect_right
from dataclasses import dataclass

from PIL import Image, ImageDraw, ImageFont

from ..errors import PreviewError
from ..models import Beatmap, TaikoHitObject, TimingPoint
from ..mods import ModSettings
from ..time_selection import PreviewTimeSelector, times_to_milliseconds
from .config import (
    BIG_NOTE_SCALE,
    CENTRE_NOTE_COLOR,
    DEFAULT_BEAT_LENGTH,
    GIF_ASPECT,
    GIF_DURATION_MS,
    GIF_FPS,
    GIF_JUDGEMENT_LINE_COLOR,
    GIF_JUDGEMENT_LINE_OFFSET,
    GIF_LOOP,
    GIF_PREVIEW_TIME_LABEL_COLOR,
    GIF_ROW_GAP,
    GIF_ROW_HEIGHT,
    GIF_SCROLL_LENGTH_PX,
    GIF_SEGMENT_COUNT,
    GIF_STABLE_GAMEFIELD_HEIGHT,
    GIF_STABLE_HIT_LOCATION,
    GIF_TIME_LABEL_COLOR,
    GIF_TIME_LABEL_FONT_SIZE,
    GIF_TIME_LABEL_NOTE_COLOR,
    GIF_TIME_LABEL_NOTE_FONT_SIZE,
    GIF_VELOCITY_MULTIPLIER,
    IMAGE_BACKGROUND,
    NORMAL_NOTE_SIZE_RATIO,
    PAGE_MARGIN_X,
    PAGE_MARGIN_Y,
    RIM_NOTE_COLOR,
    ROLL_COLOR,
    ROW_INNER_PADDING_X,
    SPAN_BODY_HEIGHT_RATIO,
    SWELL_BODY_HEIGHT_RATIO,
    SWELL_COLOR,
)
from .renderer import (
    HIT_SOUNDS_RIM,
    HIT_SOUNDS_STRONG,
    DRUMROLL_FLAG,
    SWELL_FLAG,
    _apply_taiko_object_mods,
    _effective_slider_multiplier,
    _effective_timing_points,
)
from .skin import TaikoSkin, load_taiko_skin

MULTIPLIER_BASE_BEAT_LENGTH = 1000.0


@dataclass(frozen=True)
class MultiplierPoint:
    time: float
    multiplier: float


@dataclass(frozen=True)
class PreparedTaikoHitObject:
    hit_object: TaikoHitObject
    start_multiplier: float
    end_multiplier: float
    min_multiplier: float
    max_multiplier: float


class MultiplierLookup:
    def __init__(self, points: list[MultiplierPoint]):
        self._points = points
        self._times = [point.time for point in points]

    def at(self, time: float) -> float:
        idx = bisect_right(self._times, time) - 1
        return self._points[max(0, idx)].multiplier


@dataclass(frozen=True)
class GifLayout:
    segment_count: int
    segment_width: int
    row_height: int
    left_panel_width: int
    right_panel_width: int
    image_width: int
    image_height: int
    normal_note_diameter: int
    big_note_diameter: int
    time_range: float  # osu! taiko 一屏可见时间窗口(ms)


def render_taiko_gif(
    beatmap: Beatmap,
    mods: ModSettings | None = None,
    times: list[float] | None = None,
):
    hit_objects = _apply_taiko_object_mods(
        [ho for ho in beatmap.hit_objects if isinstance(ho, TaikoHitObject)],
        mods,
    )
    if not hit_objects:
        raise PreviewError("taiko beatmap has no hit objects")

    skin = load_taiko_skin()
    speed_multiplier = mods.speed_multiplier if mods is not None else 1.0
    gameplay_segment_duration = round(GIF_DURATION_MS * speed_multiplier)

    segment_timings = PreviewTimeSelector(
        beatmap=beatmap,
        hit_objects=hit_objects,
        segment_count=GIF_SEGMENT_COUNT,
        segment_duration=gameplay_segment_duration,
        requested_start_times=times_to_milliseconds(times),
    ).choose()

    slider_multiplier = _effective_slider_multiplier(beatmap, mods)
    timing_points = _effective_timing_points(beatmap, mods)

    # 构建 Overlapping 算法所需的 multiplier 控制点列表。
    multiplier_lookup = MultiplierLookup(_build_multiplier_points(timing_points, slider_multiplier))
    prepared_hit_objects = _prepare_hit_objects(hit_objects, multiplier_lookup)
    time_range = _compute_time_range() / speed_multiplier

    layout = _build_layout(skin, time_range)
    font_regular = ImageFont.load_default(size=GIF_TIME_LABEL_FONT_SIZE)
    font_note = ImageFont.load_default(size=GIF_TIME_LABEL_NOTE_FONT_SIZE)
    frame_count = max(1, round(GIF_DURATION_MS * GIF_FPS / 1000))
    frame_duration_ms = max(1, round(1000 / GIF_FPS))

    segment_snapshot_times = [
        tuple(
            timing.start_time + round(frame_index * 1000 * speed_multiplier / GIF_FPS)
            for frame_index in range(frame_count)
        )
        for timing in segment_timings
    ]

    render_cache: dict = {}

    def frame_generator():
        for frame_index in range(frame_count):
            canvas = Image.new("RGB", (layout.image_width, layout.image_height), IMAGE_BACKGROUND[:3])

            for segment_index, segment_timing in enumerate(segment_timings):
                snapshot_time = segment_snapshot_times[segment_index][frame_index]
                _draw_row_background(canvas, skin, layout, segment_index, render_cache)
                _draw_hit_objects(
                    canvas, prepared_hit_objects, skin, layout,
                    segment_index, snapshot_time, render_cache,
                )

            draw = ImageDraw.Draw(canvas)
            for segment_index, segment_timing in enumerate(segment_timings):
                _draw_time_label(
                    draw, segment_timing.start_time, gameplay_segment_duration,
                    segment_index, layout, font_regular, font_note, segment_timing.is_preview,
                )

            yield canvas

    return frame_generator(), frame_duration_ms, GIF_LOOP


def _compute_time_range() -> float:
    """TaikoPlayfieldAdjustmentContainer.ComputeTimeRange()，16:9 1080p 参数。"""
    in_length = GIF_ASPECT * GIF_STABLE_GAMEFIELD_HEIGHT - GIF_STABLE_HIT_LOCATION
    return in_length / 100 * 1000 / GIF_VELOCITY_MULTIPLIER


def _build_multiplier_points(timing_points: list[TimingPoint], slider_multiplier: float) -> list[MultiplierPoint]:
    """构建 Overlapping 算法的 multiplier 控制点。

    MultiplierControlPoint.Multiplier = Velocity * EffectPoint.ScrollSpeed * BaseBeatLength / TimingPoint.BeatLength
    taiko 的 DrawableRuleset 没开 RelativeScaleBeatLengths，因此 BaseBeatLength 使用 1000。
    """
    base_beat_length = MULTIPLIER_BASE_BEAT_LENGTH
    points: list[MultiplierPoint] = []
    current_beat_length = base_beat_length
    current_scroll_speed = 1.0

    for tp in timing_points:
        if tp.uninherited:
            if math.isfinite(tp.beat_length) and abs(tp.beat_length) > 1e-9:
                current_beat_length = tp.beat_length
            current_scroll_speed = 1.0
        elif tp.beat_length < -0.001:
            current_scroll_speed = -100.0 / tp.beat_length
        elif not math.isnan(tp.beat_length):
            current_scroll_speed = 1.0

        multiplier = slider_multiplier * current_scroll_speed * base_beat_length / current_beat_length
        points.append(MultiplierPoint(time=float(tp.time), multiplier=multiplier))

    if not points:
        points = [MultiplierPoint(time=0.0, multiplier=slider_multiplier)]
    elif points[0].time > 0:
        points.insert(0, MultiplierPoint(time=0.0, multiplier=points[0].multiplier))

    return points


def _prepare_hit_objects(
    hit_objects: list[TaikoHitObject],
    multiplier_lookup: MultiplierLookup,
) -> list[PreparedTaikoHitObject]:
    prepared: list[PreparedTaikoHitObject] = []
    for hit_object in hit_objects:
        prepared.append(
            PreparedTaikoHitObject(
                hit_object=hit_object,
                start_multiplier=multiplier_lookup.at(hit_object.start_time),
                end_multiplier=multiplier_lookup.at(hit_object.end_time),
                min_multiplier=min(
                    multiplier_lookup.at(hit_object.start_time),
                    multiplier_lookup.at(hit_object.end_time),
                ),
                max_multiplier=max(
                    multiplier_lookup.at(hit_object.start_time),
                    multiplier_lookup.at(hit_object.end_time),
                ),
            )
        )
    return prepared


def _object_x(note_time: float, snapshot_time: float, multiplier: float, layout: GifLayout) -> int:
    """Overlapping PositionAt: X = judgement_x + (note_time - snapshot_time) / timeRange * multiplier * scrollLength"""
    judgement_x = _judgement_line_x(layout)
    offset = (note_time - snapshot_time) / layout.time_range * multiplier * layout.segment_width
    return round(judgement_x + offset)


def _build_layout(skin: TaikoSkin, time_range: float) -> GifLayout:
    segment_width = round(GIF_SCROLL_LENGTH_PX)
    left_panel_width = round(skin.bar_left.width * GIF_ROW_HEIGHT / skin.bar_left.height)
    right_panel_width = ROW_INNER_PADDING_X * 2 + segment_width

    image_width = PAGE_MARGIN_X * 2 + left_panel_width + right_panel_width
    image_height = (
        PAGE_MARGIN_Y * 2
        + GIF_SEGMENT_COUNT * GIF_ROW_HEIGHT
        + (GIF_SEGMENT_COUNT - 1) * GIF_ROW_GAP
        + 50
    )

    normal_note_diameter = round(GIF_ROW_HEIGHT * NORMAL_NOTE_SIZE_RATIO)
    big_note_diameter = round(normal_note_diameter * BIG_NOTE_SCALE)

    return GifLayout(
        segment_count=GIF_SEGMENT_COUNT,
        segment_width=segment_width,
        row_height=GIF_ROW_HEIGHT,
        left_panel_width=left_panel_width,
        right_panel_width=right_panel_width,
        image_width=image_width,
        image_height=image_height,
        normal_note_diameter=normal_note_diameter,
        big_note_diameter=big_note_diameter,
        time_range=time_range,
    )


def _row_top(row_index: int, layout: GifLayout) -> int:
    return PAGE_MARGIN_Y + row_index * (layout.row_height + GIF_ROW_GAP)


def _row_center_y(row_index: int, layout: GifLayout) -> int:
    return _row_top(row_index, layout) + layout.row_height // 2


def _judgement_line_x(layout: GifLayout) -> int:
    return PAGE_MARGIN_X + layout.left_panel_width + GIF_JUDGEMENT_LINE_OFFSET


def _draw_judgement_line(image: Image.Image, layout: GifLayout, row_index: int) -> None:
    draw = ImageDraw.Draw(image)
    line_x = _judgement_line_x(layout)
    row_top = _row_top(row_index, layout)
    draw.line((line_x, row_top, line_x, row_top + layout.row_height), fill=GIF_JUDGEMENT_LINE_COLOR, width=3)


def _draw_row_background(image: Image.Image, skin: TaikoSkin, layout: GifLayout, row_index: int, cache: dict) -> None:
    row_top = _row_top(row_index, layout)

    left_key = (id(skin.bar_left), layout.left_panel_width, layout.row_height)
    left = cache.get(left_key)
    if left is None:
        left = _resize_sprite(skin.bar_left, (layout.left_panel_width, layout.row_height))
        cache[left_key] = left

    right_key = (id(skin.bar_right), layout.right_panel_width, layout.row_height)
    right = cache.get(right_key)
    if right is None:
        right = _resize_sprite(skin.bar_right, (layout.right_panel_width, layout.row_height))
        cache[right_key] = right

    image.paste(left, (PAGE_MARGIN_X, row_top), left)
    image.paste(right, (PAGE_MARGIN_X + layout.left_panel_width, row_top), right)
    _draw_judgement_line(image, layout, row_index)


def _draw_hit_objects(
    image: Image.Image,
    hit_objects: list[PreparedTaikoHitObject],
    skin: TaikoSkin,
    layout: GifLayout,
    row_index: int,
    snapshot_time: int,
    cache: dict,
) -> None:
    # Overlapping 可视范围：
    # x = judgement + (time - snapshot) / time_range * multiplier * scroll_length
    # 对于 multiplier>0，若 start/end 都完全落在判定线左侧或右边界右侧，就可以整物件跳过。
    left_bound = _judgement_line_x(layout)
    right_bound = PAGE_MARGIN_X + layout.left_panel_width + layout.right_panel_width

    for hit_object in reversed(hit_objects):
        if _can_skip_hit_object(hit_object, snapshot_time, layout, left_bound, right_bound):
            continue
        _draw_hit_object(image, hit_object, skin, layout, row_index, snapshot_time, cache)


def _can_skip_hit_object(
    hit_object: PreparedTaikoHitObject,
    snapshot_time: int,
    layout: GifLayout,
    left_bound: int,
    right_bound: int,
) -> bool:
    base_hit_object = hit_object.hit_object
    earliest_x = _object_x(base_hit_object.start_time, snapshot_time, hit_object.min_multiplier, layout)
    latest_x = _object_x(base_hit_object.end_time, snapshot_time, hit_object.max_multiplier, layout)
    if earliest_x > latest_x:
        earliest_x, latest_x = latest_x, earliest_x
    return latest_x < left_bound or earliest_x > right_bound


def _draw_hit_object(
    image: Image.Image,
    hit_object: PreparedTaikoHitObject,
    skin: TaikoSkin,
    layout: GifLayout,
    row_index: int,
    snapshot_time: int,
    cache: dict,
) -> None:
    base_hit_object = hit_object.hit_object
    if base_hit_object.hit_type & SWELL_FLAG:
        _draw_span_object(image, hit_object, skin, layout, row_index, snapshot_time, cache,
                          is_swell=True, span_color=SWELL_COLOR, draw_spinner_warning=True)
        return
    if base_hit_object.hit_type & DRUMROLL_FLAG:
        is_big_roll = bool(base_hit_object.hitsound & HIT_SOUNDS_STRONG)
        _draw_span_object(image, hit_object, skin, layout, row_index, snapshot_time, cache,
                          is_swell=is_big_roll, span_color=ROLL_COLOR, draw_spinner_warning=False)
        return
    _draw_circle_object(image, hit_object, skin, layout, row_index, snapshot_time, cache)


def _draw_circle_object(
    image: Image.Image,
    hit_object: PreparedTaikoHitObject,
    skin: TaikoSkin,
    layout: GifLayout,
    row_index: int,
    snapshot_time: int,
    cache: dict,
) -> None:
    base_hit_object = hit_object.hit_object
    center_x = _object_x(base_hit_object.start_time, snapshot_time, hit_object.start_multiplier, layout)
    center_y = _row_center_y(row_index, layout)

    judgement_x = _judgement_line_x(layout)
    right_bound = PAGE_MARGIN_X + layout.left_panel_width + layout.right_panel_width
    if center_x < judgement_x or center_x > right_bound:
        return

    is_strong = bool(base_hit_object.hitsound & HIT_SOUNDS_STRONG)
    is_rim = bool(base_hit_object.hitsound & HIT_SOUNDS_RIM)
    diameter = layout.big_note_diameter if is_strong else layout.normal_note_diameter
    base_sprite = skin.big_hit_circle if is_strong else skin.hit_circle
    overlay_sprite = skin.big_hit_circle_overlay if is_strong else skin.hit_circle_overlay
    color = RIM_NOTE_COLOR if is_rim else CENTRE_NOTE_COLOR

    _draw_note_sprite(image, base_sprite, overlay_sprite, color, diameter, center_x, center_y, cache)


def _draw_span_object(
    image: Image.Image,
    hit_object: PreparedTaikoHitObject,
    skin: TaikoSkin,
    layout: GifLayout,
    row_index: int,
    snapshot_time: int,
    cache: dict,
    is_swell: bool,
    span_color: tuple[int, int, int],
    draw_spinner_warning: bool,
) -> None:
    base_hit_object = hit_object.hit_object
    # Overlapping: 头尾各用自己时刻的 multiplier。
    start_x = _object_x(base_hit_object.start_time, snapshot_time, hit_object.start_multiplier, layout)
    end_x = _object_x(base_hit_object.end_time, snapshot_time, hit_object.end_multiplier, layout)
    center_y = _row_center_y(row_index, layout)
    clip_left = _judgement_line_x(layout)
    clip_right = PAGE_MARGIN_X + layout.left_panel_width + layout.right_panel_width

    head_diameter = layout.big_note_diameter if is_swell else layout.normal_note_diameter
    body_height = round(head_diameter * (SWELL_BODY_HEIGHT_RATIO if is_swell else SPAN_BODY_HEIGHT_RATIO))

    _draw_roll_body(image, skin, span_color, start_x, end_x, center_y, body_height, clip_left, clip_right)
    _draw_span_head(
        image, skin, span_color, start_x, center_y, head_diameter, cache,
        is_swell, draw_spinner_warning, clip_left, clip_right,
    )
    _draw_span_tail(image, span_color, end_x, center_y, body_height, cache, clip_left, clip_right)


def _draw_roll_body(image: Image.Image, skin: TaikoSkin, color: tuple[int, int, int],
                    start_x: int, end_x: int, center_y: int, height: int,
                    clip_left: int, clip_right: int) -> None:
    if end_x <= start_x:
        return
    visible_left = max(start_x, clip_left)
    visible_right = min(end_x, clip_right)
    if visible_right <= visible_left:
        return
    body = _tint_sprite(skin.roll_middle, color, (max(1, end_x - start_x), height))
    body_left_crop = max(0, visible_left - start_x)
    body_right_crop = min(body.width, body_left_crop + (visible_right - visible_left))
    if body_right_crop <= body_left_crop:
        return
    cropped = body.crop((body_left_crop, 0, body_right_crop, body.height))
    image.paste(cropped, (visible_left, round(center_y - height / 2)), cropped)


def _draw_span_head(image: Image.Image, skin: TaikoSkin, color: tuple[int, int, int],
                    center_x: int, center_y: int, diameter: int, cache: dict,
                    is_swell: bool, draw_spinner_warning: bool,
                    clip_left: int, clip_right: int) -> None:
    base_sprite = skin.big_hit_circle if is_swell else skin.hit_circle
    overlay_sprite = skin.big_hit_circle_overlay if is_swell else skin.hit_circle_overlay
    _draw_clipped_note_sprite(
        image, base_sprite, overlay_sprite, color, diameter, center_x, center_y, cache, clip_left, clip_right,
    )
    if draw_spinner_warning:
        warning_key = (id(skin.spinner_warning), diameter)
        warning = cache.get(warning_key)
        if warning is None:
            warning = _resize_sprite(skin.spinner_warning, (diameter, diameter))
            cache[warning_key] = warning
        _paste_clipped_rgba(
            image,
            warning,
            round(center_x - diameter / 2),
            round(center_y - diameter / 2),
            clip_left,
            clip_right,
        )


def _draw_span_tail(image: Image.Image, color: tuple[int, int, int],
                    join_x: int, center_y: int, height: int, cache: dict,
                    clip_left: int, clip_right: int) -> None:
    key = (color, height)
    tail = cache.get(key)
    if tail is None:
        scale = 4
        sh = height * scale
        sw = max(1, math.ceil(height / 2) * scale)
        r = sh / 2
        bw = max(1, round(height * 0.05)) * scale
        tail = Image.new("RGBA", (sw, sh), (0, 0, 0, 0))
        d = ImageDraw.Draw(tail)
        d.ellipse((-r, 0, r, sh), fill=(0, 0, 0, 255))
        d.ellipse((-r + bw, bw, r - bw, sh - bw), fill=(*color, 255))
        tail = tail.resize((sw // scale, height), Image.Resampling.LANCZOS)
        cache[key] = tail
    _paste_clipped_rgba(image, tail, join_x, round(center_y - height / 2), clip_left, clip_right)


def _draw_note_sprite(image: Image.Image, base_sprite: Image.Image, overlay_sprite: Image.Image,
                      color: tuple[int, int, int], diameter: int, center_x: int, center_y: int, cache: dict) -> None:
    base_key = (id(base_sprite), color, diameter)
    tinted_base = cache.get(base_key)
    if tinted_base is None:
        tinted_base = _tint_sprite(base_sprite, color, diameter)
        cache[base_key] = tinted_base

    overlay_key = (id(overlay_sprite), diameter)
    overlay = cache.get(overlay_key)
    if overlay is None:
        overlay = _resize_sprite(overlay_sprite, (diameter, diameter))
        cache[overlay_key] = overlay

    position = (round(center_x - diameter / 2), round(center_y - diameter / 2))
    image.paste(tinted_base, position, tinted_base)
    image.paste(overlay, position, overlay)


def _draw_clipped_note_sprite(
    image: Image.Image,
    base_sprite: Image.Image,
    overlay_sprite: Image.Image,
    color: tuple[int, int, int],
    diameter: int,
    center_x: int,
    center_y: int,
    cache: dict,
    clip_left: int,
    clip_right: int,
) -> None:
    base_key = (id(base_sprite), color, diameter)
    tinted_base = cache.get(base_key)
    if tinted_base is None:
        tinted_base = _tint_sprite(base_sprite, color, diameter)
        cache[base_key] = tinted_base

    overlay_key = (id(overlay_sprite), diameter)
    overlay = cache.get(overlay_key)
    if overlay is None:
        overlay = _resize_sprite(overlay_sprite, (diameter, diameter))
        cache[overlay_key] = overlay

    sprite_x = round(center_x - diameter / 2)
    sprite_y = round(center_y - diameter / 2)
    _paste_clipped_rgba(image, tinted_base, sprite_x, sprite_y, clip_left, clip_right)
    _paste_clipped_rgba(image, overlay, sprite_x, sprite_y, clip_left, clip_right)


def _paste_clipped_rgba(
    image: Image.Image,
    sprite: Image.Image,
    x: int,
    y: int,
    clip_left: int,
    clip_right: int,
) -> None:
    sprite_left = x
    sprite_right = x + sprite.width
    visible_left = max(sprite_left, clip_left)
    visible_right = min(sprite_right, clip_right)
    if visible_right <= visible_left:
        return

    crop_left = visible_left - sprite_left
    crop_right = crop_left + (visible_right - visible_left)
    cropped = sprite.crop((crop_left, 0, crop_right, sprite.height))
    image.paste(cropped, (visible_left, y), cropped)


def _draw_time_label(draw: ImageDraw.ImageDraw, start_time: int, duration_ms: int, row_index: int,
                     layout: GifLayout, font_regular: ImageFont.ImageFont, font_note: ImageFont.ImageFont,
                     is_preview: bool) -> None:
    y = _row_top(row_index, layout) + layout.row_height + 5
    label = f"{_format_time(start_time)} - {_format_time(start_time + duration_ms)}"
    color = GIF_PREVIEW_TIME_LABEL_COLOR if is_preview else GIF_TIME_LABEL_COLOR
    note_color = GIF_PREVIEW_TIME_LABEL_COLOR if is_preview else GIF_TIME_LABEL_NOTE_COLOR
    box = draw.textbbox((0, 0), label, font=font_regular)
    x = PAGE_MARGIN_X + (layout.image_width - PAGE_MARGIN_X * 2 - (box[2] - box[0])) / 2
    draw.text((x, y), label, fill=color, font=font_regular)

    if is_preview:
        note = "Preview Time"
        note_box = draw.textbbox((0, 0), note, font=font_note)
        note_x = PAGE_MARGIN_X + (layout.image_width - PAGE_MARGIN_X * 2 - (note_box[2] - note_box[0])) / 2
        draw.text((note_x, y + (box[3] - box[1]) + 4), note, fill=note_color, font=font_note)


def _format_time(ms: int) -> str:
    total_seconds = max(0, ms) // 1000
    return f"{total_seconds // 60}:{total_seconds % 60:02d}"


def _resize_sprite(sprite: Image.Image, size: tuple[int, int]) -> Image.Image:
    return sprite.resize(size, Image.Resampling.LANCZOS)


def _tint_sprite(sprite: Image.Image, color: tuple[int, int, int], size: int | tuple[int, int]) -> Image.Image:
    target_size = (size, size) if isinstance(size, int) else size
    resized = _resize_sprite(sprite, target_size)
    red, green, blue, alpha = resized.split()
    return Image.merge("RGBA", (
        red.point(lambda v: round(v * color[0] / 255)),
        green.point(lambda v: round(v * color[1] / 255)),
        blue.point(lambda v: round(v * color[2] / 255)),
        alpha,
    ))
