from __future__ import annotations

from .errors import PreviewError
from .models import Beatmap
from .mods import ModSettings

_MODE_NAME_MAP: dict[str, int] = {
    "taiko": 1,
    "ctb": 2,
    "catch": 2,
    "mania": 3,
}


def resolve_convert_target(beatmap: Beatmap, convert: str) -> int:
    """校验转换参数并返回目标 mode id。

    ``--convert`` 只对原始 osu!standard 谱面有效。
    """
    if beatmap.mode != 0:
        raise PreviewError(
            f"mode conversion (--convert) is only supported for osu!standard beatmaps, "
            f"current mode is {beatmap.mode}"
        )

    key = convert.lower().strip()
    if key not in _MODE_NAME_MAP:
        raise PreviewError(
            f"unknown convert target: '{convert}', "
            f"expected one of {sorted(_MODE_NAME_MAP)}"
        )

    return _MODE_NAME_MAP[key]


def convert_beatmap(
    beatmap: Beatmap,
    target_mode: int,
    mods: ModSettings | None = None,
) -> Beatmap:
    """将 osu!standard 谱面转换为另一模式。按 target_mode 分派到具体实现。"""
    if beatmap.mode != 0:
        raise PreviewError("source beatmap must be osu!standard (mode=0)")

    if target_mode == 3:
        from .mania.convert import convert_beatmap as _mania_convert
        return _mania_convert(beatmap, target_mode, mods)

    if target_mode == 1:
        from .taiko.convert import convert_beatmap as _taiko_convert
        return _taiko_convert(beatmap, target_mode, mods)

    if target_mode == 2:
        from .catch.convert import convert_beatmap as _catch_convert
        return _catch_convert(beatmap, target_mode, mods)

    raise PreviewError(f"conversion to mode {target_mode} is not yet implemented")
