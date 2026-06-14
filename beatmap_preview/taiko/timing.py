from __future__ import annotations

import math
from bisect import bisect_right
from dataclasses import dataclass

from ..models import TimingPoint
from .config import (
    DEFAULT_BEAT_LENGTH,
    DEFAULT_METER,
    PIXELS_PER_SCROLL_MULTIPLIER_MS,
    SCROLL_LENGTH_RATIO,
)


@dataclass(frozen=True)
class ScrollSegment:
    # 一个滚动 segment 表示在某段时间里横向滚动速度恒定。
    start_time: int
    end_time: int
    beat_length: float
    meter: int
    scroll_speed: float
    pixels_per_ms: float
    start_position: float


@dataclass(frozen=True)
class RedlineSection:
    # 红线 section 只负责 BPM 与拍号，用于生成小节线和时间标签。
    start_time: int
    end_time: int
    beat_length: float
    meter: int


@dataclass(frozen=True)
class KiaiSection:
    # 一个 section 表示一段连续开启 kiai 的时间区间。
    start_time: int
    end_time: int


@dataclass(frozen=True)
class TimingLine:
    # 渲染阶段只关心线落在什么位置、是不是小节线、要不要显示标签。
    time: int
    position: float
    is_measure: bool
    show_label: bool
    is_kiai: bool
    is_kiai_start: bool
    bpm: float | None = None


class ScrollPositionMapper:
    """把毫秒时间映射为横向滚动位置。"""

    def __init__(self, segments: list[ScrollSegment]):
        self.segments = segments
        self._start_times = [segment.start_time for segment in segments]

    @property
    def end_position(self) -> float:
        # 最后一段的起点位置 + 最后一段内部走过的距离 = 整张谱面的最终横向长度。
        segment = self.segments[-1]
        return segment.start_position + (segment.end_time - segment.start_time) * segment.pixels_per_ms

    def position_at(self, time: float) -> float:
        # 二分找到 time 所属的 segment，再用 segment 内的线性速度计算局部位移。
        clamped_time = max(0.0, min(time, self.segments[-1].end_time))
        segment_index = max(0, bisect_right(self._start_times, clamped_time) - 1)
        segment = self.segments[segment_index]
        return segment.start_position + (clamped_time - segment.start_time) * segment.pixels_per_ms


def build_scroll_mapper(
    timing_points: list[TimingPoint],
    chart_end_time: int,
    slider_multiplier: float,
    spacing_bpm: float,
) -> ScrollPositionMapper:
    # taiko 横向坐标由时间轴积分得到；每个 segment 内像素速度恒定。
    segments = _build_scroll_segments(
        timing_points=timing_points,
        chart_end_time=chart_end_time,
        slider_multiplier=slider_multiplier,
        spacing_bpm=spacing_bpm,
    )
    return ScrollPositionMapper(segments)


def build_redline_sections(timing_points: list[TimingPoint], chart_end_time: int) -> list[RedlineSection]:
    # 先确定 0ms 开始时生效的 BPM / 拍号。
    beat_length = DEFAULT_BEAT_LENGTH
    meter = DEFAULT_METER
    section_start = 0

    for point in timing_points:
        if point.time > 0:
            break
        if point.uninherited:
            beat_length = point.beat_length if point.beat_length >= 60 else 60_000 / 180
            meter = point.meter

    sections: list[RedlineSection] = []
    for point in timing_points:
        point_time = int(round(point.time))
        if point_time <= 0 or point_time >= chart_end_time or not point.uninherited:
            continue
        # 遇到下一根红线时，把上一段 [section_start, point_time) 封成 section。
        if point_time > section_start:
            sections.append(
                RedlineSection(
                    start_time=section_start,
                    end_time=point_time,
                    beat_length=beat_length,
                    meter=meter,
                )
            )
        beat_length = point.beat_length if point.beat_length >= 60 else 60_000 / 180
        meter = point.meter
        section_start = point_time

    sections.append(
        RedlineSection(
            start_time=section_start,
            end_time=chart_end_time,
            beat_length=beat_length,
            meter=meter,
        )
    )
    return sections


