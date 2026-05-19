from __future__ import annotations

import math
from dataclasses import dataclass
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

from ..errors import PreviewError
from ..models import Beatmap, TaikoHitObject
from ..mods import ModSettings
from .config import (
    BASE_ROW_WIDTH_0_TO_1_MIN,
    BASE_ROW_WIDTH_1_TO_2_MIN,
    BASE_ROW_WIDTH_2_TO_3_MIN,
    BASE_ROW_WIDTH_3_TO_4_MIN,
    BASE_ROW_WIDTH_4_TO_5_MIN,
    BASE_ROW_WIDTH_5_TO_6_MIN,
    BASE_ROW_WIDTH_6_TO_10_MIN,
    BEAT_LINE_COLOR,
    BIG_NOTE_SCALE,
    CENTRE_NOTE_COLOR,
    DRAW_DRUM_EACH_ROW,
    IMAGE_BACKGROUND,
    ACCENT_LABEL_COLOR,
    BPM_FONT_SIZE,
    BPM_TOP_GAP,
    LABEL_RIGHT_PADDING,
    SV_TEXT_COLOR,
    SV_TEXT_FONT_SIZE,
    SV_TOP_GAP,
    MAX_SUPPORTED_DURATION_MS,
    MIN_BEAT_LINE_SPACING,
    NORMAL_NOTE_SIZE_RATIO,
    PAGE_MARGIN_X,
    PAGE_MARGIN_Y,
    RIM_NOTE_COLOR,
    ROLL_COLOR,
    ROW_GAP,
    ROW_HEIGHT,
    ROW_INNER_PADDING_X,
    ROW_WIDTH_BPM_0_TO_180,
    ROW_WIDTH_BPM_180_TO_240,
    ROW_WIDTH_BPM_240_TO_300,
    ROW_WIDTH_BPM_300_PLUS,
    RULER_TEXT_COLOR,
    SPACING_BPM,
    SPAN_BODY_HEIGHT_RATIO,
    SWELL_BODY_HEIGHT_RATIO,
    SWELL_COLOR,
    TIME_LABEL_FONT_SIZE,
    TIME_LABEL_NOTE_FONT_SIZE,
    TIME_LABEL_NOTE_TOP_GAP,
    TIME_LABEL_TOP_GAP,
)
from .skin import TaikoSkin, load_taiko_skin
from .timing import (
    RedlineSection,
    ScrollPositionMapper,
    SvChange,
    TimingLine,
    build_kiai_sections,
    build_redline_sections,
    build_scroll_mapper,
    build_sv_changes,
    build_timing_lines,
)

HIT_SOUNDS_RIM = 2 | 8
HIT_SOUNDS_STRONG = 4
DRUMROLL_FLAG = 2
SWELL_FLAG = 8


@dataclass(frozen=True)
class RenderLayout:
    row_count: int
    max_row_width: int
    left_panel_width: int
    right_panel_width: int
    content_width: int
    image_width: int
    image_height: int
    normal_note_diameter: int
    big_note_diameter: int


