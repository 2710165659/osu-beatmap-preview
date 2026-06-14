from __future__ import annotations

import math
from dataclasses import dataclass

from PIL import Image, ImageDraw, ImageFont

from ..errors import PreviewError
from ..models import Beatmap, BreakPeriod, StandardHitObject
from ..mods import ModSettings
from ..time_selection import PreviewTimeSelector, times_to_milliseconds
from .config import (
    CANVAS_BACKGROUND_COLOR,
    GIF_DURATION_MS,
    GIF_FPS,
    GIF_GRID_GAP,
    GIF_IMAGES_PER_ROW,
    GIF_LOOP,
    GIF_ROW_COUNT,
    HORIZONTAL_PAGE_MARGIN,
    IMAGE_BACKGROUND_COLOR,
    IMAGE_HEIGHT,
    IMAGE_WIDTH,
    INTER_ROW_GAP,
    INTRA_ROW_IMAGE_GAP,
    LEFT_PANEL_BACKGROUND_COLOR,
    LEFT_PANEL_WIDTH,
    PNG_IMAGES_PER_ROW,
    PNG_MS_PER_IMAGE,
    PNG_ROW_COUNT,
    PREVIEW_TIME_LABEL_COLOR,
    TIME_LABEL_COLOR,
    TIME_LABEL_FONT_SIZE,
    TIME_LABEL_HEIGHT,
    TIME_LABEL_NOTE_COLOR,
    TIME_LABEL_NOTE_FONT_SIZE,
    TIME_LABEL_NOTE_TOP_GAP,
    TIME_LABEL_TOP_GAP,
    VERTICAL_PAGE_MARGIN,
)
from .skin import StandardSkin, load_standard_skin
from .slider_path import SliderPath, build_path, build_slider_path, path_position_at, slice_path, _build_slider_path_cached

# ——— osu! 源码相关常量 ———
PLAYFIELD_WIDTH = 512  # osu!standard 原始游玩区域宽度
PLAYFIELD_HEIGHT = 384  # osu!standard 原始游玩区域高度
PLAYFIELD_VIEWPORT_RATIO = 0.8  # osu!standard gameplay playfield 在 4:3 视口内的缩放比例
PLAYFIELD_STORYBOARD_SHIFT = 8  # lazer gameplay 为对齐 storyboard 对 playfield 施加的下移量
OBJECT_RADIUS = 64  # osu! 源码中 hit object 基础半径
BROKEN_GAMEFIELD_ROUNDING_ALLOWANCE = 1.00041  # osu!stable 历史圆大小修正
POST_HIT_FADE_MS = 120  # 普通 hit circle 命中后的短暂残留时间
SLIDER_FADE_OUT_MS = 240  # slider 结束后的淡出时间
SPINNER_FADE_OUT_MS = 240  # spinner 结束后的淡出时间
BREAK_MIN_DURATION_MS = 650  # osu! BreakPeriod.MIN_BREAK_DURATION，短于此值不产生 break overlay
BREAK_FADE_DURATION_MS = BREAK_MIN_DURATION_MS // 2  # osu! BreakOverlay.BREAK_FADE_DURATION
BREAK_OVERLAY_BAR_WIDTH_RATIO = 0.3  # osu! break 剩余时间条最大宽度占屏幕比例
BREAK_OVERLAY_BAR_HEIGHT = 8  # osu! break 剩余时间条高度
BREAK_OVERLAY_COUNTER_FONT_SIZE = 33  # osu! RemainingTimeCounter 数字字号
BREAK_OVERLAY_INFO_FONT_SIZE = 18  # 图片中央 break 时间说明字号
BREAK_OVERLAY_INFO_TOP_GAP = 14  # 剩余时间条与 break 时间说明之间的间距
BREAK_OVERLAY_COLOR = (238, 238, 238, 255)  # break overlay 主文字颜色
BREAK_OVERLAY_INFO_COLOR = (185, 185, 185, 255)  # break 时间说明颜色
SLIDER_BORDER_WIDTH = 10  # slider 轨道边框宽度 (osu! shadow 5px + border 7px = 12px ring)
SLIDER_BODY_SUPERSAMPLE = 2  # slider body 临时高分辨率绘制倍数，减少曲线接缝
SLIDER_LEGACY_BORDER_PORTION = 0.1875  # osu!stable legacy slider body 边框在半径中的占比
SLIDER_LEGACY_TRACK_ALPHA = 0.7  # legacy slider track override 固定透明度
SLIDER_LEGACY_SHADOW_ALPHA = 0.25  # legacy slider body 外侧阴影透明度
SNAKING_IN_SLIDERS = True  # 对应 osu! 设置项 Snaking in sliders 打开
SNAKING_OUT_SLIDERS = True  # 对应 osu! 设置项 Snaking out sliders 打开


# ——— dataclasses ———

@dataclass(frozen=True)
class RowTiming:
    start_time: int
    is_preview: bool
    break_periods: tuple[BreakPeriod, ...]


@dataclass(frozen=True)
class FrameLayout:
    playfield_left: int
    playfield_top: int
    scale: float


@dataclass(frozen=True)
class ComboInfo:
    color: tuple[int, int, int]
    number: int


@dataclass(frozen=True)
class RenderSettings:
    circle_radius: float
    circle_diameter: int
    preempt_ms: int
    fade_in_ms: float
    hidden: bool


@dataclass(frozen=True)
class SliderRenderData:
    world_path: SliderPath
    frame_path: SliderPath
    head_center: tuple[float, float]
    reverse_centers: tuple[tuple[float, float], ...]
    reverse_angles: tuple[float, ...]


@dataclass(frozen=True)
class CachedLayer:
    image: Image.Image
    offset: tuple[int, int]


@dataclass
class RenderCache:
    resized_alpha: dict[tuple[int, tuple[int, int], int], Image.Image]
    tinted: dict[tuple[int, tuple[int, int], tuple[int, int, int], int], Image.Image]
    digit_crops: dict[int, Image.Image]
    slider_data: dict[StandardHitObject, SliderRenderData]
    slider_body_layers: dict[
        tuple[StandardHitObject, int, int, tuple[int, int, int], tuple[int, int, int]],
        CachedLayer,
    ]
    slider_body_alpha_layers: dict[
        tuple[
            tuple[StandardHitObject, int, int, tuple[int, int, int], tuple[int, int, int]],
            int,
        ],
        Image.Image,
    ]
    rotated_reverse_arrows: dict[int, Image.Image]


@dataclass(frozen=True)
class RenderContext:
    hit_objects: list[StandardHitObject]
    combo_info: dict[int, ComboInfo]
    skin: StandardSkin
    settings: RenderSettings
    frame_layout: FrameLayout
    frame_circle_diameter: int
    slider_body_width: int
    slider_border_width: int
    spinner_size: int
    reverse_arrow_size: int
    slider_follow_size: int
    slider_ball_size: int
    cache: RenderCache


# ——— public API ———

