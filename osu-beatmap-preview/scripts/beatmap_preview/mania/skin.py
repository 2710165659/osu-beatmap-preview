from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from .config import (
    GIF_HIT_TARGET_FROM_BOTTOM,
    LANE_BACKGROUND,
    LANE_WIDTH,
)


@dataclass(frozen=True)
class ManiaSkinConfig:
    keys: int
    hit_position: float
    column_widths: tuple[int, ...]
    column_line_widths: tuple[int, ...]
    column_colours: tuple[tuple[int, int, int, int], ...]
    hold_colour: tuple[int, int, int, int]


def load_mania_skin_config(keys: int) -> ManiaSkinConfig:
    """读取指定键数的 mania skin 配置；缺失时回退到内置默认值。"""
    configs = _load_all_mania_configs()
    config = configs.get(keys)
    if config is not None:
        return config
    return _default_config(keys)


def _load_all_mania_configs() -> dict[int, ManiaSkinConfig]:
    skin_path = Path(__file__).resolve().parents[3] / "assets" / "mania" / "skin.ini"
    if not skin_path.exists():
        return {}

    # skin.ini 里可能有多个 [Mania] 块，这里只按块收集键值对，不解析其它模式配置。
    blocks: list[dict[str, str]] = []
    current: dict[str, str] | None = None
    for raw_line in skin_path.read_text(encoding="utf-8-sig").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("//") or set(line) == {"="}:
            continue
        if line.startswith("[") and line.endswith("]"):
            if current is not None:
                blocks.append(current)
            current = {} if line[1:-1].strip().lower() == "mania" else None
            continue
        if current is None or ":" not in line:
            continue
        key, value = line.split(":", 1)
        current[key.strip()] = value.strip()
    if current is not None:
        blocks.append(current)

    result: dict[int, ManiaSkinConfig] = {}
    for block in blocks:
        config = _parse_block(block)
        if config is not None:
            result[config.keys] = config
    return result


def _parse_block(block: dict[str, str]) -> ManiaSkinConfig | None:
    try:
        keys = int(block["Keys"])
    except (KeyError, ValueError):
        return None

    # 只保留会影响矩形预览视觉的字段：判定线位置、列宽、列线宽、列底色、hold 底色。
    column_widths = _parse_int_list(block.get("ColumnWidth", ""), keys, LANE_WIDTH)
    column_line_widths = _parse_int_list(block.get("ColumnLineWidth", ""), keys + 1, 0)
    column_colours = tuple(
        _parse_colour(block.get(f"Colour{index + 1}", ""), LANE_BACKGROUND)
        for index in range(keys)
    )
    hold_colour = _parse_colour(block.get("ColourHold", ""), (255, 255, 255, 255))
    hit_position = _parse_hit_position(block.get("HitPosition"))

    return ManiaSkinConfig(
        keys=keys,
        hit_position=hit_position,
        column_widths=column_widths,
        column_line_widths=column_line_widths,
        column_colours=column_colours,
        hold_colour=hold_colour,
    )


def _parse_int_list(raw: str, count: int, default: int) -> tuple[int, ...]:
    values: list[int] = []
    for part in raw.split(","):
        part = part.strip()
        if not part:
            continue
        try:
            values.append(max(0, round(float(part))))
        except ValueError:
            continue

    if not values:
        values = [default]
    if len(values) < count:
        values.extend([values[-1]] * (count - len(values)))
    return tuple(values[:count])


def _parse_colour(raw: str, fallback: tuple[int, int, int, int]) -> tuple[int, int, int, int]:
    values: list[int] = []
    for part in raw.split(","):
        part = part.strip()
        if not part:
            continue
        try:
            values.append(max(0, min(255, round(float(part)))))
        except ValueError:
            return fallback
    if len(values) == 3:
        values.append(255)
    if len(values) != 4:
        return fallback
    return tuple(values)  # type: ignore[return-value]


def _parse_hit_position(raw: str | None) -> float:
    if raw is None:
        return float(GIF_HIT_TARGET_FROM_BOTTOM)
    try:
        legacy_position = max(240.0, min(480.0, float(raw)))
    except ValueError:
        return float(GIF_HIT_TARGET_FROM_BOTTOM)
    # osu! stable 的 HitPosition 是 480 高坐标，GIF 按 768 高坐标换算为离底部距离。
    return (480.0 - legacy_position) * 1.6


def _default_config(keys: int) -> ManiaSkinConfig:
    return ManiaSkinConfig(
        keys=keys,
        hit_position=float(GIF_HIT_TARGET_FROM_BOTTOM),
        column_widths=tuple(LANE_WIDTH for _ in range(keys)),
        column_line_widths=tuple(0 for _ in range(keys + 1)),
        column_colours=tuple(LANE_BACKGROUND for _ in range(keys)),
        hold_colour=(255, 255, 255, 255),
    )
