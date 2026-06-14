from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class TimingPoint:
    """谱面时间点；同时保留红线/绿线和 kiai 开关信息。"""

    time: float
    beat_length: float
    meter: int
    uninherited: bool
    kiai_mode: bool


@dataclass(frozen=True)
class BreakPeriod:
    start_time: int
    end_time: int


@dataclass(frozen=True)
class StandardHitObject:
    """standard 物件保留原始 hit_type 和 hitsound，渲染/转谱阶段据此区分物件与采样。"""

    x: int
    y: int
    start_time: int
    end_time: int
    hit_type: int
    hitsound: int
    new_combo: bool
    combo_offset: int
    slider_type: str | None = None
    slider_points: tuple[tuple[int, int], ...] = ()
    slider_repeats: int = 1
    slider_pixel_length: float = 0.0
    slider_edge_hitsounds: tuple[int, ...] = ()


@dataclass(frozen=True)
class TaikoHitObject:
    start_time: int
    end_time: int
    hit_type: int
    hitsound: int


@dataclass(frozen=True)
class CatchHitObject:
    """catch 顶层物件；slider / spinner 额外字段保留给渲染转换阶段使用。"""

    x: int
    y: int
    start_time: int
    end_time: int
    hit_type: int
    new_combo: bool
    combo_offset: int
    slider_type: str | None = None
    slider_points: tuple[tuple[int, int], ...] = ()
    slider_repeats: int = 1
    slider_pixel_length: float = 0.0


@dataclass(frozen=True)
class ManiaHitObject:
    """mania 物件只需要轨道、起止时间和是否长条，x 坐标在解析阶段已映射为 lane。"""

    lane: int
    start_time: int
    end_time: int
    is_long_note: bool


# 解析器按 mode 产出不同物件，service 层统一通过 Beatmap.hit_objects 传给对应 renderer。
HitObject = StandardHitObject | TaikoHitObject | CatchHitObject | ManiaHitObject


@dataclass(frozen=True)
class Beatmap:
    metadata: dict[str, str]
    difficulty: dict[str, str]
    general: dict[str, str]
    timing_points: list[TimingPoint]
    hit_objects: list[HitObject]
    break_periods: list[BreakPeriod]

    @property
    def mode(self) -> int:
        return int(self.general["Mode"])

    @property
    def title(self) -> str:
        # osu! 客户端优先展示 Unicode 标题，缺失时回退到罗马字标题。
        if "TitleUnicode" in self.metadata and self.metadata["TitleUnicode"]:
            return self.metadata["TitleUnicode"]
        return self.metadata["Title"]

    @property
    def version(self) -> str:
        return self.metadata["Version"]