def render_standard(
    beatmap: Beatmap,
    hit_objects: list[StandardHitObject],
    fmt: str,
    mods: ModSettings | None = None,
    times: list[float] | None = None,
) -> Image.Image | tuple:
    _build_slider_path_cached.cache_clear()
    try:
        if fmt == "png":
            return _render_png_grid(beatmap, hit_objects, mods)
        if fmt == "gif":
            return _render_gif(beatmap, hit_objects, mods, times)
        raise PreviewError(f"unsupported standard output format: {fmt}")
    finally:
        _build_slider_path_cached.cache_clear()


# ——— PNG grid ———

def _render_png_grid(beatmap: Beatmap, hit_objects: list[StandardHitObject], mods: ModSettings | None) -> Image.Image:
    hit_objects = _apply_standard_object_mods(hit_objects, mods)
    context = _build_render_context(beatmap, hit_objects, mods)
    row_timings = _choose_row_start_times(
        beatmap=beatmap,
        hit_objects=hit_objects,
        row_count=PNG_ROW_COUNT,
        images_per_row=PNG_IMAGES_PER_ROW,
        ms_per_row_duration=PNG_MS_PER_IMAGE,
    )
    font_regular = ImageFont.load_default(size=TIME_LABEL_FONT_SIZE)
    font_note = ImageFont.load_default(size=TIME_LABEL_NOTE_FONT_SIZE)
    canvas = Image.new("RGBA", _build_png_canvas_size(), CANVAS_BACKGROUND_COLOR)
    draw = ImageDraw.Draw(canvas)

    for row_index, row_timing in enumerate(row_timings):
        snapshot_times = tuple(row_timing.start_time + image_index * PNG_MS_PER_IMAGE for image_index in range(PNG_IMAGES_PER_ROW))
        visible_index_groups = _build_visible_indexes_by_snapshot(hit_objects, snapshot_times, context.settings.preempt_ms)
        y = VERTICAL_PAGE_MARGIN + row_index * (IMAGE_HEIGHT + TIME_LABEL_TOP_GAP + TIME_LABEL_HEIGHT + INTER_ROW_GAP)
        for image_index in range(PNG_IMAGES_PER_ROW):
            snapshot_time = snapshot_times[image_index]
            x = HORIZONTAL_PAGE_MARGIN + image_index * (IMAGE_WIDTH + INTRA_ROW_IMAGE_GAP)
            frame = _render_frame(
                context=context,
                snapshot_time=snapshot_time,
                break_periods=row_timing.break_periods if row_timing.is_preview else (),
                visible_indexes=visible_index_groups[image_index],
            )
            canvas.alpha_composite(frame, (x, y))
            note = _build_time_label_note(row_timing) if image_index == 0 else None
            is_preview_label = row_timing.is_preview and image_index == 0
            _draw_time_label(
                draw, _format_time(snapshot_time), x, y + IMAGE_HEIGHT + TIME_LABEL_TOP_GAP,
                font_regular, font_note, note,
                PREVIEW_TIME_LABEL_COLOR if is_preview_label else TIME_LABEL_COLOR,
                PREVIEW_TIME_LABEL_COLOR if is_preview_label else TIME_LABEL_NOTE_COLOR,
            )
    return canvas


# ——— GIF ———

def _render_gif(beatmap: Beatmap, hit_objects: list[StandardHitObject], mods: ModSettings | None, times: list[float] | None):
    if times is not None and len(times) > GIF_ROW_COUNT * GIF_IMAGES_PER_ROW:
        raise PreviewError("--times accepts at most 4 time points")

    hit_objects = _apply_standard_object_mods(hit_objects, mods)
    context = _build_render_context(beatmap, hit_objects, mods)
    speed_multiplier = mods.speed_multiplier if mods is not None else 1.0
    gameplay_segment_duration = round(GIF_DURATION_MS * speed_multiplier)
    row_timings = _choose_row_start_times(
        beatmap=beatmap,
        hit_objects=hit_objects,
        row_count=GIF_ROW_COUNT * GIF_IMAGES_PER_ROW,
        images_per_row=2,
        ms_per_row_duration=gameplay_segment_duration,
        requested_start_times=times_to_milliseconds(times),
    )
    font_regular = ImageFont.load_default(size=TIME_LABEL_FONT_SIZE)
    font_note = ImageFont.load_default(size=TIME_LABEL_NOTE_FONT_SIZE)
    canvas_size = _build_gif_canvas_size()
    frame_count = max(1, round(GIF_DURATION_MS * GIF_FPS / 1000))
    frame_duration_ms = max(1, round(1000 / GIF_FPS))
    segment_snapshot_times = [
        tuple(row_timing.start_time + round(frame_index * 1000 * speed_multiplier / GIF_FPS) for frame_index in range(frame_count))
        for row_timing in row_timings
    ]
    segment_visible_indexes = [
        _build_visible_indexes_by_snapshot(hit_objects, snapshot_times, context.settings.preempt_ms)
        for snapshot_times in segment_snapshot_times
    ]

    def frame_generator():
        for frame_index in range(frame_count):
            canvas = Image.new("RGBA", canvas_size, CANVAS_BACKGROUND_COLOR)
            draw = ImageDraw.Draw(canvas)

            for segment_index, row_timing in enumerate(row_timings):
                x, y = _gif_frame_origin(segment_index)
                snapshot_time = segment_snapshot_times[segment_index][frame_index]
                frame = _render_frame(
                    context=context,
                    snapshot_time=snapshot_time,
                    break_periods=row_timing.break_periods,
                    visible_indexes=segment_visible_indexes[segment_index][frame_index],
                )
                canvas.alpha_composite(frame, (x, y))
                note = _build_time_label_note(row_timing)
                is_preview_label = row_timing.is_preview
                _draw_time_label(
                    draw, _build_gif_time_label(row_timing.start_time, gameplay_segment_duration),
                    x, y + IMAGE_HEIGHT + TIME_LABEL_TOP_GAP,
                    font_regular, font_note, note,
                    PREVIEW_TIME_LABEL_COLOR if is_preview_label else TIME_LABEL_COLOR,
                    PREVIEW_TIME_LABEL_COLOR if is_preview_label else TIME_LABEL_NOTE_COLOR,
                )

            yield canvas

    return frame_generator(), frame_duration_ms, GIF_LOOP


# ——— row selection (merged from row_selection.py) ———

def _choose_row_start_times(
    beatmap: Beatmap,
    hit_objects: list[StandardHitObject],
    row_count: int,
    images_per_row: int,
    ms_per_row_duration: int,
    requested_start_times: list[int] | None = None,
) -> list[RowTiming]:
    row_duration = (images_per_row - 1) * ms_per_row_duration
    chosen = PreviewTimeSelector(
        beatmap=beatmap,
        hit_objects=hit_objects,
        segment_count=row_count,
        segment_duration=row_duration,
        requested_start_times=requested_start_times,
    ).choose()
    return [
        RowTiming(
            start_time=timing.start_time,
            is_preview=timing.is_preview,
            break_periods=timing.break_periods,
        )
        for timing in chosen
    ]