def render_taiko_grid(
    beatmap: Beatmap,
    output_path: Path,
    mods: ModSettings | None = None,
) -> Path:
    """把 osu!taiko 谱面渲染为横向多行预览图。"""
    hit_objects = _apply_taiko_object_mods(
        [ho for ho in beatmap.hit_objects if isinstance(ho, TaikoHitObject)],
        mods,
    )
    if not hit_objects:
        raise PreviewError("taiko beatmap has no hit objects")

    chart_end_time = max(hit_object.end_time for hit_object in hit_objects)
    if chart_end_time >= MAX_SUPPORTED_DURATION_MS:
        raise PreviewError("songs longer than 10 minutes are not supported")

    skin = load_taiko_skin()
    render_cache: dict = {}
    slider_multiplier = _effective_slider_multiplier(beatmap, mods)
    timing_points = _effective_timing_points(beatmap, mods)
    mapper = build_scroll_mapper(
        timing_points=timing_points,
        chart_end_time=chart_end_time,
        slider_multiplier=slider_multiplier,
        spacing_bpm=SPACING_BPM,
    )
    redline_sections = build_redline_sections(timing_points, chart_end_time)
    kiai_sections = build_kiai_sections(timing_points, chart_end_time)
    layout = _build_layout(
        skin=skin,
        beatmap_duration=chart_end_time,
        chart_width=mapper.end_position,
        redline_sections=redline_sections,
    )
    first_note_time = min(hit_object.start_time for hit_object in hit_objects) if hit_objects else 0
    timing_lines = build_timing_lines(
        redline_sections=redline_sections,
        mapper=mapper,
        min_beat_line_spacing=MIN_BEAT_LINE_SPACING,
        kiai_sections=kiai_sections,
        first_note_time=first_note_time,
    )
    font_regular = ImageFont.load_default(size=TIME_LABEL_FONT_SIZE)
    font_note = ImageFont.load_default(size=TIME_LABEL_NOTE_FONT_SIZE)
    font_bpm = ImageFont.load_default(size=BPM_FONT_SIZE)
    font_sv = ImageFont.load_default(size=SV_TEXT_FONT_SIZE)
    sv_changes = build_sv_changes(timing_points, chart_end_time, mapper)
    image = Image.new("RGB", (layout.image_width, layout.image_height), IMAGE_BACKGROUND[:3])
    draw = ImageDraw.Draw(image)

    for row_index in range(layout.row_count):
        _draw_row_background(image, skin, layout, row_index, render_cache)

    for timing_line in reversed(timing_lines):
        _draw_timing_line(image, draw, skin, timing_line, layout, font_regular, font_note, font_bpm)

    _draw_sv_indicators(draw, sv_changes, layout, font_sv, reverse_order=True)

    for hit_object in reversed(hit_objects):
        _draw_hit_object(image, hit_object, mapper, skin, layout, render_cache)

    image.save(output_path, optimize=True)
    return output_path


def _build_layout(
    skin: TaikoSkin,
    beatmap_duration: int,
    chart_width: float,
    redline_sections: list[RedlineSection],
) -> RenderLayout:
    # 先按谱面时长确定基础宽度，再按主 BPM 施加宽度倍率，最后反推总行数。
    base_row_width = _resolve_base_row_width(beatmap_duration)
    bpm_width_multiplier = 1.0 if SPACING_BPM > 0 else _resolve_row_width_bpm_multiplier(redline_sections)
    max_row_width = round(base_row_width * bpm_width_multiplier)
    row_count = max(1, math.ceil((chart_width + 1) / max_row_width))
    # 左侧鼓区域跟随行高等比缩放，右侧为真正的滚动展示区域。
    left_panel_width = round(skin.bar_left.width * ROW_HEIGHT / skin.bar_left.height)
    right_panel_width = ROW_INNER_PADDING_X * 2 + max_row_width
    content_width = left_panel_width + right_panel_width
    image_width = PAGE_MARGIN_X * 2 + content_width
    image_height = PAGE_MARGIN_Y * 2 + row_count * ROW_HEIGHT + row_count * ROW_GAP
    normal_note_diameter = round(ROW_HEIGHT * NORMAL_NOTE_SIZE_RATIO)
    big_note_diameter = round(normal_note_diameter * BIG_NOTE_SCALE)
    return RenderLayout(
        row_count=row_count,
        max_row_width=max_row_width,
        left_panel_width=left_panel_width,
        right_panel_width=right_panel_width,
        content_width=content_width,
        image_width=image_width,
        image_height=image_height,
        normal_note_diameter=normal_note_diameter,
        big_note_diameter=big_note_diameter,
    )


