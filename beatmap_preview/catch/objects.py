from __future__ import annotations

import struct
from dataclasses import dataclass, replace

from ..errors import PreviewError
from ..models import Beatmap, CatchHitObject, TimingPoint
from ..mods import ModSettings
from .config import (
    BANANA_COLORS,
    BANANA_SCALE,
    CATCHER_BASE_SIZE,
    DEFAULT_BEAT_LENGTH,
    DEFAULT_METER,
    DROPLET_SCALE,
    PLAYFIELD_WIDTH,
    RNG_SEED,
    TINY_DROPLET_SCALE,
)
from .slider_path import build_slider_path, path_position_at


@dataclass(frozen=True)
class CatchRenderObject:
    object_type: str
    x: float
    start_time: int
    color: tuple[int, int, int]
    index_in_beatmap: int
    sprite_name: str
    scale_factor: float
    rotation: float
    event_time: float | None = None
    hyper_dash: bool = False


@dataclass(frozen=True)
class SliderEvent:
    event_type: str
    time: float
    path_progress: float
    span_index: int
    span_start_time: float


class LegacyRandom:
    """复刻 osu! 的 FastRandom，用于 tiny droplet / 香蕉随机偏移。"""

    _int_mask = 0x7FFFFFFF

    def __init__(self, seed: int):
        self.x = seed & 0xFFFFFFFF
        self.y = 842502087
        self.z = 3579807591
        self.w = 273326509
        self._bit_buffer = 0
        self._bit_index = 32

    def next_uint(self) -> int:
        t = self.x ^ ((self.x << 11) & 0xFFFFFFFF)
        self.x = self.y
        self.y = self.z
        self.z = self.w
        self.w = (self.w ^ (self.w >> 19) ^ t ^ (t >> 8)) & 0xFFFFFFFF
        return self.w

    def next(self, lower_bound: int | None = None, upper_bound: int | None = None) -> int:
        if lower_bound is None and upper_bound is None:
            return self.next_uint() & self._int_mask
        if upper_bound is None:
            return int(self.next_double() * lower_bound)
        return int(lower_bound + self.next_double() * (upper_bound - lower_bound))

    def next_double(self) -> float:
        return (self.next_uint() & self._int_mask) / (2**31)

    def next_bool(self) -> bool:
        if self._bit_index == 32:
            self._bit_buffer = self.next_uint()
            self._bit_index = 1
            return (self._bit_buffer & 1) == 1

        self._bit_index += 1
        self._bit_buffer >>= 1
        return (self._bit_buffer & 1) == 1


def build_catch_render_objects(
    beatmap: Beatmap,
    hit_objects: list[CatchHitObject],
    combo_colors: list[tuple[int, int, int]],
    mods: ModSettings | None = None,
    difficulty: dict[str, float] | None = None,
) -> list[CatchRenderObject]:
    """把顶层 catch 物件展开成真正落下的 fruit / droplet / banana 列表。"""
    effective_difficulty = difficulty or _difficulty_from_beatmap(beatmap)
    slider_tick_rate = effective_difficulty["SliderTickRate"]
    slider_multiplier = effective_difficulty["SliderMultiplier"]
    beatmap_format_version = int(beatmap.general.get("FormatVersion", "14"))
    render_objects: list[CatchRenderObject] = []
    rng = LegacyRandom(RNG_SEED)
    hard_rock_offsets = mods is not None and mods.hard_rock
    last_position: float | None = None
    last_start_time = 0.0

    for index, hit_object in enumerate(hit_objects):
        combo_color = _color_for_index(index, combo_colors)
        if _is_spinner(hit_object):
            render_objects.extend(_build_banana_shower_objects(hit_object, index, rng))
            continue
        if _is_slider(hit_object):
            # HR 位移只作用于顶层 Fruit；遇到 JuiceStream 时，stable/lazer 只把“上一个位置”
            # 更新为滑条最后一个控制点，并继续消费 nested droplet 的随机数。
            last_position = _stable_slider_end_x(hit_object)
            last_start_time = float(hit_object.start_time)
            render_objects.extend(
                _build_juice_stream_objects(
                    hit_object=hit_object,
                    index_in_beatmap=index,
                    combo_color=combo_color,
                    slider_tick_rate=slider_tick_rate,
                    slider_multiplier=slider_multiplier,
                    beatmap_format_version=beatmap_format_version,
                    timing_points=beatmap.timing_points,
                    rng=rng,
                )
            )
            continue
        fruit = _build_fruit_object(hit_object.x, hit_object.start_time, index, combo_color)
        if hard_rock_offsets:
            fruit, last_position, last_start_time = _apply_hard_rock_fruit_offset(
                fruit,
                last_position,
                last_start_time,
                rng,
            )
        render_objects.append(fruit)

    return _apply_hyper_dash(render_objects, effective_difficulty["CircleSize"])