def _apply_standard_object_mods(
    hit_objects: list[StandardHitObject],
    mods: ModSettings | None,
) -> list[StandardHitObject]:
    if mods is None or not mods.hard_rock:
        return hit_objects

    reflected: list[StandardHitObject] = []
    for hit_object in hit_objects:
        reflected.append(
            StandardHitObject(
                x=hit_object.x,
                y=PLAYFIELD_HEIGHT - hit_object.y,
                start_time=hit_object.start_time,
                end_time=hit_object.end_time,
                hit_type=hit_object.hit_type,
                hitsound=hit_object.hitsound,
                new_combo=hit_object.new_combo,
                combo_offset=hit_object.combo_offset,
                slider_type=hit_object.slider_type,
                slider_points=tuple((x, PLAYFIELD_HEIGHT - y) for x, y in hit_object.slider_points),
                slider_repeats=hit_object.slider_repeats,
                slider_pixel_length=hit_object.slider_pixel_length,
                slider_edge_hitsounds=hit_object.slider_edge_hitsounds,
            )
        )
    return reflected


def _effective_difficulty(beatmap: Beatmap, mods: ModSettings | None) -> dict[str, float]:
    difficulty = {
        "CircleSize": float(beatmap.difficulty.get("CircleSize", "5")),
        "ApproachRate": float(beatmap.difficulty.get("ApproachRate", beatmap.difficulty.get("OverallDifficulty", "5"))),
        "OverallDifficulty": float(beatmap.difficulty.get("OverallDifficulty", "5")),
        "HPDrainRate": float(beatmap.difficulty.get("HPDrainRate", "5")),
    }
    if mods is None:
        return difficulty

    if mods.easy:
        difficulty["CircleSize"] *= 0.5
        difficulty["ApproachRate"] *= 0.5
        difficulty["OverallDifficulty"] *= 0.5
        difficulty["HPDrainRate"] *= 0.5

    if mods.hard_rock:
        difficulty["CircleSize"] = min(difficulty["CircleSize"] * 1.3, 10.0)
        difficulty["ApproachRate"] = min(difficulty["ApproachRate"] * 1.4, 10.0)
        difficulty["OverallDifficulty"] = min(difficulty["OverallDifficulty"] * 1.4, 10.0)
        difficulty["HPDrainRate"] = min(difficulty["HPDrainRate"] * 1.4, 10.0)

    if mods.has_da():
        if mods.da_cs is not None:
            difficulty["CircleSize"] = mods.da_cs
        if mods.da_ar is not None:
            difficulty["ApproachRate"] = mods.da_ar
        if mods.da_od is not None:
            difficulty["OverallDifficulty"] = mods.da_od
        if mods.da_hp is not None:
            difficulty["HPDrainRate"] = mods.da_hp

    return difficulty


# ——— render context ———

def _build_render_context(
    beatmap: Beatmap,
    hit_objects: list[StandardHitObject],
    mods: ModSettings | None,
) -> RenderContext:
    skin = load_standard_skin()
    settings = _build_render_settings(beatmap, mods)
    frame_layout = _build_frame_layout()
    combo_info = _build_combo_info(hit_objects, skin.combo_colors)
    frame_circle_diameter = max(1, round(settings.circle_diameter * frame_layout.scale))
    return RenderContext(
        hit_objects=hit_objects,
        combo_info=combo_info,
        skin=skin,
        settings=settings,
        frame_layout=frame_layout,
        frame_circle_diameter=frame_circle_diameter,
        slider_body_width=frame_circle_diameter,
        slider_border_width=max(1, round(SLIDER_BORDER_WIDTH * frame_layout.scale)),
        spinner_size=max(1, round(min(PLAYFIELD_WIDTH, PLAYFIELD_HEIGHT) * 0.95 * frame_layout.scale)),
        reverse_arrow_size=frame_circle_diameter,
        slider_follow_size=max(1, round(settings.circle_diameter * 2.4 * frame_layout.scale)),
        slider_ball_size=max(1, round(settings.circle_diameter * 1.15 * frame_layout.scale)),
        cache=RenderCache(
            resized_alpha={}, tinted={}, digit_crops={},
            slider_data={}, slider_body_layers={}, slider_body_alpha_layers={},
            rotated_reverse_arrows={},
        ),
    )


def _build_visible_indexes_by_snapshot(
    hit_objects: list[StandardHitObject],
    snapshot_times: tuple[int, ...],
    preempt_ms: int,
) -> tuple[tuple[int, ...], ...]:
    visible_starts = sorted(
        (hit_object.start_time - preempt_ms, index) for index, hit_object in enumerate(hit_objects)
    )
    visible_ends = sorted((_visible_end_time(hit_object), index) for index, hit_object in enumerate(hit_objects))
    active_indexes: list[int] = []
    start_pointer = 0
    end_pointer = 0
    visible_groups: list[tuple[int, ...]] = []

    for snapshot_time in snapshot_times:
        while start_pointer < len(visible_starts) and visible_starts[start_pointer][0] <= snapshot_time:
            active_indexes.append(visible_starts[start_pointer][1])
            start_pointer += 1
        while end_pointer < len(visible_ends) and visible_ends[end_pointer][0] < snapshot_time:
            ended_index = visible_ends[end_pointer][1]
            if ended_index in active_indexes:
                active_indexes.remove(ended_index)
            end_pointer += 1
        visible_groups.append(tuple(reversed(active_indexes)))

    return tuple(visible_groups)


def _build_render_settings(beatmap: Beatmap, mods: ModSettings | None) -> RenderSettings:
    difficulty = _effective_difficulty(beatmap, mods)
    circle_size = difficulty["CircleSize"]
    approach_rate = difficulty["ApproachRate"]
    scale = (1.0 - 0.7 * ((circle_size - 5.0) / 5.0)) / 2.0 * BROKEN_GAMEFIELD_ROUNDING_ALLOWANCE
    circle_radius = OBJECT_RADIUS * scale
    circle_diameter = max(1, round(circle_radius * 2))
    preempt_ms = _difficulty_range_int(approach_rate, 1800, 1200, 450)
    fade_in_ms = 400 * min(1, preempt_ms / 450)
    hidden = mods is not None and mods.hidden
    if hidden:
        fade_in_ms = preempt_ms * 0.4
    return RenderSettings(
        circle_radius=circle_radius, circle_diameter=circle_diameter,
        preempt_ms=preempt_ms, fade_in_ms=fade_in_ms, hidden=hidden,
    )


def _difficulty_range_int(difficulty: float, minimum: int, middle: int, maximum: int) -> int:
    if difficulty > 5:
        return int(middle + (maximum - middle) * ((difficulty - 5) / 5))
    if difficulty < 5:
        return int(middle + (middle - minimum) * ((difficulty - 5) / 5))
    return middle


