from __future__ import annotations

import math
from dataclasses import dataclass

from PIL import Image, ImageDraw, ImageFont

from ..errors import PreviewError
from ..models import Beatmap, CatchHitObject
from ..mods import ModSettings
from ..time_selection import PreviewTimeSelector, times_to_milliseconds
from .config import (
    FRUIT_HYPER_GLOW_ALPHA,
    FRUIT_HYPER_GLOW_SCALE,
    GIF_DURATION_MS,
    GIF_FPS,
    GIF_GRID_GAP,
    GIF_IMAGE_HEIGHT,
    GIF_IMAGE_WIDTH,
    GIF_IMAGES_PER_ROW,
    GIF_LEFT_PANEL_BACKGROUND,
    GIF_LEFT_PANEL_WIDTH,
    GIF_LOOP,
    GIF_PREVIEW_TIME_LABEL_COLOR,
    GIF_ROW_COUNT,
    GIF_SEGMENT_COUNT,
    GIF_TIME_LABEL_COLOR,
    GIF_TIME_LABEL_FONT_SIZE,
    GIF_TIME_LABEL_HEIGHT,
    GIF_TIME_LABEL_NOTE_COLOR,
    GIF_TIME_LABEL_NOTE_FONT_SIZE,
    GIF_TIME_LABEL_NOTE_TOP_GAP,
    GIF_TIME_LABEL_TOP_GAP,
    IMAGE_BACKGROUND,
    OBJECT_RADIUS,
    PAGE_MARGIN_X,
    PAGE_MARGIN_Y,
    PLAYFIELD_BACKGROUND,
    PLAYFIELD_BORDER,
    PLAYFIELD_SIDE_PADDING,
    PLAYFIELD_WIDTH,
    STABLE_CATCHER_Y,
    STABLE_FRUIT_START_Y,
    TOP_BUFFER,
)
from .objects import CatchRenderObject, build_catch_render_objects
from .skin import CatchSkin, load_catch_skin
from .slider_path import _build_slider_path_cached


@dataclass(frozen=True)
class GifLayout:
    image_width: int
    image_height: int
    canvas_width: int
    canvas_height: int
    playfield_scale: float
    object_scale: float
    side_padding: int
    visible_playfield_width: int
    pixels_per_ms: float
    time_range: float


def render_catch_gif(
    beatmap: Beatmap,
    mods: ModSettings | None = None,
    times: list[float] | None = None,
):
    _build_slider_path_cached.cache_clear()
    try:
        hit_objects = [ho for ho in beatmap.hit_objects if isinstance(ho, CatchHitObject)]
        if not hit_objects:
            raise PreviewError("catch beatmap has no hit objects")

        skin = load_catch_skin()
        effective_difficulty = _effective_difficulty(beatmap, mods)
        render_objects = build_catch_render_objects(
            beatmap, hit_objects, skin.combo_colors, mods=mods, difficulty=effective_difficulty
        )

        speed_multiplier = mods.speed_multiplier if mods is not None else 1.0
        gameplay_segment_duration = round(GIF_DURATION_MS * speed_multiplier)
        segment_timings = PreviewTimeSelector(
            beatmap=beatmap,
            hit_objects=hit_objects,
            segment_count=GIF_SEGMENT_COUNT,
            segment_duration=gameplay_segment_duration,
            requested_start_times=times_to_milliseconds(times),
        ).choose()

        layout = _build_layout(effective_difficulty["CircleSize"], effective_difficulty["ApproachRate"])

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
            try:
                for frame_index in range(frame_count):
                    canvas = Image.new("RGBA", (layout.canvas_width, layout.canvas_height), IMAGE_BACKGROUND)
                    draw = ImageDraw.Draw(canvas)

                    for segment_index, segment_timing in enumerate(segment_timings):
                        snapshot_time = segment_snapshot_times[segment_index][frame_index]
                        frame_x, frame_y = _frame_origin(segment_index)
                        frame = _render_frame(
                            skin=skin,
                            render_objects=render_objects,
                            snapshot_time=snapshot_time,
                            layout=layout,
                            cache=render_cache,
                        )
                        canvas.alpha_composite(frame, (frame_x, frame_y))
                        _draw_time_label(
                            draw=draw,
                            start_time=segment_timing.start_time,
                            duration_ms=gameplay_segment_duration,
                            frame_x=frame_x,
                            frame_y=frame_y,
                            layout=layout,
                            font_regular=font_regular,
                            font_note=font_note,
                            is_preview=segment_timing.is_preview,
                        )

                    yield canvas
            finally:
                _build_slider_path_cached.cache_clear()

        return frame_generator(), frame_duration_ms, GIF_LOOP
    except Exception:
        _build_slider_path_cached.cache_clear()
        raise