def _color_for_index(
    index_in_beatmap: int,
    combo_colors: list[tuple[int, int, int]],
) -> tuple[int, int, int]:
    # catch 里 palpable object 的颜色取自 IndexInBeatmap + 1，而不是 combo 序号。
    return combo_colors[index_in_beatmap % len(combo_colors)]


def _build_banana_shower_objects(
    hit_object: CatchHitObject,
    index_in_beatmap: int,
    rng: LegacyRandom,
):
    start_time = int(hit_object.start_time)
    end_time = int(hit_object.end_time)
    spacing = _to_float32(float(hit_object.end_time - hit_object.start_time))

    while spacing > 100:
        spacing = _to_float32(spacing / 2)
    if spacing <= 0:
        return

    current_time = _to_float32(float(start_time))
    while current_time <= end_time:
        x = rng.next_double() * PLAYFIELD_WIDTH
        rng.next()
        rng.next()
        rng.next()
        yield CatchRenderObject(
            object_type="banana",
            x=x,
            start_time=int(round(current_time)),
            color=_banana_color(int(current_time)),
            index_in_beatmap=index_in_beatmap,
            sprite_name="banana",
            scale_factor=BANANA_SCALE,
            rotation=_banana_rotation(int(current_time)),
            event_time=current_time,
        )
        current_time = _to_float32(current_time + spacing)


def _build_juice_stream_objects(
    hit_object: CatchHitObject,
    index_in_beatmap: int,
    combo_color: tuple[int, int, int],
    slider_tick_rate: float,
    slider_multiplier: float,
    beatmap_format_version: int,
    timing_points: list[TimingPoint],
    rng: LegacyRandom,
):
    if hit_object.slider_type is None:
        raise PreviewError("catch slider is missing path type")

    path = build_slider_path(hit_object)
    events = _build_slider_events(hit_object, slider_tick_rate, slider_multiplier, beatmap_format_version, timing_points)
    nested_objects: list[CatchRenderObject] = []
    previous_event: SliderEvent | None = None

    for event in events:
        if previous_event is not None:
            nested_objects.extend(
                _build_tiny_droplets_between(
                    path=path,
                    previous_event=previous_event,
                    current_event=event,
                    index_in_beatmap=index_in_beatmap,
                    combo_color=combo_color,
                )
            )

        x = path_position_at(path, event.path_progress)[0]
        if event.event_type == "tick":
            nested_objects.append(
                CatchRenderObject(
                    object_type="droplet",
                    x=x,
                    start_time=int(round(event.time)),
                    color=combo_color,
                    index_in_beatmap=index_in_beatmap,
                    sprite_name="drop",
                    scale_factor=DROPLET_SCALE,
                    rotation=_droplet_rotation(int(round(event.time))),
                    event_time=event.time,
                )
            )
        elif event.event_type != "legacy_last_tick":
            nested_objects.append(_build_fruit_object(x, int(round(event.time)), index_in_beatmap, combo_color, event.time))

        previous_event = event

    yield from _apply_stream_offsets(nested_objects, rng)