def _build_frame_layout() -> FrameLayout:
    available_width = IMAGE_WIDTH - LEFT_PANEL_WIDTH
    viewport_width = min(available_width, IMAGE_HEIGHT * PLAYFIELD_WIDTH / PLAYFIELD_HEIGHT)
    playfield_width = round(viewport_width * PLAYFIELD_VIEWPORT_RATIO)
    playfield_height = round(playfield_width * PLAYFIELD_HEIGHT / PLAYFIELD_WIDTH)
    scale = playfield_width / PLAYFIELD_WIDTH
    return FrameLayout(
        playfield_left=LEFT_PANEL_WIDTH + (available_width - playfield_width) // 2,
        playfield_top=round((IMAGE_HEIGHT - playfield_height) / 2 + PLAYFIELD_STORYBOARD_SHIFT * scale),
        scale=scale,
    )


def _build_combo_info(hit_objects: list[StandardHitObject], combo_colors: list[tuple[int, int, int]]) -> dict[int, ComboInfo]:
    combo_info: dict[int, ComboInfo] = {}
    color_index = 0
    number = 0
    previous_was_spinner = False

    for index, hit_object in enumerate(hit_objects):
        starts_combo = index == 0 or previous_was_spinner or (hit_object.new_combo and not (hit_object.hit_type & 8))
        if starts_combo:
            if index > 0:
                color_index = (color_index + hit_object.combo_offset + 1) % len(combo_colors)
            number = 1
        else:
            number += 1

        combo_info[index] = ComboInfo(color=combo_colors[color_index], number=number)
        previous_was_spinner = bool(hit_object.hit_type & 8)

    return combo_info


def _build_png_canvas_size() -> tuple[int, int]:
    width = HORIZONTAL_PAGE_MARGIN * 2 + PNG_IMAGES_PER_ROW * IMAGE_WIDTH + (PNG_IMAGES_PER_ROW - 1) * INTRA_ROW_IMAGE_GAP
    row_height = IMAGE_HEIGHT + TIME_LABEL_TOP_GAP + TIME_LABEL_HEIGHT
    height = VERTICAL_PAGE_MARGIN * 2 + PNG_ROW_COUNT * row_height + (PNG_ROW_COUNT - 1) * INTER_ROW_GAP
    return width, height


def _build_gif_canvas_size() -> tuple[int, int]:
    row_height = IMAGE_HEIGHT + TIME_LABEL_TOP_GAP + TIME_LABEL_HEIGHT
    width = HORIZONTAL_PAGE_MARGIN * 2 + GIF_IMAGES_PER_ROW * IMAGE_WIDTH + (GIF_IMAGES_PER_ROW - 1) * GIF_GRID_GAP
    height = VERTICAL_PAGE_MARGIN * 2 + GIF_ROW_COUNT * row_height + (GIF_ROW_COUNT - 1) * GIF_GRID_GAP
    return width, height


def _gif_frame_origin(segment_index: int) -> tuple[int, int]:
    row_index = segment_index // GIF_IMAGES_PER_ROW
    image_index = segment_index % GIF_IMAGES_PER_ROW
    row_height = IMAGE_HEIGHT + TIME_LABEL_TOP_GAP + TIME_LABEL_HEIGHT
    x = HORIZONTAL_PAGE_MARGIN + image_index * (IMAGE_WIDTH + GIF_GRID_GAP)
    y = VERTICAL_PAGE_MARGIN + row_index * (row_height + GIF_GRID_GAP)
    return x, y


# ——— frame rendering ———

def _render_frame(
    context: RenderContext,
    snapshot_time: int,
    break_periods: tuple[BreakPeriod, ...],
    visible_indexes: tuple[int, ...],
) -> Image.Image:
    frame = Image.new("RGBA", (IMAGE_WIDTH, IMAGE_HEIGHT), IMAGE_BACKGROUND_COLOR)
    draw = ImageDraw.Draw(frame)
    draw.rectangle((0, 0, LEFT_PANEL_WIDTH, IMAGE_HEIGHT), fill=LEFT_PANEL_BACKGROUND_COLOR)

    for index in visible_indexes:
        hit_object = context.hit_objects[index]
        combo = context.combo_info[index]
        if hit_object.hit_type & 8:
            _draw_spinner(frame, context, hit_object, snapshot_time)
        elif hit_object.hit_type & 2:
            _draw_slider(frame, context, hit_object, combo, snapshot_time)
        else:
            _draw_hit_circle(frame, context, hit_object, combo, snapshot_time)

    for index in visible_indexes:
        hit_object = context.hit_objects[index]
        if not (hit_object.hit_type & 8):
            _draw_approach_circle(frame, context, hit_object, context.combo_info[index].color, snapshot_time)

    current_break = _current_break_period(break_periods, snapshot_time)
    if current_break is not None:
        _draw_break_overlay(frame, current_break, snapshot_time)

    return frame


def _visible_end_time(hit_object: StandardHitObject) -> int:
    if hit_object.hit_type & 2:
        return hit_object.end_time + SLIDER_FADE_OUT_MS
    if hit_object.hit_type & 8:
        return hit_object.end_time + SPINNER_FADE_OUT_MS
    return hit_object.start_time + POST_HIT_FADE_MS


# ——— hit circle ———

def _draw_hit_circle(
    frame: Image.Image, context: RenderContext,
    hit_object: StandardHitObject, combo: ComboInfo, snapshot_time: int,
) -> None:
    alpha = _object_alpha(hit_object.start_time, hit_object.start_time, snapshot_time, context.settings)
    center = _to_frame_point(hit_object.x, hit_object.y, context.frame_layout)
    _draw_circle_piece(frame, context, center, combo.color, alpha, str(combo.number))


# ——— slider ———

def _draw_slider(
    frame: Image.Image, context: RenderContext,
    hit_object: StandardHitObject, combo: ComboInfo, snapshot_time: int,
) -> None:
    alpha = _slider_body_alpha(hit_object, snapshot_time, context.settings)
    overlay_alpha = _normal_object_alpha(hit_object.start_time, hit_object.end_time, snapshot_time, context.settings)
    slider_data = _get_slider_render_data(hit_object, context)
    snaked_start, snaked_end = _slider_snaked_range(hit_object, snapshot_time, context.settings)
    if _is_full_slider_body(snaked_start, snaked_end):
        _draw_cached_slider_body(frame, context, hit_object, slider_data.frame_path.points,
                                 context.skin.slider_border, context.skin.slider_track, alpha)
    else:
        visible_path = slice_path(slider_data.frame_path, snaked_start, snaked_end)
        _draw_slider_body(frame, visible_path, context.slider_body_width, context.slider_border_width,
                          context.skin.slider_border, context.skin.slider_track, alpha)

    _draw_slider_reverse_arrows(frame, context, slider_data, hit_object, snapshot_time, snaked_start, snaked_end, overlay_alpha)
    _draw_slider_ball(frame, context, slider_data, hit_object, snapshot_time, overlay_alpha)
    head_alpha = _slider_head_alpha(hit_object, snapshot_time, context.settings, snaked_start, snaked_end)
    if head_alpha > 0:
        _draw_circle_piece(frame, context, slider_data.head_center, combo.color, head_alpha, str(combo.number))


