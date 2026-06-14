from __future__ import annotations

import random
from dataclasses import dataclass
from typing import Protocol

from .errors import PreviewError
from .models import Beatmap, BreakPeriod


BREAK_GAP_MS = 2200


class TimedHitObject(Protocol):
    start_time: int
    end_time: int


@dataclass(frozen=True)
class PreviewSegmentTiming:
    start_time: int
    is_preview: bool
    break_periods: tuple[BreakPeriod, ...]


def times_to_milliseconds(times: list[float] | None) -> list[int] | None:
    if times is None:
        return None
    return [round(time * 1000) for time in times]


class PreviewTimeSelector:
    """为 GIF/PNG 片段选择开始时间，并尽量避开 break 与长空窗。"""

    def __init__(
        self,
        beatmap: Beatmap,
        hit_objects: list[TimedHitObject],
        segment_count: int,
        segment_duration: int,
        requested_start_times: list[int] | None = None,
    ) -> None:
        if segment_count <= 0:
            raise PreviewError("segment count must be positive")
        if segment_duration < 0:
            raise PreviewError("segment duration must be non-negative")
        if not hit_objects:
            raise PreviewError("beatmap has no hit objects")

        self.beatmap = beatmap
        self.hit_objects = sorted(hit_objects, key=lambda ho: (ho.start_time, ho.end_time))
        self.segment_count = segment_count
        self.segment_duration = segment_duration
        self.requested_start_times = requested_start_times or []

    def choose(self) -> list[PreviewSegmentTiming]:
        valid_intervals = self._build_valid_start_intervals()
        preview_time = self._preview_time()
        chosen = self._build_forced_times(preview_time)

        # 随机补足剩余片段，并避免多个 5 秒片段互相覆盖。
        random_source = random.Random()
        attempts = 0
        while valid_intervals and len(chosen) < self.segment_count and attempts < 3000:
            attempts += 1
            candidate = _random_start_from_intervals(valid_intervals, random_source)
            if _does_not_overlap_existing(candidate, self.segment_duration, chosen):
                chosen.append(candidate)

        if valid_intervals and len(chosen) < self.segment_count:
            for candidate in self._fallback_start_candidates(valid_intervals):
                if _does_not_overlap_existing(candidate, self.segment_duration, chosen):
                    chosen.append(candidate)
                if len(chosen) == self.segment_count:
                    break

        return [
            PreviewSegmentTiming(
                start_time=start_time,
                is_preview=start_time == preview_time,
                break_periods=tuple(_break_periods_overlapping_segment(
                    self.beatmap.break_periods,
                    start_time,
                    self.segment_duration,
                )),
            )
            for start_time in sorted(chosen)
        ]

    def _build_forced_times(self, preview_time: int) -> list[int]:
        # 用户指定时间优先；指定满 4 个时不再强塞 PreviewTime。
        chosen: list[int] = []

        for start_time in self.requested_start_times:
            if start_time < 0:
                raise PreviewError(f"requested time must be non-negative, got {start_time}")
            if start_time not in chosen:
                chosen.append(start_time)

        if len(chosen) > self.segment_count:
            raise PreviewError(
                f"--times accepts at most {self.segment_count} time point"
                f"{'' if self.segment_count == 1 else 's'}"
            )

        if len(chosen) < self.segment_count and preview_time not in chosen:
            chosen.append(preview_time)

        return chosen

    def _preview_time(self) -> int:
        preview_time = int(self.beatmap.general.get("PreviewTime", "-1"))
        if preview_time < 0:
            preview_time = self.hit_objects[0].start_time
        return preview_time

    def _build_valid_start_intervals(self) -> list[tuple[int, int]]:
        chart_start = self.hit_objects[0].start_time
        chart_end = max(hit_object.end_time for hit_object in self.hit_objects)
        # 同时使用谱面声明的 break 和物件之间推断出的长空窗。
        forbidden = _merge_periods([*self.beatmap.break_periods, *_infer_break_periods(self.hit_objects)])
        playable_segments = _subtract_periods(chart_start, chart_end, forbidden)

        intervals: list[tuple[int, int]] = []
        for start, end in playable_segments:
            latest_start = end - self.segment_duration
            if latest_start >= start:
                intervals.append((start, latest_start))
        return intervals

    def _fallback_start_candidates(self, intervals: list[tuple[int, int]]) -> list[int]:
        candidates = [_nearest_valid_start(hit_object.start_time, intervals) for hit_object in self.hit_objects]
        return sorted(set(candidates))


def _infer_break_periods(hit_objects: list[TimedHitObject]) -> list[BreakPeriod]:
    periods: list[BreakPeriod] = []
    previous_end = hit_objects[0].end_time
    for hit_object in hit_objects[1:]:
        if hit_object.start_time - previous_end >= BREAK_GAP_MS:
            periods.append(BreakPeriod(start_time=previous_end, end_time=hit_object.start_time))
        previous_end = max(previous_end, hit_object.end_time)
    return periods


def _merge_periods(periods: list[BreakPeriod]) -> list[BreakPeriod]:
    ordered = sorted(periods, key=lambda period: (period.start_time, period.end_time))
    merged: list[BreakPeriod] = []
    for period in ordered:
        if not merged or period.start_time > merged[-1].end_time:
            merged.append(period)
            continue
        previous = merged[-1]
        merged[-1] = BreakPeriod(previous.start_time, max(previous.end_time, period.end_time))
    return merged


def _subtract_periods(start_time: int, end_time: int, forbidden_periods: list[BreakPeriod]) -> list[tuple[int, int]]:
    segments: list[tuple[int, int]] = []
    cursor = start_time
    for period in forbidden_periods:
        if period.end_time <= cursor:
            continue
        if period.start_time > cursor:
            segments.append((cursor, min(period.start_time, end_time)))
        cursor = max(cursor, period.end_time)
        if cursor >= end_time:
            break
    if cursor < end_time:
        segments.append((cursor, end_time))
    return [(start, end) for start, end in segments if end > start]


def _nearest_valid_start(time: int, intervals: list[tuple[int, int]]) -> int:
    if any(start <= time <= end for start, end in intervals):
        return time
    return min(
        (start if time < start else end for start, end in intervals),
        key=lambda candidate: abs(candidate - time),
    )


def _random_start_from_intervals(intervals: list[tuple[int, int]], random_source: random.Random) -> int:
    total = sum(end - start + 1 for start, end in intervals)
    pick = random_source.randrange(total)
    for start, end in intervals:
        length = end - start + 1
        if pick < length:
            return start + pick
        pick -= length
    return intervals[-1][1]


def _does_not_overlap_existing(candidate: int, segment_duration: int, chosen: list[int]) -> bool:
    candidate_end = candidate + segment_duration
    for existing in chosen:
        existing_end = existing + segment_duration
        if candidate < existing_end and candidate_end > existing:
            return False
    return True


def _break_periods_overlapping_segment(
    break_periods: list[BreakPeriod],
    segment_start_time: int,
    segment_duration: int,
) -> list[BreakPeriod]:
    segment_end_time = segment_start_time + segment_duration
    return [
        period
        for period in break_periods
        if period.start_time < segment_end_time and period.end_time > segment_start_time
    ]