def _build_slider_events(
    hit_object: CatchHitObject,
    slider_tick_rate: float,
    slider_multiplier: float,
    beatmap_format_version: int,
    timing_points: list[TimingPoint],
) -> list[SliderEvent]:
    beat_length, slider_velocity = _resolve_slider_timing(hit_object.start_time, timing_points)
    if slider_tick_rate <= 0:
        raise PreviewError("SliderTickRate must be positive")

    span_count = max(1, hit_object.slider_repeats)
    velocity = 100 * slider_multiplier / _precision_adjusted_beat_length(beat_length, slider_velocity)
    if hit_object.slider_pixel_length <= 0 or velocity <= 0:
        return [
            SliderEvent(
                event_type="head",
                time=hit_object.start_time,
                path_progress=0.0,
                span_index=0,
                span_start_time=float(hit_object.start_time),
            ),
            SliderEvent(
                event_type="tail",
                time=hit_object.end_time,
                path_progress=1.0 if span_count % 2 == 1 else 0.0,
                span_index=span_count - 1,
                span_start_time=float(hit_object.end_time),
            ),
        ]

    span_duration = hit_object.slider_pixel_length / velocity
    scoring_distance = velocity * beat_length
    if beatmap_format_version < 8:
        scoring_distance /= slider_velocity
    total_distance = min(100000.0, hit_object.slider_pixel_length)
    tick_distance = min(max(0.0, scoring_distance / slider_tick_rate), total_distance)
    min_distance_from_end = velocity * 10

    events = [
        SliderEvent(
            event_type="head",
            time=hit_object.start_time,
            path_progress=0.0,
            span_index=0,
            span_start_time=float(hit_object.start_time),
        )
    ]

    for span_index in range(span_count):
        span_start_time = hit_object.start_time + span_index * span_duration
        reversed_span = span_index % 2 == 1
        events.extend(
            _generate_span_ticks(
                span_index=span_index,
                span_start_time=span_start_time,
                span_duration=span_duration,
                reversed_span=reversed_span,
                total_distance=total_distance,
                tick_distance=tick_distance,
                min_distance_from_end=min_distance_from_end,
            )
        )

        is_last_span = span_index == span_count - 1
        events.append(
            SliderEvent(
                event_type="tail" if is_last_span else "repeat",
                time=span_start_time + span_duration,
                path_progress=1.0 if span_index % 2 == 0 else 0.0,
                span_index=span_index,
                span_start_time=span_start_time,
            )
        )

    legacy_last_tick = _build_legacy_last_tick(hit_object.start_time, span_duration, span_count)
    if legacy_last_tick is not None:
        # LegacyLastTick 只参与 tiny droplet 分段，不单独绘制。
        events.insert(len(events) - 1, legacy_last_tick)

    return events


def _generate_span_ticks(
    span_index: int,
    span_start_time: float,
    span_duration: float,
    reversed_span: bool,
    total_distance: float,
    tick_distance: float,
    min_distance_from_end: float,
):
    if tick_distance <= 0:
        return

    ticks: list[SliderEvent] = []
    distance = tick_distance

    while distance <= total_distance + 0.001:
        if distance >= total_distance - min_distance_from_end:
            break

        path_progress = distance / total_distance
        time_progress = 1.0 - path_progress if reversed_span else path_progress
        ticks.append(
            SliderEvent(
                event_type="tick",
                time=span_start_time + time_progress * span_duration,
                path_progress=path_progress,
                span_index=span_index,
                span_start_time=span_start_time,
            )
        )
        distance += tick_distance

    if reversed_span:
        ticks.reverse()

    yield from ticks


def _build_legacy_last_tick(start_time: int, span_duration: float, span_count: int) -> SliderEvent | None:
    if span_count <= 0:
        return None

    total_duration = span_count * span_duration
    final_span_index = span_count - 1
    final_span_start_time = start_time + final_span_index * span_duration
    legacy_last_tick_time = max(
        start_time + total_duration / 2,
        final_span_start_time + span_duration - 36,
    )
    path_progress = (legacy_last_tick_time - final_span_start_time) / span_duration
    if span_count % 2 == 0:
        path_progress = 1.0 - path_progress

    return SliderEvent(
        event_type="legacy_last_tick",
        time=legacy_last_tick_time,
        path_progress=path_progress,
        span_index=final_span_index,
        span_start_time=final_span_start_time,
    )


def _build_tiny_droplets_between(
    path: list[tuple[float, float]],
    previous_event: SliderEvent,
    current_event: SliderEvent,
    index_in_beatmap: int,
    combo_color: tuple[int, int, int],
):
    since_last_event = int(current_event.time) - int(previous_event.time)
    if since_last_event <= 80:
        return

    time_between_tiny = float(since_last_event)
    while time_between_tiny > 100:
        time_between_tiny /= 2

    offset = time_between_tiny
    while offset < since_last_event - 0.001:
        ratio = offset / since_last_event
        progress = previous_event.path_progress + ratio * (current_event.path_progress - previous_event.path_progress)
        x = path_position_at(path, progress)[0]
        yield CatchRenderObject(
            object_type="tiny_droplet",
            x=x,
            start_time=int(round(previous_event.time + offset)),
            color=combo_color,
            index_in_beatmap=index_in_beatmap,
            sprite_name="drop",
            scale_factor=TINY_DROPLET_SCALE,
            rotation=_droplet_rotation(int(round(previous_event.time + offset))),
            event_time=previous_event.time + offset,
        )
        offset += time_between_tiny