def _slider_snaked_range(hit_object: StandardHitObject, snapshot_time: int, settings: RenderSettings) -> tuple[float, float]:
    span_count = max(1, hit_object.slider_repeats)
    start = 0.0
    end = 1.0

    if snapshot_time < hit_object.start_time:
        if SNAKING_IN_SLIDERS:
            snake_start = hit_object.start_time - settings.preempt_ms
            end = max(0.0, min(1.0, (snapshot_time - snake_start) / (settings.preempt_ms / 3)))
        return start, end

    effective_time = min(snapshot_time, hit_object.end_time)
    completion = max(0.0, min(1.0, (effective_time - hit_object.start_time) / max(1, hit_object.end_time - hit_object.start_time)))
    span = min(span_count - 1, int(completion * span_count))
    span_progress = _slider_path_progress(span_count, completion)

    if span >= span_count - 1 and SNAKING_OUT_SLIDERS:
        if span % 2 == 1:
            end = span_progress
        else:
            start = span_progress

    return start, end


# ——— spinner ———

def _draw_spinner(frame: Image.Image, context: RenderContext, hit_object: StandardHitObject, snapshot_time: int) -> None:
    alpha = _spinner_alpha(hit_object, snapshot_time, context.settings)
    center = _to_frame_point(PLAYFIELD_WIDTH / 2, PLAYFIELD_HEIGHT / 2, context.frame_layout)
    sprite = _resize_with_alpha(context.skin.spinner_circle, context.spinner_size, alpha, context.cache)
    frame.alpha_composite(sprite, (round(center[0] - sprite.width / 2), round(center[1] - sprite.height / 2)))


# ——— approach circle ———

def _draw_approach_circle(
    frame: Image.Image, context: RenderContext,
    hit_object: StandardHitObject, color: tuple[int, int, int], snapshot_time: int,
) -> None:
    if context.settings.hidden:
        return
    if snapshot_time >= hit_object.start_time:
        return

    elapsed = snapshot_time - (hit_object.start_time - context.settings.preempt_ms)
    progress = max(0.0, min(1.0, elapsed / context.settings.preempt_ms))
    alpha = 0.9 * min(1.0, elapsed / max(1.0, context.settings.fade_in_ms * 2))
    approach_scale = 4 - 3 * progress
    size = max(1, round(context.settings.circle_diameter * approach_scale * context.frame_layout.scale))
    center = _to_frame_point(hit_object.x, hit_object.y, context.frame_layout)
    sprite = _tint_sprite(context.skin.approachcircle, color, alpha, size, context.cache)
    frame.alpha_composite(sprite, (round(center[0] - sprite.width / 2), round(center[1] - sprite.height / 2)))


# ——— alpha / timing helpers ———

def _object_alpha(start_time: int, end_time: int, snapshot_time: int, settings: RenderSettings) -> float:
    if settings.hidden:
        return _hidden_object_alpha(start_time, end_time, snapshot_time, settings)
    return _normal_object_alpha(start_time, end_time, snapshot_time, settings)


def _normal_object_alpha(start_time: int, end_time: int, snapshot_time: int, settings: RenderSettings) -> float:
    if snapshot_time < start_time:
        fade_start = start_time - settings.preempt_ms
        return max(0.0, min(1.0, (snapshot_time - fade_start) / settings.fade_in_ms))
    if snapshot_time <= end_time:
        return 1.0
    return max(0.0, 1.0 - (snapshot_time - end_time) / SLIDER_FADE_OUT_MS)


def _hidden_object_alpha(start_time: int, end_time: int, snapshot_time: int, settings: RenderSettings) -> float:
    if end_time > start_time:
        return _hidden_slider_body_alpha(start_time, end_time, snapshot_time, settings)

    fade_start = start_time - settings.preempt_ms
    fade_in_end = fade_start + settings.fade_in_ms
    if snapshot_time < start_time:
        fade_in_alpha = max(0.0, min(1.0, (snapshot_time - fade_start) / max(1.0, settings.fade_in_ms)))
        fade_out_end = fade_in_end + settings.preempt_ms * 0.3
        fade_out_alpha = max(0.0, min(1.0, 1.0 - (snapshot_time - fade_in_end) / max(1.0, fade_out_end - fade_in_end)))
        return min(fade_in_alpha, fade_out_alpha)

    if snapshot_time <= end_time:
        return 0.0

    return 0.0


def _slider_body_alpha(hit_object: StandardHitObject, snapshot_time: int, settings: RenderSettings) -> float:
    if settings.hidden:
        return _hidden_slider_body_alpha(hit_object.start_time, hit_object.end_time, snapshot_time, settings)
    return _normal_object_alpha(hit_object.start_time, hit_object.end_time, snapshot_time, settings)


def _hidden_slider_body_alpha(start_time: int, end_time: int, snapshot_time: int, settings: RenderSettings) -> float:
    fade_start = start_time - settings.preempt_ms
    fade_in_end = fade_start + settings.fade_in_ms
    if snapshot_time < fade_in_end:
        return max(0.0, min(1.0, (snapshot_time - fade_start) / max(1.0, settings.fade_in_ms)))
    if snapshot_time <= end_time:
        return max(0.0, min(1.0, 1.0 - (snapshot_time - fade_in_end) / max(1.0, end_time - fade_in_end)))
    return max(0.0, 1.0 - (snapshot_time - end_time) / SLIDER_FADE_OUT_MS)


def _spinner_alpha(hit_object: StandardHitObject, snapshot_time: int, settings: RenderSettings) -> float:
    if not settings.hidden:
        return _normal_object_alpha(hit_object.start_time, hit_object.end_time, snapshot_time, settings)

    fade_start = hit_object.start_time - settings.fade_in_ms
    if snapshot_time < hit_object.start_time:
        return max(0.0, min(1.0, (snapshot_time - fade_start) / max(1.0, settings.fade_in_ms)))
    if snapshot_time <= hit_object.end_time:
        return 1.0
    return max(0.0, 1.0 - (snapshot_time - hit_object.end_time) / max(1.0, settings.preempt_ms * 0.3))


def _slider_head_alpha(
    hit_object: StandardHitObject, snapshot_time: int,
    settings: RenderSettings, snaked_start: float, snaked_end: float,
) -> float:
    if snaked_start > 0.001 or snaked_end <= 0.001:
        return 0.0
    if snapshot_time < hit_object.start_time:
        return _object_alpha(hit_object.start_time, hit_object.start_time, snapshot_time, settings)
    if settings.hidden:
        return 0.0
    if snapshot_time <= hit_object.start_time + POST_HIT_FADE_MS:
        return 1.0 - (snapshot_time - hit_object.start_time) / POST_HIT_FADE_MS
    return 0.0


