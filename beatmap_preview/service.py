from __future__ import annotations

import re
import tempfile
from pathlib import Path

from .composer import save_animated_gif, save_png
from .downloader import download_beatmap_file
from .errors import PreviewError
from .models import Beatmap
from .mods import ModSettings, mods_for_mode, validate_mods
from .parser import parse_beatmap
from .standard.renderer import render_standard
from .taiko.renderer import render_taiko_grid
from .catch.renderer import render_catch_grid
from .mania.renderer import render_mania_grid


def generate_preview(
    bid: str,
    fmt: str | None = None,
    convert: str | None = None,
    mods: ModSettings | None = None,
    times: list[float] | None = None,
) -> dict[str, object]:
    if not bid.isdigit():
        raise PreviewError("bid must be numeric")

    temp_root = Path(tempfile.gettempdir()) / "osu-beatmap-preview"
    beatmap_path = download_beatmap_file(bid=bid, temp_dir=temp_root / "osu-download-cache")
    beatmap = parse_beatmap(beatmap_path)

    # ── 模式转换 ──
    target_mode = beatmap.mode
    if convert:
        from .convert import resolve_convert_target

        target_mode = resolve_convert_target(beatmap, convert)

    # ── 解析 fmt 默认值 ──
    if fmt is None:
        fmt = "gif" if target_mode == 0 else "png"
    if times is not None and fmt != "gif":
        raise PreviewError("--times is only valid for GIF output")

    # ── mod 校验 ──
    if mods is not None and mods.has_any_mod():
        mode_errors = validate_mods(mods, target_mode, fmt)
        if mode_errors:
            raise PreviewError("mod conflict: " + "; ".join(mode_errors))
        mods = mods_for_mode(mods, target_mode)
    else:
        mods = None

    parts = [bid]
    if convert:
        parts.append(convert)
    if mods is not None and mods.has_any_mod():
        parts.append(_format_mod_suffix(mods))
    output_path = temp_root / "outputs" / f"{'_'.join(parts)}.{fmt}"

    preview_path = _render_preview_for_mode(
        beatmap, output_path, fmt, target_mode, mods, times
    )

    return {
        "status": "success",
        "msg": f"preview generated successfully for bid {bid}",
        "preview-img": str(preview_path.resolve()),
        "beatmap-info": {
            "meta-data": _format_section_keys(beatmap.metadata),
            "difficulty": _format_section_keys(beatmap.difficulty),
        },
    }


def _format_section_keys(section: dict[str, str]) -> dict[str, str]:
    return {
        re.sub(
            r"([A-Z]+)([A-Z][a-z])", r"\1-\2",
            re.sub(r"([a-z0-9])([A-Z])", r"\1-\2", key),
        ).lower(): value
        for key, value in section.items()
    }


def _format_mod_suffix(mods: ModSettings) -> str:
    tokens = [token.strip().lower() for token in mods.tokens if token.strip()]
    if not tokens:
        return "mod"
    return "mod-" + "-".join(re.sub(r"[^a-z0-9.-]+", "", token) for token in tokens if token)


def _render_preview_for_mode(
    beatmap: Beatmap,
    output_path: Path,
    fmt: str,
    target_mode: int,
    mods: ModSettings | None,
    times: list[float] | None,
) -> Path:
    if target_mode == 0:
        from .models import StandardHitObject
        hit_objects = [ho for ho in beatmap.hit_objects if isinstance(ho, StandardHitObject)]
        if not hit_objects:
            raise PreviewError("standard beatmap has no hit objects")
        result = render_standard(beatmap, hit_objects, fmt, mods=mods, times=times)
        if fmt == "gif":
            frames, frame_duration_ms, loop = result
            save_animated_gif(frames, output_path, frame_duration_ms, loop)
        else:
            save_png(result, output_path)
        return output_path

    if target_mode == 1:
        if beatmap.mode != 1:
            from .convert import convert_beatmap
            beatmap = convert_beatmap(beatmap, target_mode, mods)
        if fmt == "gif":
            from .taiko.gif_renderer import render_taiko_gif

            frames, frame_duration_ms, loop = render_taiko_gif(beatmap, mods=mods, times=times)
            save_animated_gif(frames, output_path, frame_duration_ms, loop)
            return output_path
        return render_taiko_grid(beatmap, output_path, mods=mods)

    if target_mode == 2:
        if beatmap.mode != 2:
            from .convert import convert_beatmap
            beatmap = convert_beatmap(beatmap, target_mode, mods)
        if fmt == "gif":
            from .catch.gif_renderer import render_catch_gif

            frames, frame_duration_ms, loop = render_catch_gif(beatmap, mods=mods, times=times)
            save_animated_gif(frames, output_path, frame_duration_ms, loop)
            return output_path
        return render_catch_grid(beatmap, output_path, mods=mods)

    if target_mode == 3:
        if beatmap.mode != 3:
            from .convert import convert_beatmap
            beatmap = convert_beatmap(beatmap, target_mode, mods)
        if fmt == "gif":
            from .mania.gif_renderer import render_mania_gif

            frames, frame_duration_ms, loop = render_mania_gif(beatmap, mods=mods, times=times)
            save_animated_gif(frames, output_path, frame_duration_ms, loop)
            return output_path
        return render_mania_grid(beatmap, output_path, mods=mods)

    raise PreviewError(f"unsupported beatmap mode: {target_mode}")