def _build_layout(circle_size: float, approach_rate: float) -> GifLayout:
    # playfield 缩放：GIF_IMAGE_WIDTH 对应游戏内 PLAYFIELD_WIDTH
    playfield_scale = (GIF_IMAGE_WIDTH - GIF_LEFT_PANEL_WIDTH) / PLAYFIELD_WIDTH
    object_scale = _circle_scale(circle_size)
    side_padding = round(PLAYFIELD_SIDE_PADDING * playfield_scale)
    visible_playfield_width = round(PLAYFIELD_WIDTH * playfield_scale) + side_padding * 2

    time_range = _catch_time_range(approach_rate)
    # pixels_per_ms：水果从出现到判定线的逻辑像素距离 / 时间窗
    visible_fall_height = (STABLE_CATCHER_Y - STABLE_FRUIT_START_Y) * playfield_scale
    pixels_per_ms = visible_fall_height / time_range

    row_height = GIF_IMAGE_HEIGHT + GIF_TIME_LABEL_TOP_GAP + GIF_TIME_LABEL_HEIGHT
    canvas_width = PAGE_MARGIN_X * 2 + GIF_IMAGES_PER_ROW * GIF_IMAGE_WIDTH + (GIF_IMAGES_PER_ROW - 1) * GIF_GRID_GAP
    canvas_height = PAGE_MARGIN_Y * 2 + GIF_ROW_COUNT * row_height + (GIF_ROW_COUNT - 1) * GIF_GRID_GAP

    return GifLayout(
        image_width=GIF_IMAGE_WIDTH,
        image_height=GIF_IMAGE_HEIGHT,
        canvas_width=canvas_width,
        canvas_height=canvas_height,
        playfield_scale=playfield_scale,
        object_scale=object_scale,
        side_padding=side_padding,
        visible_playfield_width=visible_playfield_width,
        pixels_per_ms=pixels_per_ms,
        time_range=time_range,
    )


def _frame_origin(segment_index: int) -> tuple[int, int]:
    row_index = segment_index // GIF_IMAGES_PER_ROW
    col_index = segment_index % GIF_IMAGES_PER_ROW
    row_height = GIF_IMAGE_HEIGHT + GIF_TIME_LABEL_TOP_GAP + GIF_TIME_LABEL_HEIGHT
    x = PAGE_MARGIN_X + col_index * (GIF_IMAGE_WIDTH + GIF_GRID_GAP)
    y = PAGE_MARGIN_Y + row_index * (row_height + GIF_GRID_GAP)
    return x, y


def _render_frame(
    skin: CatchSkin,
    render_objects: list[CatchRenderObject],
    snapshot_time: int,
    layout: GifLayout,
    cache: dict,
) -> Image.Image:
    frame = Image.new("RGBA", (GIF_IMAGE_WIDTH, GIF_IMAGE_HEIGHT), IMAGE_BACKGROUND)
    draw = ImageDraw.Draw(frame)

    # 左侧灰色区域
    draw.rectangle((0, 0, GIF_LEFT_PANEL_WIDTH, GIF_IMAGE_HEIGHT), fill=GIF_LEFT_PANEL_BACKGROUND)

    # playfield 背景
    playfield_left = GIF_LEFT_PANEL_WIDTH
    playfield_right = GIF_IMAGE_WIDTH
    draw.rectangle((playfield_left, 0, playfield_right, GIF_IMAGE_HEIGHT), fill=PLAYFIELD_BACKGROUND)
    draw.line((playfield_left, 0, playfield_left, GIF_IMAGE_HEIGHT), fill=PLAYFIELD_BORDER, width=1)

    # 判定线（水果落到底部的位置）
    judgement_y = round(STABLE_CATCHER_Y * layout.playfield_scale)
    draw.line((playfield_left, judgement_y, GIF_IMAGE_WIDTH - 1, judgement_y), fill=(238, 238, 238, 200), width=2)

    # 绘制物件（从晚到早，晚的被早的遮住）
    for catch_object in sorted(render_objects, key=lambda o: -o.start_time):
        _draw_catch_object(frame, skin, catch_object, snapshot_time, layout, cache)

    return frame