def _resolve_base_row_width(beatmap_duration: int) -> int:
    # 长谱给更宽的单行，尽量减少换行次数。
    if beatmap_duration < 1 * 60 * 1000:
        return BASE_ROW_WIDTH_0_TO_1_MIN
    if beatmap_duration < 2 * 60 * 1000:
        return BASE_ROW_WIDTH_1_TO_2_MIN
    if beatmap_duration < 3 * 60 * 1000:
        return BASE_ROW_WIDTH_2_TO_3_MIN
    if beatmap_duration < 4 * 60 * 1000:
        return BASE_ROW_WIDTH_3_TO_4_MIN
    if beatmap_duration < 5 * 60 * 1000:
        return BASE_ROW_WIDTH_4_TO_5_MIN
    if beatmap_duration < 6 * 60 * 1000:
        return BASE_ROW_WIDTH_5_TO_6_MIN
    return BASE_ROW_WIDTH_6_TO_10_MIN


def _resolve_row_width_bpm_multiplier(redline_sections: list[RedlineSection]) -> float:
    main_bpm = _resolve_main_bpm(redline_sections)
    if main_bpm < 180:
        return ROW_WIDTH_BPM_0_TO_180
    if main_bpm < 240:
        return ROW_WIDTH_BPM_180_TO_240
    if main_bpm < 300:
        return ROW_WIDTH_BPM_240_TO_300
    return ROW_WIDTH_BPM_300_PLUS


def _resolve_main_bpm(redline_sections: list[RedlineSection]) -> float:
    # 用红线 section 的持续时长加权，取"主 BPM"作为宽度倍率依据。
    # 这样比直接取最高 BPM 更稳，不会因为短暂变速把整图拉得过宽。
    weighted_duration_by_bpm: dict[int, int] = {}
    for section in redline_sections:
        bpm = round(60_000 / section.beat_length)
        duration = max(0, section.end_time - section.start_time)
        weighted_duration_by_bpm[bpm] = weighted_duration_by_bpm.get(bpm, 0) + duration

    if not weighted_duration_by_bpm:
        return 120.0
    return float(max(weighted_duration_by_bpm, key=weighted_duration_by_bpm.get))


def _draw_row_background(
    image: Image.Image,
    skin: TaikoSkin,
    layout: RenderLayout,
    row_index: int,
    cache: dict,
) -> None:
    row_top = _row_top(row_index)
    row_has_drum = DRAW_DRUM_EACH_ROW or row_index == 0

    if row_has_drum:
        left_key = (id(skin.bar_left), layout.left_panel_width, ROW_HEIGHT)
        left = cache.get(left_key)
        if left is None:
            left = _resize_sprite(skin.bar_left, (layout.left_panel_width, ROW_HEIGHT))
            cache[left_key] = left
        right_key = (id(skin.bar_right), layout.right_panel_width, ROW_HEIGHT)
        right = cache.get(right_key)
        if right is None:
            right = _resize_sprite(skin.bar_right, (layout.right_panel_width, ROW_HEIGHT))
            cache[right_key] = right
        image.paste(left, (PAGE_MARGIN_X, row_top), left)
        image.paste(right, (PAGE_MARGIN_X + layout.left_panel_width, row_top), right)
        return

    full_key = (id(skin.bar_right), layout.content_width, ROW_HEIGHT)
    full_row = cache.get(full_key)
    if full_row is None:
        full_row = _resize_sprite(skin.bar_right, (layout.content_width, ROW_HEIGHT))
        cache[full_key] = full_row
    image.paste(full_row, (PAGE_MARGIN_X, row_top), full_row)