def build_kiai_sections(timing_points: list[TimingPoint], chart_end_time: int) -> list[KiaiSection]:
    # parser 已经把 timing point 的 effects 解析成 kiai_mode；
    # 这里仅把连续开启的点合并成区间，供渲染阶段直接使用。
    kiai_mode = False
    active_start: int | None = None

    for point in timing_points:
        if point.time > 0:
            break
        kiai_mode = point.kiai_mode

    if kiai_mode:
        active_start = 0

    sections: list[KiaiSection] = []
    for point in timing_points:
        point_time = int(round(point.time))
        if point_time <= 0 or point_time >= chart_end_time:
            continue
        if point.kiai_mode == kiai_mode:
            continue

        if kiai_mode:
            sections.append(KiaiSection(start_time=active_start or 0, end_time=point_time))
            active_start = None
        else:
            active_start = point_time

        kiai_mode = point.kiai_mode

    if kiai_mode:
        sections.append(KiaiSection(start_time=active_start or 0, end_time=chart_end_time))
    return sections


def build_timing_lines(
    redline_sections: list[RedlineSection],
    mapper: ScrollPositionMapper,
    min_beat_line_spacing: int,
    kiai_sections: list[KiaiSection],
    first_note_time: int,
) -> list[TimingLine]:
    # 同一毫秒可能因为 timing section 边界被重复生成，最后用 dict 去重。
    line_by_time: dict[int, TimingLine] = {}
    last_bpm: float | None = None
    deferred_first_bpm: float | None = None

    for section in redline_sections:
        bpm = 60_000.0 / section.beat_length
        show_bpm = last_bpm is None or abs(bpm - last_bpm) > 0.01
        last_bpm = bpm

        if show_bpm and section.start_time == 0 and first_note_time > 0:
            deferred_first_bpm = bpm
            show_bpm = False

        beat_index = 0
        current_time = float(section.start_time)
        while current_time <= section.end_time + 0.001:
            rounded_time = int(round(current_time))
            next_time = current_time + section.beat_length
            # beat_spacing 用来控制普通拍线密度，避免 taiko 像 mania 那样过密。
            beat_spacing = mapper.position_at(min(next_time, section.end_time)) - mapper.position_at(current_time)
            is_measure = beat_index % section.meter == 0
            is_first_beat = beat_index == 0

            if is_measure or beat_spacing >= min_beat_line_spacing or (show_bpm and is_first_beat):
                _merge_timing_line(
                    line_by_time=line_by_time,
                    time=rounded_time,
                    position=mapper.position_at(current_time),
                    is_measure=is_measure,
                    show_label=True,
                    is_kiai=False,
                    is_kiai_start=False,
                    bpm=round(bpm) if show_bpm and is_first_beat else None,
                )
                if show_bpm and is_first_beat:
                    show_bpm = False

            current_time = next_time
            beat_index += 1

    # kiai 开始时间即使不在拍线上，也单独补一根带标签的线，方便看清切换点。
    for section in kiai_sections:
        _merge_timing_line(
            line_by_time=line_by_time,
            time=section.start_time,
            position=mapper.position_at(section.start_time),
            is_measure=False,
            show_label=True,
            is_kiai=True,
            is_kiai_start=True,
        )

    if deferred_first_bpm is not None and first_note_time > 0:
        _merge_timing_line(
            line_by_time=line_by_time,
            time=first_note_time,
            position=mapper.position_at(first_note_time),
            is_measure=False,
            show_label=True,
            is_kiai=False,
            is_kiai_start=False,
            bpm=round(deferred_first_bpm),
        )

    return _dedupe_display_labels(_apply_kiai_flags(line_by_time, kiai_sections))


