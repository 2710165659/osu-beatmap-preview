"""standard -> taiko 转谱，按 osu!lazer 的 TaikoBeatmapConverter 移植。"""

from __future__ import annotations

import math
import struct

from ..errors import PreviewError
from ..models import Beatmap, StandardHitObject, TaikoHitObject, TimingPoint
from ..mods import ModSettings


# 游戏内转换时隐藏应用的倍率。C# 常量是 1.4f，所以这里也按 float32 保存。
VELOCITY_MULTIPLIER = struct.unpack("<f", struct.pack("<f", 1.4))[0]
OSU_BASE_SCORING_DISTANCE = 100.0

HIT_NORMAL = 1
HIT_WHISTLE = 1 << 1
HIT_FINISH = 1 << 2
HIT_CLAP = 1 << 3

DRUMROLL_FLAG = 2
SWELL_FLAG = 8


def convert_beatmap(
    beatmap: Beatmap,
    target_mode: int,
    mods: ModSettings | None = None,
) -> Beatmap:
    if beatmap.mode != 0:
        raise PreviewError("source beatmap must be osu!standard (mode=0)")
    if target_mode != 1:
        raise PreviewError("only taiko (mode=1) conversion is supported here")

    std_objects = [ho for ho in beatmap.hit_objects if isinstance(ho, StandardHitObject)]
    if not std_objects:
        raise PreviewError("standard beatmap has no hit objects to convert")

    taiko_objects: list[TaikoHitObject] = []
    for hit_object in std_objects:
        taiko_objects.extend(_convert_hit_object(hit_object, beatmap))

    new_general = dict(beatmap.general)
    new_general["Mode"] = "1"

    # EZ/HR 等 difficulty mod 在 lazer 的流程里发生于 Convert() 之后。
    # 因此这里保留原 difficulty，交给渲染阶段按目标 ruleset 处理显示滚速。
    return Beatmap(
        metadata=dict(beatmap.metadata),
        difficulty=dict(beatmap.difficulty),
        general=new_general,
        timing_points=_convert_timing_points(beatmap),
        hit_objects=sorted(taiko_objects, key=lambda ho: (ho.start_time, ho.end_time)),
        break_periods=list(beatmap.break_periods),
    )


def _convert_hit_object(hit_object: StandardHitObject, beatmap: Beatmap) -> list[TaikoHitObject]:
    if hit_object.hit_type & 2:
        return _convert_slider(hit_object, beatmap)

    if hit_object.hit_type & 8:
        return [
            TaikoHitObject(
                start_time=hit_object.start_time,
                end_time=hit_object.end_time,
                hit_type=SWELL_FLAG,
                hitsound=hit_object.hitsound,
            )
        ]

    return [
        TaikoHitObject(
            start_time=hit_object.start_time,
            end_time=hit_object.start_time,
            hit_type=0,
            hitsound=hit_object.hitsound,
        )
    ]


def _convert_slider(hit_object: StandardHitObject, beatmap: Beatmap) -> list[TaikoHitObject]:
    taiko_duration, tick_spacing = _slider_conversion_values(hit_object, beatmap)

    if _should_convert_slider_to_hits(hit_object, beatmap, taiko_duration, tick_spacing):
        result: list[TaikoHitObject] = []
        all_hitsounds = _slider_node_hitsounds(hit_object)
        sample_index = 0
        current_time = float(hit_object.start_time)
        # stable/lazer 会多给 tickSpacing / 8 的容差，避免浮点尾差吞掉最后一个切分 hit。
        end_time = hit_object.start_time + taiko_duration + tick_spacing / 8

        while current_time <= end_time + 1e-7:
            result.append(
                TaikoHitObject(
                    start_time=int(current_time),
                    end_time=int(current_time),
                    hit_type=0,
                    hitsound=all_hitsounds[sample_index],
                )
            )
            sample_index = (sample_index + 1) % len(all_hitsounds)

            if _almost_equals(tick_spacing, 0):
                break
            current_time += tick_spacing

        return result

    return [
        TaikoHitObject(
            start_time=hit_object.start_time,
            end_time=hit_object.start_time + taiko_duration,
            hit_type=DRUMROLL_FLAG,
            hitsound=hit_object.hitsound,
        )
    ]