def _draw_timing_line(
    image: Image.Image,
    draw: ImageDraw.ImageDraw,
    skin: TaikoSkin,
    timing_line: TimingLine,
    layout: RenderLayout,
    font: ImageFont.ImageFont,
    note_font: ImageFont.ImageFont,
    bpm_font: ImageFont.ImageFont,
) -> None:
    # position 已经是整张谱面的绝对横向位置，这里只负责把它映射到具体行。
    row_index = min(layout.row_count - 1, int(timing_line.position // layout.max_row_width))
    local_position = timing_line.position - row_index * layout.max_row_width
    line_x = round(_row_chart_left(layout, row_index) + local_position)
    line_y0 = _row_top(row_index)
    line_y1 = line_y0 + ROW_HEIGHT

    if timing_line.is_measure:
        # 小节线直接使用皮肤里的 barline 贴图，更接近游戏观感。
        line_width = max(1, round(skin.bar_line.width * ROW_HEIGHT / skin.bar_line.height))
        line_sprite = _resize_sprite(skin.bar_line, (line_width, ROW_HEIGHT))
        image.paste(line_sprite, (round(line_x - line_sprite.width / 2), line_y0), line_sprite)
    else:
        draw.line((line_x, line_y0, line_x, line_y1), fill=BEAT_LINE_COLOR, width=1)

    if timing_line.show_label:
        _draw_time_label(draw, timing_line, line_x, line_y0, layout, font, note_font, bpm_font)


def _draw_time_label(
    draw: ImageDraw.ImageDraw,
    timing_line: TimingLine,
    line_x: int,
    row_top: int,
    layout: RenderLayout,
    font: ImageFont.ImageFont,
    note_font: ImageFont.ImageFont,
    bpm_font: ImageFont.ImageFont,
) -> None:
    label = f"{timing_line.time / 1000:.1f}s"
    note = _build_time_label_note(timing_line)
    label_color = ACCENT_LABEL_COLOR if timing_line.is_kiai else RULER_TEXT_COLOR
    label_box = draw.textbbox((0, 0), label, font=font)
    label_width = label_box[2] - label_box[0]
    label_height = label_box[3] - label_box[1]
    label_x = min(
        round(line_x - label_width / 2),
        PAGE_MARGIN_X + layout.content_width - label_width - LABEL_RIGHT_PADDING,
    )
    label_x = max(PAGE_MARGIN_X, label_x)
    label_y = row_top + ROW_HEIGHT + TIME_LABEL_TOP_GAP

    # 时间标签固定绘制在该行轨道的下方，避免盖住 note 与小节线主体。
    draw.text((label_x, label_y), label, fill=label_color, font=font)

    next_y = label_y + label_height
    if note is not None:
        note_box = draw.textbbox((0, 0), note, font=note_font)
        note_width = note_box[2] - note_box[0]
        note_x = min(
            round(line_x - note_width / 2),
            PAGE_MARGIN_X + layout.content_width - note_width - LABEL_RIGHT_PADDING,
        )
        note_x = max(PAGE_MARGIN_X, note_x)
        note_y = next_y + TIME_LABEL_NOTE_TOP_GAP
        draw.text((note_x, note_y), note, fill=ACCENT_LABEL_COLOR, font=note_font)
        note_box_full = draw.textbbox((0, 0), note, font=note_font)
        next_y = note_y + (note_box_full[3] - note_box_full[1])

    if timing_line.bpm is not None:
        bpm_label = f"{timing_line.bpm:.0f}BPM"
        bpm_box = draw.textbbox((0, 0), bpm_label, font=bpm_font)
        bpm_width = bpm_box[2] - bpm_box[0]
        bpm_x = min(
            round(line_x - bpm_width / 2),
            PAGE_MARGIN_X + layout.content_width - bpm_width - LABEL_RIGHT_PADDING,
        )
        bpm_x = max(PAGE_MARGIN_X, bpm_x)
        bpm_y = next_y + BPM_TOP_GAP
        bpm_color = ACCENT_LABEL_COLOR if timing_line.is_kiai else RULER_TEXT_COLOR
        draw.text((bpm_x, bpm_y), bpm_label, fill=bpm_color, font=bpm_font)


def _build_time_label_note(timing_line: TimingLine) -> str | None:
    if not timing_line.is_kiai_start:
        return None
    return "Kiai Start"


def _draw_hit_object(
    image: Image.Image,
    hit_object: TaikoHitObject,
    mapper: ScrollPositionMapper,
    skin: TaikoSkin,
    layout: RenderLayout,
    cache: dict,
) -> None:
    if hit_object.hit_type & SWELL_FLAG:
        _draw_span_object(
            image,
            hit_object,
            mapper,
            skin,
            layout,
            cache,
            is_swell=True,
            span_color=SWELL_COLOR,
            draw_spinner_warning=True,
        )
        return
    if hit_object.hit_type & DRUMROLL_FLAG:
        is_big_roll = bool(hit_object.hitsound & HIT_SOUNDS_STRONG)
        # 大鼓滑条使用 swell 的外观；普通小鼓滑条保持 roll 外观不变。
        _draw_span_object(
            image,
            hit_object,
            mapper,
            skin,
            layout,
            cache,
            is_swell=is_big_roll,
            span_color=ROLL_COLOR,
            draw_spinner_warning=False,
        )
        return
    _draw_circle_object(image, hit_object, mapper, skin, layout, cache)


def _draw_circle_object(
    image: Image.Image,
    hit_object: TaikoHitObject,
    mapper: ScrollPositionMapper,
    skin: TaikoSkin,
    layout: RenderLayout,
    cache: dict,
) -> None:
    absolute_position = mapper.position_at(hit_object.start_time)
    # 物件绝对位置先换算出所在行，再换算成该行内部的局部 x。
    row_index = min(layout.row_count - 1, int(absolute_position // layout.max_row_width))
    local_position = absolute_position - row_index * layout.max_row_width
    center_x = round(_row_chart_left(layout, row_index) + local_position)
    center_y = _row_center_y(row_index)
    is_strong = bool(hit_object.hitsound & HIT_SOUNDS_STRONG)
    is_rim = bool(hit_object.hitsound & HIT_SOUNDS_RIM)
    diameter = layout.big_note_diameter if is_strong else layout.normal_note_diameter
    base_sprite = skin.big_hit_circle if is_strong else skin.hit_circle
    overlay_sprite = skin.big_hit_circle_overlay if is_strong else skin.hit_circle_overlay
    color = RIM_NOTE_COLOR if is_rim else CENTRE_NOTE_COLOR

    _draw_note_sprite(image, base_sprite, overlay_sprite, color, diameter, center_x, center_y, cache)


def _draw_span_object(
    image: Image.Image,
    hit_object: TaikoHitObject,
    mapper: ScrollPositionMapper,
    skin: TaikoSkin,
    layout: RenderLayout,
    cache: dict,
    is_swell: bool,
    span_color: tuple[int, int, int],
    draw_spinner_warning: bool,
) -> None:
    absolute_start = mapper.position_at(hit_object.start_time)
    absolute_end = max(absolute_start, mapper.position_at(hit_object.end_time))
    # 长条可能跨多行，所以需要把一个 span 切成多个行内 segment 分别绘制。
    row_start = int(absolute_start // layout.max_row_width)
    row_end = int(absolute_end // layout.max_row_width)
    head_diameter = layout.big_note_diameter if is_swell else layout.normal_note_diameter
    body_height = round(head_diameter * (SWELL_BODY_HEIGHT_RATIO if is_swell else SPAN_BODY_HEIGHT_RATIO))

    for row_index in range(row_start, row_end + 1):
        segment_start = max(absolute_start, row_index * layout.max_row_width)
        segment_end = min(absolute_end, (row_index + 1) * layout.max_row_width)
        start_x = round(_row_chart_left(layout, row_index) + (segment_start - row_index * layout.max_row_width))
        end_x = round(_row_chart_left(layout, row_index) + (segment_end - row_index * layout.max_row_width))
        _draw_roll_body(image, skin, span_color, start_x, end_x, _row_center_y(row_index), body_height)

    head_center_x = round(_row_chart_left(layout, row_start) + (absolute_start - row_start * layout.max_row_width))
    tail_join_x = round(_row_chart_left(layout, row_end) + (absolute_end - row_end * layout.max_row_width))
    _draw_span_head(
        image,
        skin,
        span_color,
        head_center_x,
        _row_center_y(row_start),
        head_diameter,
        cache,
        is_swell,
        draw_spinner_warning,
    )
    _draw_span_tail(image, span_color, tail_join_x, _row_center_y(row_end), body_height, cache)


def _draw_roll_body(
    image: Image.Image,
    skin: TaikoSkin,
    color: tuple[int, int, int],
    start_x: int,
    end_x: int,
    center_y: int,
    height: int,
) -> None:
    if end_x <= start_x:
        return

    # roll-middle 只有 2px 宽，本质就是沿 x 方向拉伸出身体。
    body = _tint_sprite(skin.roll_middle, color, (max(1, end_x - start_x), height))
    image.paste(body, (start_x, round(center_y - height / 2)), body)


def _draw_span_head(
    image: Image.Image,
    skin: TaikoSkin,
    color: tuple[int, int, int],
    center_x: int,
    center_y: int,
    diameter: int,
    cache: dict,
    is_swell: bool,
    draw_spinner_warning: bool,
) -> None:
    base_sprite = skin.big_hit_circle if is_swell else skin.hit_circle
    overlay_sprite = skin.big_hit_circle_overlay if is_swell else skin.hit_circle_overlay
    _draw_note_sprite(image, base_sprite, overlay_sprite, color, diameter, center_x, center_y, cache)
    if draw_spinner_warning:
        _draw_spinner_warning(image, skin, diameter, center_x, center_y, cache)


def _draw_spinner_warning(
    image: Image.Image,
    skin: TaikoSkin,
    diameter: int,
    center_x: int,
    center_y: int,
    cache: dict,
) -> None:
    warning_key = (id(skin.spinner_warning), diameter)
    warning = cache.get(warning_key)
    if warning is None:
        warning = _resize_sprite(skin.spinner_warning, (diameter, diameter))
        cache[warning_key] = warning
    position = (round(center_x - diameter / 2), round(center_y - diameter / 2))
    image.paste(warning, position, warning)


def _draw_span_tail(
    image: Image.Image,
    color: tuple[int, int, int],
    join_x: int,
    center_y: int,
    height: int,
    cache: dict,
) -> None:
    tail = _build_roll_tail_sprite(color, height, cache)
    image.paste(tail, (join_x, round(center_y - height / 2)), tail)


def _build_roll_tail_sprite(color: tuple[int, int, int], height: int, cache: dict) -> Image.Image:
    key = (color, height)
    cached = cache.get(key)
    if cached is not None:
        return cached
    scale = 4
    scaled_height = height * scale
    scaled_width = max(1, math.ceil(height / 2) * scale)
    radius = scaled_height / 2
    border_width = max(1, round(height * 0.05)) * scale

    tail = Image.new("RGBA", (scaled_width, scaled_height), (0, 0, 0, 0))
    draw = ImageDraw.Draw(tail)
    draw.ellipse((-radius, 0, radius, scaled_height), fill=(0, 0, 0, 255))
    draw.ellipse(
        (
            -radius + border_width,
            border_width,
            radius - border_width,
            scaled_height - border_width,
        ),
        fill=(*color, 255),
    )
    tail = tail.resize((scaled_width // scale, height), Image.Resampling.LANCZOS)
    cache[key] = tail
    return tail


def _draw_note_sprite(
    image: Image.Image,
    base_sprite: Image.Image,
    overlay_sprite: Image.Image,
    color: tuple[int, int, int],
    diameter: int,
    center_x: int,
    center_y: int,
    cache: dict,
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

    position = (round(center_x - diameter / 2), round(center_y - diameter / 2))
    image.paste(tinted_base, position, tinted_base)
    image.paste(overlay, position, overlay)


def _row_top(row_index: int) -> int:
    return PAGE_MARGIN_Y + row_index * (ROW_HEIGHT + ROW_GAP)


def _row_center_y(row_index: int) -> int:
    return _row_top(row_index) + ROW_HEIGHT // 2


def _row_chart_left(layout: RenderLayout, row_index: int) -> int:
    row_left = PAGE_MARGIN_X
    if DRAW_DRUM_EACH_ROW or row_index == 0:
        row_left += layout.left_panel_width
    return row_left + ROW_INNER_PADDING_X


def _resize_sprite(sprite: Image.Image, size: tuple[int, int]) -> Image.Image:
    return sprite.resize(size, Image.Resampling.LANCZOS)


def _tint_sprite(
    sprite: Image.Image,
    color: tuple[int, int, int],
    size: int | tuple[int, int],
) -> Image.Image:
    if isinstance(size, int):
        target_size = (size, size)
    else:
        target_size = size

    resized = _resize_sprite(sprite, target_size)
    # osu! 对 legacy taiko circle 的 AccentColour 是颜色相乘：
    # 白色圆面变成 note 颜色，黑色表情仍保持黑色，overlay 白边再盖在最上层。
    red, green, blue, alpha = resized.split()
    color_red = red.point(lambda value: round(value * color[0] / 255))
    color_green = green.point(lambda value: round(value * color[1] / 255))
    color_blue = blue.point(lambda value: round(value * color[2] / 255))
    return Image.merge("RGBA", (color_red, color_green, color_blue, alpha))


def _draw_sv_indicators(
    draw: ImageDraw.ImageDraw,
    sv_changes: list[SvChange],
    layout: RenderLayout,
    font: ImageFont.ImageFont,
    reverse_order: bool = False,
) -> None:
    items = reversed(sv_changes) if reverse_order else sv_changes
    for sv_change in items:
        row_index = min(layout.row_count - 1, int(sv_change.position // layout.max_row_width))
        local_position = sv_change.position - row_index * layout.max_row_width
        x = round(_row_chart_left(layout, row_index) + local_position)
        row_top = _row_top(row_index)

        label = _format_sv_label(sv_change.sv)
        label_box = draw.textbbox((0, 0), label, font=font)
        label_width = label_box[2] - label_box[0]
        label_height = label_box[3] - label_box[1]

        label_x = round(x - label_width / 2)
        label_y = max(PAGE_MARGIN_Y, row_top - SV_TOP_GAP - label_height)
        draw.text((label_x, label_y), label, fill=SV_TEXT_COLOR, font=font)


def _format_sv_label(sv: float) -> str:
    if sv == round(sv, 1):
        return f"{sv:.1f}x"
    return f"{sv:.2f}x"


def _apply_taiko_object_mods(
    hit_objects: list[TaikoHitObject],
    mods: ModSettings | None,
) -> list[TaikoHitObject]:
    if mods is None or not mods.swap:
        return hit_objects

    # SW 只交换普通 hit 的红/蓝；roll 和 swell 在游戏内不是 Hit，不参与交换。
    swapped: list[TaikoHitObject] = []
    for hit_object in hit_objects:
        if hit_object.hit_type & (DRUMROLL_FLAG | SWELL_FLAG):
            swapped.append(hit_object)
            continue

        hitsound = hit_object.hitsound
        is_rim = bool(hitsound & HIT_SOUNDS_RIM)
        if is_rim:
            hitsound &= ~HIT_SOUNDS_RIM
            if hitsound == 0:
                hitsound = 0
        else:
            hitsound |= 8

        swapped.append(
            TaikoHitObject(
                start_time=hit_object.start_time,
                end_time=hit_object.end_time,
                hit_type=hit_object.hit_type,
                hitsound=hitsound,
            )
        )
    return swapped


def _effective_slider_multiplier(beatmap: Beatmap, mods: ModSettings | None) -> float:
    slider_multiplier = float(beatmap.difficulty["SliderMultiplier"])
    if mods is None:
        return slider_multiplier
    # Taiko EZ/HR 会额外改 SliderMultiplier，视觉上就是整张图滚速变慢/变快。
    if mods.easy:
        slider_multiplier *= 0.8
    if mods.hard_rock:
        slider_multiplier *= 1.4 * 4 / 3
    return slider_multiplier


def _effective_timing_points(beatmap: Beatmap, mods: ModSettings | None):
    if mods is not None and mods.cs_override:
        # Constant Speed 在 drawable ruleset 层禁用 SV 可视化，这里用只保留红线来等价呈现。
        return [point for point in beatmap.timing_points if point.uninherited]
    return beatmap.timing_points