def _build_scroll_segments(
    timing_points: list[TimingPoint],
    chart_end_time: int,
    slider_multiplier: float,
    spacing_bpm: float,
) -> list[ScrollSegment]:
    # beat_length / meter / scroll_speed 始终表示“当前时刻实际生效”的滚动状态。
    beat_length = DEFAULT_BEAT_LENGTH
    meter = DEFAULT_METER
    scroll_speed = 1.0

    for point in timing_points:
        if point.time > 0:
            break
        beat_length, meter, scroll_speed = _apply_timing_state(point, beat_length, meter, scroll_speed)

    # spacing_bpm=0 时使用游戏内当前 BPM；
    # 否则强制用固定 BPM 来统一横向间距。
    display_beat_length = 60_000 / spacing_bpm if spacing_bpm > 0 else beat_length
    segment_start = 0
    segment_position = 0.0
    segments: list[ScrollSegment] = []

    for point in timing_points:
        point_time = int(round(point.time))
        if point_time <= 0 or point_time >= chart_end_time:
            continue

        if point_time > segment_start:
            # 每次控制点发生变化时，先把上一段速度恒定的区间落成 segment。
            pixels_per_ms = _pixels_per_ms(slider_multiplier, scroll_speed, display_beat_length)
            segments.append(
                ScrollSegment(
                    start_time=segment_start,
                    end_time=point_time,
                    beat_length=beat_length,
                    meter=meter,
                    scroll_speed=scroll_speed,
                    pixels_per_ms=pixels_per_ms,
                    start_position=segment_position,
                )
            )
            segment_position += (point_time - segment_start) * pixels_per_ms

        # 更新 point_time 之后生效的新滚动状态。
        beat_length, meter, scroll_speed = _apply_timing_state(point, beat_length, meter, scroll_speed)
        display_beat_length = 60_000 / spacing_bpm if spacing_bpm > 0 else beat_length
        segment_start = point_time

    pixels_per_ms = _pixels_per_ms(slider_multiplier, scroll_speed, display_beat_length)
    segments.append(
        ScrollSegment(
            start_time=segment_start,
            end_time=chart_end_time,
            beat_length=beat_length,
            meter=meter,
            scroll_speed=scroll_speed,
            pixels_per_ms=pixels_per_ms,
            start_position=segment_position,
        )
    )
    return segments


def _merge_timing_line(
    line_by_time: dict[int, TimingLine],
    time: int,
    position: float,
    is_measure: bool,
    show_label: bool,
    is_kiai: bool,
    is_kiai_start: bool,
    bpm: float | None = None,
) -> None:
    existing = line_by_time.get(time)
    if existing is None:
        line_by_time[time] = TimingLine(
            time=time,
            position=position,
            is_measure=is_measure,
            show_label=show_label,
            is_kiai=is_kiai,
            is_kiai_start=is_kiai_start,
            bpm=bpm,
        )
        return

    line_by_time[time] = TimingLine(
        time=time,
        position=existing.position,
        is_measure=existing.is_measure or is_measure,
        show_label=existing.show_label or show_label,
        is_kiai=existing.is_kiai or is_kiai,
        is_kiai_start=existing.is_kiai_start or is_kiai_start,
        bpm=existing.bpm if existing.bpm is not None else bpm,
    )


def _apply_kiai_flags(
    line_by_time: dict[int, TimingLine],
    kiai_sections: list[KiaiSection],
) -> list[TimingLine]:
    # 这里把“这根线是否位于 kiai 中”统一补齐，渲染层就不用再关心 section 查找。
    lines: list[TimingLine] = []
    kiai_index = 0

    for time in sorted(line_by_time):
        line = line_by_time[time]
        while kiai_index < len(kiai_sections) and kiai_sections[kiai_index].end_time <= time:
            kiai_index += 1

        is_kiai = line.is_kiai
        if kiai_index < len(kiai_sections):
            current_section = kiai_sections[kiai_index]
            is_kiai = is_kiai or current_section.start_time <= time < current_section.end_time

        lines.append(
            TimingLine(
                time=line.time,
                position=line.position,
                is_measure=line.is_measure,
                show_label=line.show_label,
                is_kiai=is_kiai,
                is_kiai_start=line.is_kiai_start,
                bpm=line.bpm,
            )
        )

    return lines


