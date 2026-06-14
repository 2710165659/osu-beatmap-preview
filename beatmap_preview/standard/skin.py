from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from PIL import Image

STANDARD_ASSET_DIR = Path(__file__).resolve().parents[2] / "assets" / "standard"


@dataclass(frozen=True)
class StandardSkin:
    hitcircle: Image.Image
    hitcircle_overlay: Image.Image
    approachcircle: Image.Image
    reverse_arrow: Image.Image
    slider_ball: Image.Image
    slider_follow_circle: Image.Image
    spinner_circle: Image.Image
    digits: dict[str, Image.Image]
    hitcircle_overlap: int
    combo_colors: list[tuple[int, int, int]]
    slider_border: tuple[int, int, int]
    slider_track: tuple[int, int, int]


_skin_singleton: StandardSkin | None = None


def load_standard_skin() -> StandardSkin:
    global _skin_singleton
    if _skin_singleton is not None:
        return _skin_singleton
    skin_config = _parse_skin_config(STANDARD_ASSET_DIR / "skin.ini")
    combo_colors = _parse_combo_colors(skin_config)
    _skin_singleton = StandardSkin(
        hitcircle=Image.open(STANDARD_ASSET_DIR / "hitcircle@2x.png").convert("RGBA"),
        hitcircle_overlay=Image.open(STANDARD_ASSET_DIR / "hitcircleoverlay@2x.png").convert("RGBA"),
        approachcircle=Image.open(STANDARD_ASSET_DIR / "approachcircle@2x.png").convert("RGBA"),
        reverse_arrow=Image.open(STANDARD_ASSET_DIR / "reversearrow@2x.png").convert("RGBA"),
        slider_ball=Image.open(STANDARD_ASSET_DIR / "sliderb0@2x.png").convert("RGBA"),
        slider_follow_circle=Image.open(STANDARD_ASSET_DIR / "sliderfollowcircle@2x.png").convert("RGBA"),
        spinner_circle=Image.open(STANDARD_ASSET_DIR / "spinner-circle@2x.png").convert("RGBA"),
        digits={
            str(digit): Image.open(STANDARD_ASSET_DIR / f"Fonts/default-{digit}@2x.png").convert("RGBA")
            for digit in range(10)
        },
        hitcircle_overlap=int(skin_config["HitCircleOverlap"]),
        combo_colors=combo_colors,
        slider_border=_parse_rgb(skin_config["SliderBorder"]),
        slider_track=_parse_rgb(skin_config["SliderTrackOverride"]),
    )
    return _skin_singleton


def _parse_skin_config(skin_path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw_line in skin_path.read_text(encoding="utf-8-sig").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("//") or line.startswith("[") or ":" not in line:
            continue
        key, value = line.split(":", 1)
        values[key.strip()] = value.strip()
    return values


def _parse_combo_colors(skin_config: dict[str, str]) -> list[tuple[int, int, int]]:
    colors: list[tuple[int, int, int]] = []
    index = 1
    while f"Combo{index}" in skin_config:
        colors.append(_parse_rgb(skin_config[f"Combo{index}"]))
        index += 1
    return colors


def _parse_rgb(value: str) -> tuple[int, int, int]:
    red, green, blue = [int(channel.strip()) for channel in value.split(",")]
    return red, green, blue