def _slider_conversion_values(hit_object: StandardHitObject, beatmap: Beatmap) -> tuple[int, float]:
    spans = max(1, hit_object.slider_repeats)

    # 不要合并下面三步。lazer 源码明确保留这些中间浮点误差，用来贴合 stable。
    distance = hit_object.slider_pixel_length
    distance *= VELOCITY_MULTIPLIER
    distance *= spans

    timing_beat_length = _timing_beat_length_at(hit_object.start_time, beatmap.timing_points)
    slider_velocity = _slider_velocity_at(hit_object.start_time, beatmap.timing_points)
    beat_length = _precision_adjusted_beat_length(timing_beat_length, slider_velocity)

    slider_multiplier = _slider_multiplier(beatmap)
    slider_tick_rate = _slider_tick_rate(beatmap)
    slider_scoring_point_distance = (
        OSU_BASE_SCORING_DISTANCE * (slider_multiplier * VELOCITY_MULTIPLIER) / slider_tick_rate
    )

    taiko_velocity = slider_scoring_point_distance * slider_tick_rate
    taiko_duration = int(distance / taiko_velocity * beat_length)

    # v8+ 的谱面只在前面的 duration 计算使用带 SV 精度误差的 beatLength；
    # tickSpacing 判定会退回当前红线 BPM，这个分支是和游戏内保持一致的关键。
    if _format_version(beatmap) >= 8:
        beat_length = timing_beat_length

    tick_spacing = min(beat_length / slider_tick_rate, taiko_duration / spans)
    return taiko_duration, tick_spacing


def _should_convert_slider_to_hits(
    hit_object: StandardHitObject,
    beatmap: Beatmap,
    taiko_duration: int,
    tick_spacing: float,
) -> bool:
    spans = max(1, hit_object.slider_repeats)
    # 这里故意重新计算一遍，而不是复用上面的局部变量；
    # osu!lazer 的实现也是这样写，以保留 stable 兼容用的浮点行为。
    distance = hit_object.slider_pixel_length
    distance *= VELOCITY_MULTIPLIER
    distance *= spans

    timing_beat_length = _timing_beat_length_at(hit_object.start_time, beatmap.timing_points)
    slider_velocity = _slider_velocity_at(hit_object.start_time, beatmap.timing_points)
    beat_length = _precision_adjusted_beat_length(timing_beat_length, slider_velocity)

    slider_multiplier = _slider_multiplier(beatmap)
    slider_tick_rate = _slider_tick_rate(beatmap)
    slider_scoring_point_distance = (
        OSU_BASE_SCORING_DISTANCE * (slider_multiplier * VELOCITY_MULTIPLIER) / slider_tick_rate
    )
    taiko_velocity = slider_scoring_point_distance * slider_tick_rate
    osu_velocity = taiko_velocity * (1000.0 / beat_length)

    if _format_version(beatmap) >= 8:
        beat_length = timing_beat_length

    return tick_spacing > 0 and distance / osu_velocity * 1000.0 < 2 * beat_length


def _slider_node_hitsounds(hit_object: StandardHitObject) -> list[int]:
    if not hit_object.slider_edge_hitsounds:
        return [hit_object.hitsound]

    # edge hitsound 的 0 在 osu! 里就是纯 don，不继承 slider head。
    return list(hit_object.slider_edge_hitsounds)


