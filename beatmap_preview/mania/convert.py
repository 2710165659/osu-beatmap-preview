"""standard → mania 转谱，匹配 osu!lazer 的 ManiaBeatmapConverter + PatternGenerator。"""

from __future__ import annotations

import math
from collections.abc import Callable
from enum import IntFlag

from ..errors import PreviewError
from ..models import Beatmap, ManiaHitObject, StandardHitObject, TimingPoint
from ..mods import ModSettings

SOURCE_MODE_KEY = "PreviewSourceMode"


HIT_WHISTLE = 1 << 1
HIT_FINISH = 1 << 2
HIT_CLAP = 1 << 3


class _LegacyRandom:
    """osu!stable/lazer legacy FastRandom implementation used by mania conversion."""

    _INT_TO_REAL = 1.0 / (2**31)
    _INT_MASK = 0x7FFFFFFF

    def __init__(self, seed: int) -> None:
        self.x = seed & 0xFFFFFFFF
        self.y = 842502087
        self.z = 3579807591
        self.w = 273326509

    def next_uint(self) -> int:
        t = (self.x ^ ((self.x << 11) & 0xFFFFFFFF)) & 0xFFFFFFFF
        self.x = self.y
        self.y = self.z
        self.z = self.w
        self.w = (self.w ^ (self.w >> 19) ^ t ^ (t >> 8)) & 0xFFFFFFFF
        return self.w

    def next(self, lower: int | None = None, upper: int | None = None) -> int:
        if lower is None and upper is None:
            return self._INT_MASK & self.next_uint()
        if upper is None:
            upper = lower
            lower = 0
        assert lower is not None and upper is not None
        return int(lower + self.next_double() * (upper - lower))

    def next_double(self) -> float:
        return self._INT_TO_REAL * self.next()


# ── pattern type 位标志 ───────────────────────────────────────────────────────

class Pat(IntFlag):
    NONE = 0
    ForceNotStack = 1 << 0
    KeepSingle = 1 << 1
    Mirror = 1 << 2
    Gathered = 1 << 3
    Stair = 1 << 4
    Reverse = 1 << 5
    Cycle = 1 << 6
    LowProbability = 1 << 7
    ForceStack = 1 << 8

    # stair 方向追踪用的额外位（不参与 pattern 逻辑）
    ReverseStair = 1 << 9


# ── 主入口 ────────────────────────────────────────────────────────────────────

def convert_beatmap(
    beatmap: Beatmap,
    target_mode: int,
    mods: ModSettings | None = None,
) -> Beatmap:
    if beatmap.mode != 0:
        raise PreviewError("source beatmap must be osu!standard (mode=0)")
    if target_mode != 3:
        raise PreviewError("only mania (mode=3) conversion is currently supported")

    std_objects = [ho for ho in beatmap.hit_objects if isinstance(ho, StandardHitObject)]
    if not std_objects:
        raise PreviewError("standard beatmap has no hit objects to convert")

    diff = beatmap.difficulty
    total_columns = _resolve_total_columns(std_objects, diff, mods)
    seed = _build_seed(diff)
    rng = _LegacyRandom(seed)
    conv_diff = _compute_conversion_difficulty(std_objects, beatmap.break_periods, diff)

    gen = _ConversionState(
        rng=rng,
        total_columns=total_columns,
        conv_diff=conv_diff,
        timing_points=beatmap.timing_points,
        difficulty=diff,
    )
    mania_objects = gen.convert_all(std_objects)

    new_diff = dict(diff)
    new_diff["CircleSize"] = str(total_columns)
    new_general = dict(beatmap.general)
    new_general[SOURCE_MODE_KEY] = str(beatmap.mode)
    new_general["Mode"] = "3"

    return Beatmap(
        metadata=dict(beatmap.metadata),
        difficulty=new_diff,
        general=new_general,
        timing_points=list(beatmap.timing_points),
        hit_objects=mania_objects,
        break_periods=list(beatmap.break_periods),
    )


# ── 列数 & 种子 ────────────────────────────────────────────────────────────────