def _slider_path_progress(span_count: int, completion: float) -> float:
    span = min(span_count - 1, int(completion * span_count))
    progress = (completion * span_count) % 1
    if completion >= 1:
        progress = 1
    if span % 2 == 1:
        progress = 1 - progress
    return progress


# ——— slider ball ———

def _draw_slider_ball(
    frame: Image.Image, context: RenderContext,
    slider_data: SliderRenderData, hit_object: StandardHitObject,
    snapshot_time: int, alpha: float,
) -> None:
    if not (hit_object.start_time <= snapshot_time <= hit_object.end_time):
        return

    completion = (snapshot_time - hit_object.start_time) / max(1, hit_object.end_time - hit_object.start_time)
    progress = _slider_path_progress(max(1, hit_object.slider_repeats), completion)
    center = path_position_at(slider_data.frame_path, progress)
    follow_circle = _resize_with_alpha(context.skin.slider_follow_circle, context.slider_follow_size, alpha * 0.7, context.cache)
    ball = _resize_with_alpha(context.skin.slider_ball, context.slider_ball_size, alpha, context.cache)
    frame.alpha_composite(follow_circle, (round(center[0] - follow_circle.width / 2), round(center[1] - follow_circle.height / 2)))
    frame.alpha_composite(ball, (round(center[0] - ball.width / 2), round(center[1] - ball.height / 2)))


# ——— circle piece (hitcircle + overlay + number) ———

def _draw_circle_piece(
    frame: Image.Image, context: RenderContext,
    center: tuple[float, float], color: tuple[int, int, int],
    alpha: float, number: str | None,
) -> None:
    hitcircle = _tint_sprite(context.skin.hitcircle, color, alpha, context.frame_circle_diameter, context.cache)
    overlay = _resize_with_alpha(context.skin.hitcircle_overlay, context.frame_circle_diameter, alpha, context.cache)
    position = (round(center[0] - context.frame_circle_diameter / 2), round(center[1] - context.frame_circle_diameter / 2))
    frame.alpha_composite(hitcircle, position)
    frame.alpha_composite(overlay, position)
    if number is not None:
        _draw_number(frame, context, number, center, context.frame_circle_diameter, alpha)


def _draw_number(
    frame: Image.Image, context: RenderContext,
    number: str, center: tuple[float, float],
    circle_diameter: int, alpha: float,
) -> None:
    digit_height = max(1, round(circle_diameter * 0.52))
    digit_images = [_resize_digit(context.skin.digits[digit], digit_height, alpha, context.cache) for digit in number]
    overlap = round(context.skin.hitcircle_overlap * digit_height / 100)
    total_width = sum(image.width for image in digit_images) - overlap * (len(digit_images) - 1)
    x = round(center[0] - total_width / 2)
    y = round(center[1] - digit_height / 2)

    for digit_image in digit_images:
        frame.alpha_composite(digit_image, (x, y))
        x += digit_image.width - overlap


def _resize_digit(sprite: Image.Image, height: int, alpha: float, cache: RenderCache) -> Image.Image:
    cropped = cache.digit_crops.get(id(sprite))
    if cropped is None:
        box = sprite.getbbox()
        cropped = sprite.crop(box)
        cache.digit_crops[id(sprite)] = cropped
    width = max(1, round(cropped.width * height / cropped.height))
    return _resize_with_alpha(cropped, (width, height), alpha, cache)


# ——— slider body rendering ———

def _draw_slider_body(
    frame: Image.Image, points: list[tuple[float, float]],
    width: int, border_width: int,
    border_color: tuple[int, int, int], track_color: tuple[int, int, int], alpha: float,
) -> None:
    if len(points) < 2:
        return
    layer = _render_slider_body_layer(points, width, border_width, border_color, track_color, _alpha_to_byte(alpha))
    frame.alpha_composite(layer.image, layer.offset)


def _draw_cached_slider_body(
    frame: Image.Image, context: RenderContext,
    hit_object: StandardHitObject, points: tuple[tuple[float, float], ...],
    border_color: tuple[int, int, int], track_color: tuple[int, int, int], alpha: float,
) -> None:
    body_key = (hit_object, context.slider_body_width, context.slider_border_width, border_color, track_color)
    layer = context.cache.slider_body_layers.get(body_key)
    if layer is None:
        layer = _render_slider_body_layer(points, context.slider_body_width, context.slider_border_width,
                                          border_color, track_color, 255)
        context.cache.slider_body_layers[body_key] = layer

    alpha_key = _alpha_to_byte(alpha)
    if alpha_key >= 255:
        frame.alpha_composite(layer.image, layer.offset)
        return

    tinted_key = (body_key, alpha_key)
    alpha_layer = context.cache.slider_body_alpha_layers.get(tinted_key)
    if alpha_layer is None:
        r, g, b, a = layer.image.split()
        a = a.point(lambda value: round(value * (alpha_key / 255)))
        alpha_layer = Image.merge("RGBA", (r, g, b, a))
        context.cache.slider_body_alpha_layers[tinted_key] = alpha_layer
    frame.alpha_composite(alpha_layer, layer.offset)