def _draw_catch_object(
    frame: Image.Image,
    skin: CatchSkin,
    catch_object: CatchRenderObject,
    snapshot_time: int,
    layout: GifLayout,
    cache: dict,
) -> None:
    local_time = catch_object.start_time - snapshot_time
    # X: 游戏内 x（0-512）映射到帧内像素
    center_x = round(GIF_LEFT_PANEL_WIDTH + catch_object.x * layout.playfield_scale)
    # Y: 判定线在 STABLE_CATCHER_Y，水果在 local_time ms 前出现在上方
    # center_y = judgement_y - local_time * pixels_per_ms
    judgement_y = round(STABLE_CATCHER_Y * layout.playfield_scale)
    center_y = round(judgement_y - local_time * layout.pixels_per_ms)

    diameter = max(
        1,
        round(OBJECT_RADIUS * 2 * layout.object_scale * layout.playfield_scale * catch_object.scale_factor),
    )

    # 只绘制在帧内可见的物件（判定线以上）
    if center_y + diameter / 2 < 0 or center_y - diameter / 2 > judgement_y:
        return

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
        frame.paste(glow, (round(center_x - glow.width / 2), round(center_y - glow.height / 2)), glow)

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

    frame.paste(base, (round(center_x - base.width / 2), round(center_y - base.height / 2)), base)
    frame.paste(overlay, (round(center_x - overlay.width / 2), round(center_y - overlay.height / 2)), overlay)


def _draw_time_label(
    draw: ImageDraw.ImageDraw,
    start_time: int,
    duration_ms: int,
    frame_x: int,
    frame_y: int,
    layout: GifLayout,
    font_regular: ImageFont.ImageFont,
    font_note: ImageFont.ImageFont,
    is_preview: bool,
) -> None:
    label = f"{_format_time(start_time)} - {_format_time(start_time + duration_ms)}"
    color = GIF_PREVIEW_TIME_LABEL_COLOR if is_preview else GIF_TIME_LABEL_COLOR
    note_color = GIF_PREVIEW_TIME_LABEL_COLOR if is_preview else GIF_TIME_LABEL_NOTE_COLOR

    y = frame_y + GIF_IMAGE_HEIGHT + GIF_TIME_LABEL_TOP_GAP
    box = draw.textbbox((0, 0), label, font=font_regular)
    text_width = box[2] - box[0]
    text_height = box[3] - box[1]
    x = frame_x + (GIF_IMAGE_WIDTH - text_width) / 2
    draw.text((x, y), label, fill=color, font=font_regular)

    if is_preview:
        note = "Preview Time"
        note_box = draw.textbbox((0, 0), note, font=font_note)
        note_width = note_box[2] - note_box[0]
        note_x = frame_x + (GIF_IMAGE_WIDTH - note_width) / 2
        draw.text((note_x, y + text_height + GIF_TIME_LABEL_NOTE_TOP_GAP), note, fill=note_color, font=font_note)


def _format_time(ms: int) -> str:
    total_seconds = max(0, ms) // 1000
    return f"{total_seconds // 60:02d}:{total_seconds % 60:02d}"


def _catch_time_range(approach_rate: float) -> float:
    return _difficulty_range(approach_rate, 1800.0, 1200.0, 450.0)


def _difficulty_range(difficulty: float, minimum: float, middle: float, maximum: float) -> float:
    scaled = (difficulty - 5.0) / 5.0
    if difficulty > 5.0:
        return middle + (maximum - middle) * scaled
    if difficulty < 5.0:
        return middle + (middle - minimum) * scaled
    return middle


def _circle_scale(circle_size: float) -> float:
    return (1.0 - 0.7 * ((circle_size - 5.0) / 5.0)) / 2.0


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