def _apply_stream_offsets(
    nested_objects: list[CatchRenderObject],
    rng: LegacyRandom,
):
    for catch_object in nested_objects:
        if catch_object.object_type == "tiny_droplet":
            offset = max(-catch_object.x, min(rng.next(-20, 20), PLAYFIELD_WIDTH - catch_object.x))
            yield replace(catch_object, x=catch_object.x + offset)
            continue
        if catch_object.object_type == "droplet":
            rng.next()
        yield catch_object


def _stable_slider_end_x(hit_object: CatchHitObject) -> float:
    if hit_object.slider_points:
        return float(hit_object.slider_points[-1][0])
    return float(hit_object.x)


def _apply_hard_rock_fruit_offset(
    catch_object: CatchRenderObject,
    last_position: float | None,
    last_start_time: float,
    rng: LegacyRandom,
) -> tuple[CatchRenderObject, float | None, float]:
    offset_position = float(catch_object.x)
    start_time = float(catch_object.start_time)

    if last_position is None or last_position == 0:
        return catch_object, offset_position, start_time

    position_diff = offset_position - last_position
    # stable 使用 int 时间差；这里保留截断行为，否则随机偏移会错位。
    time_diff = int(start_time - last_start_time)

    if time_diff > 1000:
        return catch_object, offset_position, start_time

    if position_diff == 0:
        offset_position = _apply_random_offset(offset_position, time_diff / 4.0, rng)
        return replace(catch_object, x=offset_position), last_position, last_start_time

    if abs(position_diff) < time_diff / 3:
        offset_position = _apply_offset(offset_position, position_diff)

    return replace(catch_object, x=offset_position), offset_position, start_time


def _apply_random_offset(position: float, max_offset: float, rng: LegacyRandom) -> float:
    right = rng.next_bool()
    random_offset = min(20.0, float(rng.next(0, max(0.0, max_offset))))

    if right:
        if position + random_offset <= PLAYFIELD_WIDTH:
            return position + random_offset
        return position - random_offset

    if position - random_offset >= 0:
        return position - random_offset
    return position + random_offset


def _apply_offset(position: float, amount: float) -> float:
    if amount > 0:
        if position + amount < PLAYFIELD_WIDTH:
            return position + amount
        return position

    if position + amount > 0:
        return position + amount
    return position


def _apply_hyper_dash(
    render_objects: list[CatchRenderObject],
    circle_size: float,
) -> list[CatchRenderObject]:
    render_objects = [_clamp_effective_x(catch_object) for catch_object in render_objects]
    palpable_indexes = [
        index
        for index, catch_object in enumerate(render_objects)
        if catch_object.object_type in {"fruit", "droplet"}
    ]
    if len(palpable_indexes) < 2:
        return render_objects

    scale = _scale_from_circle_size(circle_size)
    half_catcher_width = CATCHER_BASE_SIZE * scale
    last_direction = 0
    last_excess = half_catcher_width
    updated_objects = render_objects[:]

    for sequence_index in range(len(palpable_indexes) - 1):
        current_index = palpable_indexes[sequence_index]
        next_index = palpable_indexes[sequence_index + 1]
        current_object = updated_objects[current_index]
        next_object = updated_objects[next_index]
        direction = 1 if next_object.x > current_object.x else -1
        time_to_next = int(_event_time(next_object)) - int(_event_time(current_object)) - 1000 / 60 / 4
        distance_to_next = abs(next_object.x - current_object.x) - (
            last_excess if last_direction == direction else half_catcher_width
        )
        distance_to_hyper = time_to_next - distance_to_next

        if distance_to_hyper < 0:
            updated_objects[current_index] = replace(current_object, hyper_dash=True)
            last_excess = half_catcher_width
        else:
            last_excess = max(0.0, min(distance_to_hyper, half_catcher_width))

        last_direction = direction

    return updated_objects




def _clamp_effective_x(catch_object: CatchRenderObject) -> CatchRenderObject:
    clamped_x = max(0.0, min(float(catch_object.x), PLAYFIELD_WIDTH))
    if clamped_x == catch_object.x:
        return catch_object
    return replace(catch_object, x=clamped_x)


