from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from scripts.beatmap_preview.errors import PreviewError
from scripts.beatmap_preview.mods import parse_mods, validate_mods
from scripts.beatmap_preview.service import generate_preview


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Download an osu! beatmap and render a preview image."
    )
    parser.add_argument(
        "--bid", required=True, type=str, help="osu! beatmap id (numeric)"
    )
    parser.add_argument(
        "--convert",
        choices=["mania", "ctb", "taiko"],
        default=None,
        help="convert osu!standard beatmap to another mode",
    )
    parser.add_argument(
        "--mod", "--mods",
        dest="mod",
        type=str,
        default=None,
        help="mod string, e.g. HR+HD+DT2+DAar0od10 (case-insensitive)",
    )
    parser.add_argument(
        "--fmt", "--format",
        choices=["png", "gif"],
        default=None,
        help="output format (default: gif for standard, png for others)",
    )
    parser.add_argument(
        "--time", "--times",
        dest="time",
        type=str,
        default=None,
        help="time points in seconds for GIF preview, joined by '+', e.g. 10+20+30",
    )
    return parser


def _parse_times(raw: str) -> list[float]:
    """将 ``"10+20+30"`` 解析为 ``[10.0, 20.0, 30.0]``。"""
    parts = [p.strip() for p in raw.split("+") if p.strip()]
    if len(parts) > 4:
        raise PreviewError("--time accepts at most 4 time points")

    result: list[float] = []
    for p in parts:
        try:
            val = float(p)
        except ValueError:
            raise PreviewError(f"invalid time value: '{p}'")
        if val < 0:
            raise PreviewError(f"time must be non-negative, got {val}")
        result.append(val)
    return result


def main() -> int:
    parser = _build_parser()
    args = parser.parse_args()

    try:
        # ── 解析 & 校验 mods ──
        mod_settings = None
        if args.mod:
            mod_settings = parse_mods(args.mod)
            errors = validate_mods(mod_settings)
            if errors:
                raise PreviewError("mod conflict: " + "; ".join(errors))

        # ── 解析 times ──
        times = None
        if args.time:
            if args.fmt == "png":
                raise PreviewError("--time is only valid for GIF output")
            times = _parse_times(args.time)

        result = generate_preview(
            args.bid,
            fmt=args.fmt,
            convert=args.convert,
            mods=mod_settings,
            times=times,
        )
    except PreviewError as exc:
        payload = json.dumps(
            {
                "status": "error",
                "msg": str(exc),
                "preview-img": "",
                "beatmap-info": {},
            },
            ensure_ascii=False,
            indent=4,
        )
        sys.stdout.buffer.write((payload + "\n").encode("utf-8"))
        return 1

    payload = json.dumps(result, ensure_ascii=False, indent=4)
    sys.stdout.buffer.write((payload + "\n").encode("utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