def _resolve_total_columns(
    hit_objects: list[StandardHitObject],
    difficulty: dict[str, str],
    mods: ModSettings | None,
) -> int:
    if mods is not None and mods.mania_keys is not None:
        cols = mods.mania_keys
        if mods.dual_stage:
            cols = min(18, cols * 2)
        return cols

    cs = float(difficulty.get("CircleSize", "4"))
    od = float(difficulty.get("OverallDifficulty", "8"))
    rounded_cs = round(cs)
    rounded_od = round(od)
    total = len(hit_objects)
    end_time_obj = sum(1 for ho in hit_objects if ho.end_time > ho.start_time)
    ratio = end_time_obj / total if total > 0 else 0.0

    if ratio < 0.2:
        cols = 7
    elif ratio < 0.3 or rounded_cs >= 5:
        cols = 7 if rounded_od > 5 else 6
    elif ratio > 0.6:
        cols = 5 if rounded_od > 4 else 4
    else:
        cols = max(4, min(7, rounded_od + 1))

    if mods is not None and mods.dual_stage:
        cols = min(18, cols * 2)
    return cols


def _build_seed(difficulty: dict[str, str]) -> int:
    cs = float(difficulty.get("CircleSize", "4"))
    od = float(difficulty.get("OverallDifficulty", "8"))
    ar = float(difficulty.get("ApproachRate", "5"))
    dr = float(difficulty.get("HPDrainRate", "5"))
    return round(dr + cs) * 20 + int(od * 41.2) + round(ar)


def _compute_conversion_difficulty(
    hit_objects: list[StandardHitObject],
    break_periods: list,
    difficulty: dict[str, str],
) -> float:
    total_break = sum(b.end_time - b.start_time for b in break_periods)
    first_time = hit_objects[0].start_time
    last_time = hit_objects[-1].start_time
    drain_time = int((last_time - first_time - total_break) / 1000)
    if drain_time == 0:
        drain_time = 10000.0

    dr = float(difficulty.get("HPDrainRate", "5"))
    ar = float(difficulty.get("ApproachRate", "5"))
    clamped_ar = max(4.0, min(7.0, ar))
    obj_density = len(hit_objects) / drain_time

    cd = ((dr + clamped_ar) / 1.5 + obj_density * 9.0) / 38.0 * 5.0 / 1.15
    return min(cd, 12.0)


def _resolve_slider_timing(start_time: int, timing_points: list[TimingPoint]) -> tuple[float, float]:
    beat_length = timing_points[0].beat_length if timing_points else 500.0
    slider_velocity = 1.0

    for point in timing_points:
        if point.time > start_time:
            break
        if point.uninherited:
            beat_length = point.beat_length
            slider_velocity = 1.0
        elif point.beat_length < 0:
            slider_velocity = -100 / point.beat_length

    return beat_length, slider_velocity


def _kiai_at(time: int, timing_points: list[TimingPoint]) -> bool:
    kiai = False
    for point in timing_points:
        if point.time > time:
            break
        kiai = point.kiai_mode
    return kiai


# ── 转换状态机 ─────────────────────────────────────────────────────────────────