def _dedupe_display_labels(lines: list[TimingLine]) -> list[TimingLine]:
    # 渲染时标签只保留 1 位小数，因此很接近的两根线可能显示成同一个时间文本。
    # 遇到这种情况时优先保留 kiai start 的那一根，避免肉眼看起来像重复渲染。
    deduped: list[TimingLine] = []

    for line in lines:
        if not line.show_label or not deduped:
            deduped.append(line)
            continue

        previous = deduped[-1]
        same_label = f"{previous.time / 1000:.1f}s" == f"{line.time / 1000:.1f}s"
        if not previous.show_label or not same_label:
            deduped.append(line)
            continue

        if (line.is_kiai_start and not previous.is_kiai_start) or (line.bpm is not None and previous.bpm is None):
            deduped[-1] = TimingLine(
                time=previous.time,
                position=previous.position,
                is_measure=previous.is_measure,
                show_label=False,
                is_kiai=previous.is_kiai,
                is_kiai_start=previous.is_kiai_start,
                bpm=None,
            )
            deduped.append(line)
            continue

        deduped.append(
            TimingLine(
                time=line.time,
                position=line.position,
                is_measure=line.is_measure,
                show_label=False,
                is_kiai=line.is_kiai,
                is_kiai_start=line.is_kiai_start,
                bpm=None,
            )
        )

    return deduped


def _apply_timing_state(
    point: TimingPoint,
    beat_length: float,
    meter: int,
    scroll_speed: float,
) -> tuple[float, int, float]:
    # 红线修改 BPM / 拍号；绿线修改 scroll speed。
    if point.uninherited:
        bl = point.beat_length if point.beat_length >= 60 else 60_000 / 180
        return bl, point.meter, scroll_speed
    if math.isnan(point.beat_length):
        return beat_length, meter, scroll_speed
    if point.beat_length >= -0.001:
        return beat_length, meter, 1.0
    return beat_length, meter, -100 / point.beat_length


@dataclass(frozen=True)
class SvChange:
    time: int
    position: float
    sv: float


def build_sv_changes(
    timing_points: list[TimingPoint],
    chart_end_time: int,
    mapper: ScrollPositionMapper,
) -> list[SvChange]:
    inherited = [
        tp for tp in timing_points
        if not tp.uninherited and tp.beat_length < -0.001 and 0 <= tp.time <= chart_end_time
    ]
    if not inherited:
        return []

    changes: list[SvChange] = []
    prev_sv: float | None = None
    for tp in inherited:
        sv = -100.0 / tp.beat_length
        if prev_sv is not None and abs(sv - prev_sv) <= 0.001:
            continue
        prev_sv = sv
        changes.append(SvChange(time=int(tp.time), position=mapper.position_at(tp.time), sv=sv))

    return changes


def _pixels_per_ms(
    slider_multiplier: float,
    scroll_speed: float,
    display_beat_length: float,
) -> float:
    # root cause：
    # 之前把 0.14 当成最终速度常量直接使用了。
    # 但在游戏里，0.14 对应的是 ComputeTimeRange() 里基于 inLength 推出来的部分，
    # 真正参与滚动位置计算的还有一个固定的 scrollLength / inLength 比例。
    #
    # 由 lazer / stable 的 taiko 布局可得：
    # scrollLength = aspect * 768 - 256
    # inLength = aspect * 480 - 160
    # 二者恒满足 scrollLength = 1.6 * inLength
    #
    # 所以最终像素速度应该是 0.14 再乘以 1.6，而不是乘行高缩放。
    # 这就是之前 `SPACING_BPM=0` 看起来比游戏更密、而上次修改后又变得更密的原因。
    #
    # 两个物件最终像素间距 = 时间差 * pixels_per_ms。
    # 而 pixels_per_ms 同时受：
    # 1. 当前 base velocity（SliderMultiplier）
    # 2. 当前绿线 scroll speed
    # 3. 当前显示 BPM（真实 BPM 或配置的 SPACING_BPM）
    # 共同决定。
    return (
        PIXELS_PER_SCROLL_MULTIPLIER_MS
        * SCROLL_LENGTH_RATIO
        * slider_multiplier
        * scroll_speed
        * 1000
        / display_beat_length
    )