def _render_slider_body_layer(
    points: list[tuple[float, float]] | tuple[tuple[float, float], ...],
    width: int, border_width: int,
    border_color: tuple[int, int, int], track_color: tuple[int, int, int], alpha_byte: int,
) -> CachedLayer:
    scale = SLIDER_BODY_SUPERSAMPLE
    shadow_width = max(1, width + border_width)
    pad = shadow_width + 4
    xs = [point[0] for point in points]
    ys = [point[1] for point in points]
    left = max(0, math.floor(min(xs) - pad))
    top = max(0, math.floor(min(ys) - pad))
    right = min(IMAGE_WIDTH, math.ceil(max(xs) + pad))
    bottom = min(IMAGE_HEIGHT, math.ceil(max(ys) + pad))

    layer = Image.new("RGBA", (max(1, (right - left) * scale), max(1, (bottom - top) * scale)), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    scaled_points = [((x - left) * scale, (y - top) * scale) for x, y in points]
    accent_width = max(1, round(width * (1 - SLIDER_LEGACY_BORDER_PORTION)))
    middle_width = max(1, round(accent_width * 0.72))
    inner_width = max(1, round(accent_width * 0.44))
    outer_track = _darken(track_color, 0.1)
    inner_track = _legacy_lighten(track_color, 0.5)
    middle_track = _mix_rgb(outer_track, inner_track, 0.5)
    alpha = alpha_byte / 255

    _draw_round_path(draw, scaled_points, shadow_width * scale, (0, 0, 0, round(255 * alpha * SLIDER_LEGACY_SHADOW_ALPHA)))
    _draw_round_path(draw, scaled_points, width * scale, (*border_color, round(255 * alpha)))
    _draw_round_path(draw, scaled_points, accent_width * scale, (*outer_track, round(255 * alpha * SLIDER_LEGACY_TRACK_ALPHA)))
    _draw_round_path(draw, scaled_points, middle_width * scale, (*middle_track, round(255 * alpha * SLIDER_LEGACY_TRACK_ALPHA)))
    _draw_round_path(draw, scaled_points, inner_width * scale, (*inner_track, round(255 * alpha * SLIDER_LEGACY_TRACK_ALPHA)))
    resized_layer = layer.resize((max(1, right - left), max(1, bottom - top)), Image.Resampling.LANCZOS)
    return CachedLayer(resized_layer, (left, top))


def _draw_round_path(draw: ImageDraw.ImageDraw, points: list[tuple[float, float]],
                     width: int, color: tuple[int, int, int, int]) -> None:
    draw.line(points, fill=color, width=width, joint="curve")
    radius = width / 2
    for x, y in points:
        draw.ellipse((x - radius, y - radius, x + radius, y + radius), fill=color)


def _darken(color: tuple[int, int, int], amount: float) -> tuple[int, int, int]:
    return tuple(round(channel * (1 - amount)) for channel in color)


def _legacy_lighten(color: tuple[int, int, int], amount: float) -> tuple[int, int, int]:
    amount *= 0.5
    return tuple(min(255, round(channel * (1 + 0.5 * amount) + 255 * amount)) for channel in color)


def _mix_rgb(first: tuple[int, int, int], second: tuple[int, int, int], amount: float) -> tuple[int, int, int]:
    return tuple(round(first[index] * (1 - amount) + second[index] * amount) for index in range(3))


# ——— slider data & helpers ———

def _get_slider_render_data(hit_object: StandardHitObject, context: RenderContext) -> SliderRenderData:
    cached = context.cache.slider_data.get(hit_object)
    if cached is not None:
        return cached

    world_path = build_slider_path(hit_object)
    frame_points = tuple(_to_frame_point(x, y, context.frame_layout) for x, y in world_path.points)
    frame_path = build_path(frame_points)
    reverse_centers: list[tuple[float, float]] = []
    reverse_angles: list[float] = []
    for repeat_index in range(1, hit_object.slider_repeats):
        if repeat_index % 2 == 1:
            center = frame_path.points[-1]
            dx = frame_path.points[-2][0] - center[0]
            dy = frame_path.points[-2][1] - center[1]
        else:
            center = frame_path.points[0]
            dx = frame_path.points[1][0] - center[0]
            dy = frame_path.points[1][1] - center[1]
        reverse_centers.append(center)
        reverse_angles.append(math.atan2(dy, dx))
    slider_data = SliderRenderData(
        world_path=world_path, frame_path=frame_path,
        head_center=frame_path.points[0],
        reverse_centers=tuple(reverse_centers), reverse_angles=tuple(reverse_angles),
    )
    context.cache.slider_data[hit_object] = slider_data
    return slider_data


def _is_full_slider_body(snaked_start: float, snaked_end: float) -> bool:
    return snaked_start <= 0.001 and snaked_end >= 0.999


# ——— reverse arrows ———

def _draw_slider_reverse_arrows(
    frame: Image.Image, context: RenderContext,
    slider_data: SliderRenderData, hit_object: StandardHitObject,
    snapshot_time: int, snaked_start: float, snaked_end: float, alpha: float,
) -> None:
    if hit_object.slider_repeats <= 1:
        return

    span_count = max(1, hit_object.slider_repeats)
    fade_out_ratio = min(300, (hit_object.end_time - hit_object.start_time) / span_count) / max(
        1, hit_object.end_time - hit_object.start_time
    )

    for i, center in enumerate(slider_data.reverse_centers):
        repeat_index = i + 1
        position = 1.0 if repeat_index % 2 == 1 else 0.0

        if not (snaked_start - 0.001 <= position <= snaked_end + 0.001):
            continue

        if snapshot_time < hit_object.start_time:
            if repeat_index > 1:
                continue
            repeat_alpha = 1.0
        else:
            completion = (snapshot_time - hit_object.start_time) / max(1, hit_object.end_time - hit_object.start_time)
            traversal = completion * span_count
            if traversal < repeat_index - 1:
                continue
            if traversal >= repeat_index:
                continue
            if traversal > repeat_index - fade_out_ratio:
                repeat_alpha = max(0.0, (repeat_index - traversal) / fade_out_ratio)
            else:
                repeat_alpha = 1.0

        effective_alpha = alpha * repeat_alpha
        if effective_alpha <= 0:
            continue

        angle_deg = -math.degrees(slider_data.reverse_angles[i])
        angle_key = round(angle_deg)
        rotated = context.cache.rotated_reverse_arrows.get(angle_key)
        if rotated is None:
            rotated = context.skin.reverse_arrow.rotate(angle_deg, expand=True, resample=Image.Resampling.BICUBIC)
            context.cache.rotated_reverse_arrows[angle_key] = rotated
        arrow = _resize_with_alpha(rotated, context.reverse_arrow_size, effective_alpha, context.cache)
        frame.alpha_composite(arrow, (round(center[0] - arrow.width / 2), round(center[1] - arrow.height / 2)))


# ——— sprite helpers ———

def _target_size(size: int | tuple[int, int]) -> tuple[int, int]:
    if isinstance(size, int):
        return (size, size)
    return size


def _alpha_to_byte(alpha: float) -> int:
    return max(0, min(255, round(alpha * 255)))


def _tint_sprite(sprite: Image.Image, color: tuple[int, int, int], alpha: float,
                 size: int | tuple[int, int], cache: RenderCache) -> Image.Image:
    target_size = _target_size(size)
    alpha_key = _alpha_to_byte(alpha)
    key = (id(sprite), target_size, color, alpha_key)
    cached = cache.tinted.get(key)
    if cached is not None:
        return cached

    resized = _resize_with_alpha(sprite, target_size, alpha, cache)
    mask = resized.getchannel("A")
    tinted = Image.new("RGBA", resized.size, (*color, 0))
    tinted.putalpha(mask)
    cache.tinted[key] = tinted
    return tinted


def _resize_with_alpha(sprite: Image.Image, size: int | tuple[int, int],
                       alpha: float, cache: RenderCache) -> Image.Image:
    target_size = _target_size(size)
    alpha_key = _alpha_to_byte(alpha)
    key = (id(sprite), target_size, alpha_key)
    cached = cache.resized_alpha.get(key)
    if cached is not None:
        return cached

    resized = sprite.resize(target_size, Image.Resampling.LANCZOS)
    if alpha_key < 255:
        alpha_channel = resized.getchannel("A").point(lambda value: round(value * (alpha_key / 255)))
        resized.putalpha(alpha_channel)
    cache.resized_alpha[key] = resized
    return resized


def _to_frame_point(x: float, y: float, frame_layout: FrameLayout) -> tuple[float, float]:
    return (frame_layout.playfield_left + x * frame_layout.scale,
            frame_layout.playfield_top + y * frame_layout.scale)


# ——— time labels ———

def _draw_time_label(
    draw: ImageDraw.ImageDraw, label: str, x: int, y: int,
    font: ImageFont.ImageFont, note_font: ImageFont.ImageFont,
    note: str | None, label_color: tuple[int, int, int, int], note_color: tuple[int, int, int, int],
) -> None:
    _draw_centered_text(draw, label, x, y, font, label_color)
    if note is not None:
        text_box = draw.textbbox((0, 0), label, font=font)
        note_y = y + (text_box[3] - text_box[1]) + TIME_LABEL_NOTE_TOP_GAP
        _draw_centered_text(draw, note, x, note_y, note_font, note_color)


def _draw_centered_text(draw: ImageDraw.ImageDraw, text: str, x: int, y: int,
                        font: ImageFont.ImageFont, color: tuple[int, int, int, int]) -> None:
    text_box = draw.textbbox((0, 0), text, font=font)
    text_width = text_box[2] - text_box[0]
    text_x = x + (IMAGE_WIDTH - text_width) / 2
    draw.text((text_x, y), text, fill=color, font=font)


def _build_time_label_note(row_timing: RowTiming) -> str | None:
    if not row_timing.is_preview:
        return None
    return "Preview Time"


def _build_gif_time_label(start_time: int, duration_ms: int = GIF_DURATION_MS) -> str:
    end_time = start_time + duration_ms
    return f"{_format_time(start_time)} - {_format_time(end_time)}"


def _format_time(time_ms: int) -> str:
    minutes = time_ms // 60000
    seconds = (time_ms % 60000) // 1000
    milliseconds = time_ms % 1000
    return f"{minutes:02d}:{seconds:02d}:{milliseconds:03d}"


# ——— break overlay ———

def _current_break_period(break_periods: tuple[BreakPeriod, ...], snapshot_time: int) -> BreakPeriod | None:
    for period in break_periods:
        if _break_overlay_alpha(period, snapshot_time) > 0:
            return period
    return None


def _draw_break_overlay(frame: Image.Image, break_period: BreakPeriod, snapshot_time: int) -> None:
    alpha = _break_overlay_alpha(break_period, snapshot_time)
    if alpha <= 0:
        return

    layer = Image.new("RGBA", frame.size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    center_x = IMAGE_WIDTH / 2
    center_y = IMAGE_HEIGHT / 2
    counter_font = ImageFont.load_default(size=BREAK_OVERLAY_COUNTER_FONT_SIZE)
    info_font = ImageFont.load_default(size=BREAK_OVERLAY_INFO_FONT_SIZE)

    _draw_break_arrows(draw, alpha)
    _draw_break_remaining_bar(draw, break_period, snapshot_time, center_x, center_y, alpha)

    remaining_seconds = max(0, (break_period.end_time - snapshot_time + 999) // 1000)
    counter_label = str(remaining_seconds)
    counter_box = draw.textbbox((0, 0), counter_label, font=counter_font)
    counter_y = center_y - 15 - (counter_box[3] - counter_box[1])
    counter_color = (BREAK_OVERLAY_COLOR[0], BREAK_OVERLAY_COLOR[1], BREAK_OVERLAY_COLOR[2], round(BREAK_OVERLAY_COLOR[3] * alpha))
    _draw_centered_text(draw, counter_label, 0, counter_y, counter_font, counter_color)

    break_label = f"Break {_format_time(break_period.start_time)} - {_format_time(break_period.end_time)}"
    info_y = center_y + BREAK_OVERLAY_INFO_TOP_GAP
    info_color = (BREAK_OVERLAY_INFO_COLOR[0], BREAK_OVERLAY_INFO_COLOR[1], BREAK_OVERLAY_INFO_COLOR[2], round(BREAK_OVERLAY_INFO_COLOR[3] * alpha))
    _draw_centered_text(draw, break_label, 0, info_y, info_font, info_color)
    frame.alpha_composite(layer)


def _draw_break_remaining_bar(draw: ImageDraw.ImageDraw, break_period: BreakPeriod,
                              snapshot_time: int, center_x: float, center_y: float, alpha: float) -> None:
    track_width = round(IMAGE_WIDTH * BREAK_OVERLAY_BAR_WIDTH_RATIO)
    track_height = BREAK_OVERLAY_BAR_HEIGHT
    track_left = center_x - track_width / 2
    track_top = center_y - track_height / 2
    track_bounds = (track_left, track_top, track_left + track_width, track_top + track_height)
    draw.rounded_rectangle(track_bounds, radius=track_height / 2, fill=(48, 48, 48, round(150 * alpha)))

    remaining_ratio = _break_remaining_bar_ratio(break_period, snapshot_time)
    fill_width = track_width * remaining_ratio
    fill_left = center_x - fill_width / 2
    fill_bounds = (fill_left, track_top, fill_left + fill_width, track_top + track_height)
    draw.rounded_rectangle(fill_bounds, radius=track_height / 2, fill=(238, 238, 238, round(230 * alpha)))


def _draw_break_arrows(draw: ImageDraw.ImageDraw, alpha: float) -> None:
    color = (238, 238, 238, round(80 * alpha))
    glow_color = (238, 238, 238, round(35 * alpha))
    center_y = IMAGE_HEIGHT / 2
    for offset, direction in ((-0.22, 1), (0.22, -1)):
        center_x = IMAGE_WIDTH / 2 + IMAGE_WIDTH * offset
        _draw_chevron(draw, center_x, center_y, 32, direction, glow_color, 9)
        _draw_chevron(draw, center_x, center_y, 20, direction, color, 4)


def _draw_chevron(draw: ImageDraw.ImageDraw, center_x: float, center_y: float, size: int,
                  direction: int, color: tuple[int, int, int, int], width: int) -> None:
    half = size / 2
    point = (center_x + direction * half, center_y)
    top = (center_x - direction * half, center_y - half)
    bottom = (center_x - direction * half, center_y + half)
    draw.line((top, point, bottom), fill=color, width=width, joint="curve")


def _break_overlay_alpha(break_period: BreakPeriod, snapshot_time: int) -> float:
    if break_period.end_time - break_period.start_time < BREAK_MIN_DURATION_MS:
        return 0.0
    if snapshot_time < break_period.start_time or snapshot_time > break_period.end_time:
        return 0.0
    if snapshot_time < break_period.start_time + BREAK_FADE_DURATION_MS:
        return (snapshot_time - break_period.start_time) / BREAK_FADE_DURATION_MS
    if snapshot_time > break_period.end_time - BREAK_FADE_DURATION_MS:
        return (break_period.end_time - snapshot_time) / BREAK_FADE_DURATION_MS
    return 1.0


def _break_remaining_bar_ratio(break_period: BreakPeriod, snapshot_time: int) -> float:
    effective_duration = break_period.end_time - BREAK_FADE_DURATION_MS - break_period.start_time
    if effective_duration <= 0:
        return 0.0
    remaining = break_period.end_time - BREAK_FADE_DURATION_MS - snapshot_time
    return max(0.0, min(1.0, remaining / effective_duration))