class _ConversionState:
    """持有跨越单个 hit object 的状态（stair 方向、前一个 pattern、density 队列）。"""

    def __init__(
        self,
        rng: _LegacyRandom,
        total_columns: int,
        conv_diff: float,
        timing_points: list[TimingPoint],
        difficulty: dict[str, str],
    ) -> None:
        self.rng = rng
        self.total_columns = total_columns
        self.conv_diff = conv_diff
        self.timing_points = timing_points
        self.difficulty = difficulty
        self.random_start = 1 if total_columns == 8 else 0

        self.stair_type = Pat.Stair
        self.prev_pattern = _Pattern()
        self.prev_note_times: list[int] = []  # 最近 7 个 note 时间
        self.last_time = 0
        self.last_x = 0
        self.last_y = 0

    # ── 主循环 ──

    def convert_all(self, hit_objects: list[StandardHitObject]) -> list[ManiaHitObject]:
        result: list[ManiaHitObject] = []
        for ho in hit_objects:
            if ho.end_time > ho.start_time and ho.slider_type is not None:
                for i in range(ho.slider_repeats + 1):
                    time = int(ho.start_time + self._slider_segment_duration(ho) * i)
                    self._record_note(time, ho.x, ho.y)
                    self._compute_density(time)
                gen = _SliderGenerator(self, ho)
                result.extend(gen.generate())
            elif ho.end_time > ho.start_time:
                self._record_note(ho.end_time, 256, 192)
                self._compute_density(ho.end_time)
                gen = _SpinnerGenerator(self, ho)
                result.extend(gen.generate())
            else:
                time_gap = ho.start_time - self.last_time
                pos_gap = math.hypot(ho.x - self.last_x, ho.y - self.last_y)
                self._compute_density(ho.start_time)
                gen = _CircleGenerator(self, ho, time_gap, pos_gap)
                result.extend(gen.generate())
                self._record_note(ho.start_time, ho.x, ho.y)
        return result

    def _compute_density(self, time: int) -> None:
        self.prev_note_times.append(time)
        if len(self.prev_note_times) > 7:
            self.prev_note_times.pop(0)

    def _record_note(self, time: int, x: int, y: int) -> None:
        self.last_time = time
        self.last_x = x
        self.last_y = y

    def density(self) -> float:
        if len(self.prev_note_times) < 2:
            return float(2**31 - 1)
        return (self.prev_note_times[-1] - self.prev_note_times[0]) / len(self.prev_note_times)

    def _slider_segment_duration(self, ho: StandardHitObject) -> int:
        span_count = max(1, ho.slider_repeats)
        beat_length, slider_velocity = _resolve_slider_timing(ho.start_time, self.timing_points)
        adjusted_beat_length = beat_length * max(10.0, min(10000.0, 100.0 / slider_velocity)) / 100.0
        duration = math.floor(
            ho.start_time
            + ho.slider_pixel_length * adjusted_beat_length * span_count * 0.01 / float(self.difficulty["SliderMultiplier"])
        ) - ho.start_time
        return duration // span_count if span_count > 0 else duration

    # ── 列选择工具 ──

    def get_column(self, x: int, allow_special: bool = False) -> int:
        if allow_special and self.total_columns == 8:
            return min(6, x * 7 // 512) + 1
        return min(self.total_columns - 1, x * self.total_columns // 512)

    def get_random_column(self, lo: int | None = None, hi: int | None = None) -> int:
        return self.rng.next(
            lo if lo is not None else self.random_start,
            hi if hi is not None else self.total_columns,
        )

    def find_available_column(
        self,
        start: int,
        patterns: list[_Pattern],
        lo: int | None = None,
        hi: int | None = None,
        next_column: Callable[[int], int] | None = None,
        validation: Callable[[int], bool] | None = None,
    ) -> int:
        lo = lo if lo is not None else self.random_start
        hi = hi if hi is not None else self.total_columns
        if next_column is None:
            next_column = lambda _last: self.get_random_column(lo, hi)

        def _ok(c: int) -> bool:
            if validation is not None and not validation(c):
                return False
            for p in patterns:
                if p.has_column(c):
                    return False
            return True

        if lo <= start < hi and _ok(start):
            return start
        if not any(_ok(c) for c in range(lo, hi)):
            raise PreviewError("not enough columns to complete mania conversion")

        col = start
        while True:
            col = next_column(col)
            if _ok(col):
                return col

    def get_random_note_count(self, p2: float = 0, p3: float = 0,
                               p4: float = 0, p5: float = 0) -> int:
        """逆累积概率：val >= 1-pN → 返回 N（最高匹配者胜）。"""
        val = self.rng.next_double()
        if p5 and val >= 1 - p5:
            return 5
        if p4 and val >= 1 - p4:
            return 4
        if p3 and val >= 1 - p3:
            return 3
        if p2 and val >= 1 - p2:
            return 2
        return 1


# ── Pattern ───────────────────────────────────────────────────────────────────

class _Pattern:
    """一组 mania 物件（可能同时发生）。"""

    def __init__(self) -> None:
        self.columns: set[int] = set()
        self.objects: list[ManiaHitObject] = []

    def has_column(self, c: int) -> bool:
        return c in self.columns

    def add(self, col: int, start_time: int, end_time: int) -> None:
        self.columns.add(col)
        self.objects.append(
            ManiaHitObject(
                lane=col,
                start_time=start_time,
                end_time=end_time,
                is_long_note=end_time > start_time,
            )
        )

    @property
    def column_count(self) -> int:
        return len(self.columns)

    @property
    def any_column(self) -> int:
        return next(iter(self.columns)) if self.columns else 0


# ── Circle Generator ──────────────────────────────────────────────────────────

class _CircleGenerator:
    def __init__(self, s: _ConversionState, ho: StandardHitObject,
                 time_gap: int, pos_gap: int) -> None:
        self.s = s
        self.ho = ho
        self.conv_type = self._resolve_convert_type(time_gap, pos_gap)

    def _resolve_convert_type(self, time_gap: int, pos_gap: int) -> Pat:
        ct = Pat.NONE
        T = self.s.total_columns
        prev = self.s.prev_pattern
        beat_len = self._beat_length_at(self.ho.start_time)
        density = self.s.density()

        if time_gap <= 80:
            ct |= Pat.ForceNotStack | Pat.KeepSingle
        elif time_gap <= 95:
            ct |= Pat.ForceNotStack | Pat.KeepSingle | self.s.stair_type
        elif time_gap <= 105:
            ct |= Pat.ForceNotStack | Pat.LowProbability
        elif time_gap <= 125:
            ct |= Pat.ForceNotStack
        elif time_gap <= 135 and pos_gap < 20:
            ct |= Pat.Cycle | Pat.KeepSingle
        elif time_gap <= 150 and pos_gap < 20:
            ct |= Pat.ForceStack | Pat.LowProbability
        elif pos_gap < 20 and density >= beat_len / 2.5:
            ct |= Pat.Reverse | Pat.LowProbability
        elif density < beat_len / 2.5 or _kiai_at(self.ho.start_time, self.s.timing_points):
            pass  # high density, no special flag
        else:
            ct |= Pat.LowProbability

        if Pat.KeepSingle not in ct:
            if (self.ho.hitsound & HIT_FINISH) and T != 8:
                ct |= Pat.Mirror
            elif self.ho.hitsound & HIT_CLAP:
                ct |= Pat.Gathered

        return ct

    def generate(self) -> list[ManiaHitObject]:
        s = self.s
        T = s.total_columns
        ct = self.conv_type
        pattern = _Pattern()

        if T <= 1:
            pattern.add(0, self.ho.start_time, self.ho.start_time)
            s.prev_pattern = pattern
            return pattern.objects

        # Reverse
        if Pat.Reverse in ct and s.prev_pattern.column_count > 0:
            for c in range(s.random_start, T):
                if s.prev_pattern.has_column(c):
                    pattern.add(s.random_start + T - c - 1,
                                self.ho.start_time, self.ho.start_time)
            s.prev_pattern = pattern
            return pattern.objects

        # Cycle
        if (Pat.Cycle in ct and s.prev_pattern.column_count == 1
                and not (T == 8 and s.prev_pattern.any_column == 0)
                and not (T % 2 == 1 and s.prev_pattern.any_column == T // 2)):
            col = s.random_start + T - s.prev_pattern.any_column - 1
            pattern.add(col, self.ho.start_time, self.ho.start_time)
            s.prev_pattern = pattern
            return pattern.objects

        # ForceStack
        if Pat.ForceStack in ct and s.prev_pattern.column_count > 0:
            for c in range(s.random_start, T):
                if s.prev_pattern.has_column(c):
                    pattern.add(c, self.ho.start_time, self.ho.start_time)
            s.prev_pattern = pattern
            return pattern.objects

        # Stair (from previous)
        if s.prev_pattern.column_count == 1:
            last_col = s.prev_pattern.any_column
            if Pat.Stair in ct:
                col = last_col + 1
                if col >= T:
                    col = s.random_start
                pattern.add(col, self.ho.start_time, self.ho.start_time)
                if col == T - 1:
                    s.stair_type = Pat.ReverseStair
                s.prev_pattern = pattern
                return pattern.objects
            if Pat.ReverseStair in ct:
                col = last_col - 1
                if col < s.random_start:
                    col = T - 1
                pattern.add(col, self.ho.start_time, self.ho.start_time)
                if col == s.random_start:
                    s.stair_type = Pat.Stair
                s.prev_pattern = pattern
                return pattern.objects

        # KeepSingle
        if Pat.KeepSingle in ct:
            self._gen_random_notes(pattern, 1)
            s.prev_pattern = pattern
            return pattern.objects

        # Mirror
        if Pat.Mirror in ct:
            self._gen_mirrored(pattern)
            s.prev_pattern = pattern
            return pattern.objects

        # Random with conversion difficulty
        cd = s.conv_diff
        lp = Pat.LowProbability in ct
        if cd > 6.5:
            p2, p3 = (0.78, 0.42) if lp else (1.0, 0.62)
        elif cd > 4:
            p2, p3 = (0.35, 0.08) if lp else (0.52, 0.15)
        elif cd > 2:
            p2, p3 = (0.18, 0) if lp else (0.45, 0)
        else:
            p2, p3 = 0.0, 0.0

        note_count = self._get_random_note_count(p2, p3)
        self._gen_random_notes(pattern, note_count)
        self._add_special_column(pattern)
        s.prev_pattern = pattern
        return pattern.objects

    def _gen_random_notes(self, pattern: _Pattern, count: int) -> None:
        s = self.s
        T = s.total_columns
        ct = self.conv_type
        allow_stack = Pat.ForceNotStack not in ct

        if not allow_stack:
            occupied = s.prev_pattern.column_count if s.prev_pattern else 0
            count = min(count, T - s.random_start - occupied)

        col = s.get_column(self.ho.x, allow_special=True)
        patterns = [pattern] if allow_stack else ([pattern] + ([s.prev_pattern] if s.prev_pattern else []))

        def next_circle_column(last: int) -> int:
            if Pat.Gathered in ct:
                last += 1
                if last == T:
                    last = s.random_start
                return last
            return s.get_random_column()

        for i in range(count):
            col = s.find_available_column(col, patterns, next_column=next_circle_column)
            pattern.add(col, self.ho.start_time, self.ho.start_time)

    def _gen_mirrored(self, pattern: _Pattern) -> None:
        s = self.s
        T = s.total_columns
        cd = s.conv_diff
        ct = self.conv_type

        if Pat.ForceNotStack in ct:
            if cd > 6.5:
                p2, p3, p4, p5 = 0.5 + 0.38 / 2, 0.38, (0.38 + 0.12) / 2, 0.12
            elif cd > 4:
                p2, p3, p4, p5 = 0.5 + 0.17 / 2, 0.17, 0.17 / 2, 0.0
            else:
                p2, p3, p4, p5 = 0.5, 0.0, 0.0, 0.0
            self._gen_random_notes(pattern, self._get_random_note_count(p2, p3, p4, p5))
            self._add_special_column(pattern)
            return

        centre_p = 0.12
        if cd > 6.5:
            p2, p3 = 0.38, 0.12
        elif cd > 4:
            p2, p3 = 0.17, 0.0
        else:
            p2, p3 = 0.0, 0.0

        # cap mirrored probabilities per key count
        if T == 2:
            centre_p, p2, p3 = 0, 0, 0
        elif T == 3:
            centre_p = min(centre_p, 0.03)
            p2 = p3 = 0
        elif T == 4:
            centre_p = 0
            p2 = 1 - max((1 - p2) * 2, 0.8)
        elif T == 5:
            centre_p = min(centre_p, 0.03)
            p3 = 0
        elif T == 6:
            centre_p = 0
            p2 = 1 - max((1 - p2) * 2, 0.5)
            p3 = 1 - max((1 - p3) * 2, 0.85)

        p2 = max(0, min(1, p2))
        p3 = max(0, min(1, p3))
        centre_val = s.rng.next_double()
        note_count = s.get_random_note_count(p2, p3)
        add_centre = T % 2 == 1 and note_count != 3 and centre_val > 1 - centre_p

        half = (T if T % 2 == 0 else T - 1) // 2
        col = s.get_random_column(hi=s.random_start + half)
        for _ in range(note_count):
            col = s.find_available_column(col, [pattern],
                                           hi=s.random_start + half)
            pattern.add(col, self.ho.start_time, self.ho.start_time)
            pattern.add(s.random_start + T - col - 1,
                        self.ho.start_time, self.ho.start_time)

        if add_centre:
            pattern.add(T // 2, self.ho.start_time, self.ho.start_time)

        self._add_special_column(pattern)

    def _add_special_column(self, pattern: _Pattern) -> None:
        if (
            self.s.random_start > 0
            and (self.ho.hitsound & HIT_CLAP)
            and (self.ho.hitsound & HIT_FINISH)
            and not pattern.has_column(0)
        ):
            pattern.add(0, self.ho.start_time, self.ho.start_time)

    def _cap_note_counts(self, p2: float, p3: float = 0.0,
                          p4: float = 0.0, p5: float = 0.0) -> tuple[float, ...]:
        T = self.s.total_columns
        if T == 2:
            return 0.0, 0.0, 0.0, 0.0
        if T == 3:
            return min(p2, 0.1), 0.0, 0.0, 0.0
        if T == 4:
            return min(p2, 0.23), min(p3, 0.04), 0.0, 0.0
        if T == 5:
            return p2, min(p3, 0.15), min(p4, 0.03), 0.0
        return p2, p3, p4, p5

    def _get_random_note_count(
        self,
        p2: float,
        p3: float = 0.0,
        p4: float = 0.0,
        p5: float = 0.0,
    ) -> int:
        p2, p3, p4, p5 = self._cap_note_counts(p2, p3, p4, p5)
        if self.ho.hitsound & HIT_CLAP:
            p2 = 1.0
        return self.s.get_random_note_count(p2, p3, p4, p5)

    def _beat_length_at(self, time: int) -> float:
        base = 500.0
        for tp in self.s.timing_points:
            if tp.uninherited and tp.time <= time:
                base = tp.beat_length
        return base


# ── Slider Generator ──────────────────────────────────────────────────────────

class _SliderGenerator:
    def __init__(self, s: _ConversionState, ho: StandardHitObject) -> None:
        self.s = s
        self.ho = ho
        self.start_time = ho.start_time
        self.spans = max(1, ho.slider_repeats)
        self.seg_dur = s._slider_segment_duration(ho)
        self.end_time = self.start_time + self.seg_dur * self.spans
        self.duration = self.end_time - self.start_time
        self.convert_type = Pat.NONE
        if not _kiai_at(ho.start_time, s.timing_points):
            self.convert_type = Pat.LowProbability

    def generate(self) -> list[ManiaHitObject]:
        if self.spans > 1:
            pattern = self._gen_multi_span()
        else:
            pattern = self._gen_single_span()

        return self._split_patterns(pattern)

    def _split_patterns(self, pattern: _Pattern) -> list[ManiaHitObject]:
        if len(pattern.objects) == 1:
            self.s.prev_pattern = pattern
            return pattern.objects

        intermediate = _Pattern()
        end_pattern = _Pattern()
        for obj in pattern.objects:
            if self.end_time != obj.end_time:
                intermediate.add(obj.lane, obj.start_time, obj.end_time)
            else:
                end_pattern.add(obj.lane, obj.start_time, obj.end_time)

        self.s.prev_pattern = end_pattern
        return intermediate.objects + end_pattern.objects

    def _gen_multi_span(self) -> _Pattern:
        T = self.s.total_columns
        seg = self.seg_dur

        if seg <= 90:
            return self._gen_holds(self.start_time, 1)
        if seg <= 120:
            self.convert_type |= Pat.ForceNotStack
            return self._gen_notes_no_stack(self.start_time, self.spans + 1)
        if seg <= 160:
            return self._gen_stair(self.start_time)
        if seg <= 200 and self.s.conv_diff > 3:
            return self._gen_random_multiple(self.start_time)

        if self.duration >= 4000:
            return self._gen_n_random_notes(self.start_time, 0.23, 0, 0)

        if seg > 400 and self.spans < T - 1 - self.s.random_start:
            return self._gen_tiled_holds(self.start_time)

        return self._gen_hold_and_normal(self.start_time)

    def _gen_single_span(self) -> _Pattern:
        T = self.s.total_columns
        cd = self.s.conv_diff
        lp = Pat.LowProbability in self.convert_type
        seg = self.seg_dur

        if seg <= 110:
            if self.s.prev_pattern.column_count < T:
                self.convert_type |= Pat.ForceNotStack
            else:
                self.convert_type &= ~Pat.ForceNotStack
            return self._gen_notes_no_stack(self.start_time, 1 if seg < 80 else 2)

        # 按 ConversionDifficulty 分级
        if cd > 6.5:
            p2, p3 = (0.78, 0.3) if lp else (0.85, 0.36)
        elif cd > 4:
            p2, p3 = (0.43, 0.08) if lp else (0.56, 0.18)
        elif cd > 2.5:
            p2, p3 = (0.3, 0) if lp else (0.37, 0.08)
        else:
            p2, p3 = (0.17, 0) if lp else (0.27, 0)

        p2, p3, _ = self._cap_hold_counts(p2, p3)
        return self._gen_n_random_notes(self.start_time, p2, p3, 0)

    def _gen_holds(self, start: int, count: int) -> _Pattern:
        s = self.s
        T = s.total_columns
        prev = s.prev_pattern
        usable = T - s.random_start - prev.column_count
        count = min(count, T - s.random_start)
        pattern = _Pattern()
        col = s.get_random_column()
        for _ in range(min(usable, count)):
            col = s.find_available_column(col, [pattern, prev])
            pattern.add(col, start, self.end_time)
        for _ in range(count - min(usable, count)):
            col = s.find_available_column(col, [pattern])
            pattern.add(col, start, self.end_time)
        return pattern

    def _gen_notes_no_stack(self, start: int, count: int) -> _Pattern:
        s = self.s
        T = s.total_columns
        prev = s.prev_pattern
        pattern = _Pattern()
        col = s.get_column(self.ho.x, allow_special=True)
        if Pat.ForceNotStack in self.convert_type and prev.column_count < T:
            col = s.find_available_column(col, [prev])

        last_col = col
        for i in range(count):
            t = int(start + i * self.seg_dur)
            pattern.add(col, t, t)
            col = s.find_available_column(col, [], validation=lambda c, old=last_col: c != old)
            last_col = col
        return pattern

    def _gen_stair(self, start: int) -> _Pattern:
        s = self.s
        T = s.total_columns
        col = s.get_column(self.ho.x, allow_special=True)
        increasing = s.rng.next_double() > 0.5
        pattern = _Pattern()
        for i in range(self.spans + 1):
            t = int(start + i * self.seg_dur)
            pattern.add(col, t, t)
            if increasing:
                if col >= T - 1:
                    increasing = False
                    col -= 1
                else:
                    col += 1
            else:
                if col <= s.random_start:
                    increasing = True
                    col += 1
                else:
                    col -= 1
        return pattern

    def _gen_random_multiple(self, start: int) -> _Pattern:
        s = self.s
        T = s.total_columns
        legacy = 4 <= T <= 8
        interval = s.rng.next(1, T - (1 if legacy else 0))
        col = s.get_column(self.ho.x, allow_special=True)
        pattern = _Pattern()
        for i in range(self.spans + 1):
            t = int(start + i * self.seg_dur)
            pattern.add(col, t, t)
            col2 = col + interval
            if col2 >= T - s.random_start:
                col2 = col2 - T - s.random_start + (1 if legacy else 0)
            col2 += s.random_start
            if T > 2:
                pattern.add(col2, t, t)
            col = s.get_random_column()
        return pattern

    def _gen_tiled_holds(self, start: int) -> _Pattern:
        s = self.s
        T = s.total_columns
        col_repeat = min(self.spans, T)
        col = s.get_column(self.ho.x, allow_special=True)
        if Pat.ForceNotStack in self.convert_type and s.prev_pattern.column_count < T:
            col = s.find_available_column(col, [s.prev_pattern])

        pattern = _Pattern()
        for i in range(col_repeat):
            t = int(start + i * self.seg_dur)
            col = s.find_available_column(col, [pattern])
            pattern.add(col, t, self.end_time)
        return pattern

    def _gen_hold_and_normal(self, start: int) -> _Pattern:
        s = self.s
        T = s.total_columns
        cd = s.conv_diff
        col = s.get_column(self.ho.x, allow_special=True)
        if Pat.ForceNotStack in self.convert_type and s.prev_pattern.column_count < T:
            col = s.find_available_column(col, [s.prev_pattern])

        pattern = _Pattern()
        pattern.add(col, start, self.end_time)  # hold

        # 伴随 note count
        if cd > 6.5:
            nc = s.get_random_note_count(0.63)
        elif cd > 4:
            nc = s.get_random_note_count(0.12 if T < 6 else 0.45)
        elif cd > 2.5:
            nc = s.get_random_note_count(0 if T < 6 else 0.24)
        else:
            nc = 0
        nc = min(T - 1, nc)

        next_col = s.get_random_column()
        row = _Pattern()
        ignore_head = not (self._hitsound_at(start) & (HIT_WHISTLE | HIT_FINISH | HIT_CLAP))

        for i in range(self.spans + 1):
            t = int(start + i * self.seg_dur)
            if not (ignore_head and t == self.start_time):
                for _ in range(nc):
                    next_col = s.find_available_column(next_col, [row], validation=lambda c: c != col)
                    row.add(next_col, t, t)
            for obj in row.objects:
                pattern.add(obj.lane, obj.start_time, obj.end_time)
            row = _Pattern()

        return pattern

    def _gen_n_random_notes(self, start: int, p2: float, p3: float,
                             p4: float = 0) -> _Pattern:
        can_generate_two_notes = Pat.LowProbability not in self.convert_type
        can_generate_two_notes = can_generate_two_notes and (
            bool(self.ho.hitsound & (HIT_CLAP | HIT_FINISH))
            or bool(self._hitsound_at(self.start_time) & (HIT_CLAP | HIT_FINISH))
        )
        if can_generate_two_notes:
            p2 = 1
        return self._gen_holds(start, self.s.get_random_note_count(p2, p3, p4))

    def _hitsound_at(self, time: int) -> int:
        if not self.ho.slider_edge_hitsounds:
            return self.ho.hitsound
        index = 0 if self.seg_dur == 0 else (time - self.start_time) // self.seg_dur
        index = max(0, min(index, len(self.ho.slider_edge_hitsounds) - 1))
        return self.ho.slider_edge_hitsounds[index]

    def _cap_hold_counts(self, p2: float, p3: float = 0.0,
                          p4: float = 0.0) -> tuple[float, ...]:
        T = self.s.total_columns
        if T == 2:
            return 0.0, 0.0, 0.0
        if T == 3:
            return min(p2, 0.1), 0.0, 0.0
        if T == 4:
            return min(p2, 0.3), min(p3, 0.04), 0.0
        if T == 5:
            return min(p2, 0.34), min(p3, 0.1), min(p4, 0.03)
        return p2, p3, p4


# ── Spinner Generator ─────────────────────────────────────────────────────────

class _SpinnerGenerator:
    def __init__(self, s: _ConversionState, ho: StandardHitObject) -> None:
        self.s = s
        self.ho = ho

    def generate(self) -> list[ManiaHitObject]:
        s = self.s
        T = s.total_columns
        dur = self.ho.end_time - self.ho.start_time
        is_hold = dur >= 100
        prev = s.prev_pattern
        force_not = prev.column_count < T

        if force_not:
            col = s.find_available_column(s.get_random_column(), [prev])
        else:
            col = s.get_random_column(0)

        end = self.ho.end_time if is_hold else self.ho.start_time
        pattern = _Pattern()
        pattern.add(col, self.ho.start_time, end)
        return pattern.objects