def _difficulty_from_beatmap(beatmap: Beatmap) -> dict[str, float]:
    return {
        "CircleSize": float(beatmap.difficulty.get("CircleSize", "5")),
        "OverallDifficulty": float(beatmap.difficulty.get("OverallDifficulty", "5")),
        "ApproachRate": float(beatmap.difficulty.get("ApproachRate", beatmap.difficulty.get("OverallDifficulty", "5"))),
        "HPDrainRate": float(beatmap.difficulty.get("HPDrainRate", "5")),
        "SliderMultiplier": float(beatmap.difficulty.get("SliderMultiplier", "1.4")),
        "SliderTickRate": float(beatmap.difficulty.get("SliderTickRate", "1")),
    }


def _event_time(catch_object: CatchRenderObject) -> float:
    return catch_object.event_time if catch_object.event_time is not None else float(catch_object.start_time)


def _build_fruit_object(
    x: float,
    start_time: int,
    index_in_beatmap: int,
    combo_color: tuple[int, int, int],
    event_time: float | None = None,
) -> CatchRenderObject:
    fruit_name = ("pear", "grapes", "apple", "orange")[index_in_beatmap % 4]
    return CatchRenderObject(
        object_type="fruit",
        x=x,
        start_time=start_time,
        color=combo_color,
        index_in_beatmap=index_in_beatmap,
        sprite_name=fruit_name,
        scale_factor=1.0,
        rotation=_fruit_rotation(start_time),
        event_time=event_time,
    )


def _resolve_slider_timing(start_time: int, timing_points: list[TimingPoint]) -> tuple[float, float]:
    beat_length = DEFAULT_BEAT_LENGTH
    meter = DEFAULT_METER
    slider_velocity = 1.0

    for point in timing_points:
        if point.time > 0:
            break
        beat_length, meter, slider_velocity = _apply_timing_state(point, beat_length, meter, slider_velocity)

    for point in timing_points:
        if point.time > start_time:
            break
        beat_length, meter, slider_velocity = _apply_timing_state(point, beat_length, meter, slider_velocity)

    return beat_length, slider_velocity


def _apply_timing_state(
    point: TimingPoint,
    beat_length: float,
    meter: int,
    slider_velocity: float,
) -> tuple[float, int, float]:
    if point.uninherited:
        return point.beat_length, point.meter, slider_velocity
    if point.beat_length >= 0:
        return beat_length, meter, 1.0
    return beat_length, meter, -100 / point.beat_length


def _precision_adjusted_beat_length(beat_length: float, slider_velocity: float) -> float:
    if slider_velocity <= 0:
        return beat_length
    bpm_multiplier = min(max(_to_float32(100 / slider_velocity), 10), 1000) / 100
    return beat_length * bpm_multiplier


def _to_float32(value: float) -> float:
    return struct.unpack("f", struct.pack("f", value))[0]


def _banana_color(seed: int) -> tuple[int, int, int]:
    return BANANA_COLORS[_stateless_next_int(3, seed)]


def _fruit_rotation(seed: int) -> float:
    return (_stateless_next_single(seed, 1) - 0.5) * 40


def _droplet_rotation(seed: int) -> float:
    return _stateless_next_single(seed, 1) * 360


def _banana_rotation(seed: int) -> float:
    return 180 * (_stateless_next_single(seed, 2) * 2 - 1)


def _stateless_next_int(max_value: int, seed: int, series: int = 0) -> int:
    return _stateless_next_ulong(seed, series) % max_value


def _stateless_next_single(seed: int, series: int = 0) -> float:
    return float(_stateless_next_ulong(seed, series) & ((1 << 24) - 1)) / (1 << 24)


def _stateless_next_ulong(seed: int, series: int = 0) -> int:
    combined = ((series & 0xFFFFFFFF) << 32) | (seed & 0xFFFFFFFF)
    return _stateless_mix(combined ^ 0x12345678)


def _stateless_mix(value: int) -> int:
    value ^= value >> 33
    value = (value * 0xFF51AFD7ED558CCD) & 0xFFFFFFFFFFFFFFFF
    value ^= value >> 33
    value = (value * 0xC4CEB9FE1A85EC53) & 0xFFFFFFFFFFFFFFFF
    value ^= value >> 33
    return value


def _scale_from_circle_size(circle_size: float) -> float:
    return (1.0 - 0.7 * ((circle_size - 5.0) / 5.0)) / 2.0


def _is_slider(hit_object: CatchHitObject) -> bool:
    return bool(hit_object.hit_type & 2)


def _is_spinner(hit_object: CatchHitObject) -> bool:
    return bool(hit_object.hit_type & 8)