def _convert_timing_points(beatmap: Beatmap) -> list[TimingPoint]:
    # standard 绿线表示 slider velocity；转成 taiko 后不能直接作为 scroll speed。
    # lazer 会把每个 slider 自己命中的 SV 转换成对应时间点的 EffectControlPoint.ScrollSpeed。
    converted = [
        point if point.uninherited else TimingPoint(
            time=point.time,
            # standard 谱的 inherited 点在 taiko convert 后不应自动变成 scroll speed。
            # lazer 里这些点的 slider velocity 会留在 DifficultyControlPoint，而不是写进 taiko 的 EffectControlPoint。
            # 这里只保留 kiai / 时序占位信息，后续 timing 状态机会忽略 NaN，不把它当成 1.0x reset。
            beat_length=math.nan,
            meter=point.meter,
            uninherited=False,
            kiai_mode=point.kiai_mode,
        )
        for point in beatmap.timing_points
    ]
    last_scroll_speed = 1.0
    additions: list[TimingPoint] = []

    for hit_object in beatmap.hit_objects:
        if not isinstance(hit_object, StandardHitObject) or not (hit_object.hit_type & 2):
            continue

        next_scroll_speed = _slider_velocity_at(hit_object.start_time, beatmap.timing_points)
        if _almost_equals(last_scroll_speed, next_scroll_speed):
            continue

        additions.append(
            TimingPoint(
                time=float(hit_object.start_time),
                beat_length=-100.0 / next_scroll_speed,
                meter=_meter_at(hit_object.start_time, beatmap.timing_points),
                uninherited=False,
                kiai_mode=_kiai_at(hit_object.start_time, beatmap.timing_points),
            )
        )
        last_scroll_speed = next_scroll_speed

    return sorted(converted + additions, key=lambda point: point.time)


def _precision_adjusted_beat_length(timing_beat_length: float, slider_velocity: float) -> float:
    # 对应 LegacyRulesetExtensions.GetPrecisionAdjustedBeatLength(..., "taiko")。
    # 这里的 float(...) 等价于 C# 里先按 float clamp 再回到 double 的兼容路径。
    slider_velocity_as_beat_length = -100.0 / slider_velocity
    bpm_multiplier = max(10.0, min(10000.0, _single(-slider_velocity_as_beat_length))) / 100.0
    return timing_beat_length * bpm_multiplier


def _timing_beat_length_at(time: int, timing_points: list[TimingPoint]) -> float:
    beat_length = 500.0
    for point in timing_points:
        if point.time > time:
            break
        if point.uninherited:
            beat_length = point.beat_length
    return beat_length


def _slider_velocity_at(time: int, timing_points: list[TimingPoint]) -> float:
    slider_velocity = 1.0
    for point in timing_points:
        if point.time > time:
            break
        if point.uninherited:
            slider_velocity = 1.0
        elif point.beat_length < -0.001:
            slider_velocity = -100.0 / point.beat_length
    return slider_velocity


def _meter_at(time: int, timing_points: list[TimingPoint]) -> int:
    meter = 4
    for point in timing_points:
        if point.time > time:
            break
        if point.uninherited:
            meter = point.meter
    return meter


def _kiai_at(time: int, timing_points: list[TimingPoint]) -> bool:
    kiai = False
    for point in timing_points:
        if point.time > time:
            break
        kiai = point.kiai_mode
    return kiai


def _format_version(beatmap: Beatmap) -> int:
    return int(beatmap.general.get("FormatVersion", "14"))


def _almost_equals(a: float, b: float, acceptable_difference: float = 1e-7) -> bool:
    return math.isclose(a, b, abs_tol=acceptable_difference)


def _single(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", value))[0]


def _slider_multiplier(beatmap: Beatmap) -> float:
    # legacy decoder 会把难度参数限制到 stable 允许范围内，转谱前需要同样处理。
    return max(0.4, min(3.6, float(beatmap.difficulty["SliderMultiplier"])))


def _slider_tick_rate(beatmap: Beatmap) -> float:
    return max(0.5, min(8.0, float(beatmap.difficulty["SliderTickRate"])))
