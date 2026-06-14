from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from PIL import Image

CATCH_ASSET_DIR = Path(__file__).resolve().parents[2] / "assets" / "catch"

_skin_singleton: CatchSkin | None = None


@dataclass(frozen=True)
class CatchSkin:
    fruit_bases: dict[str, Image.Image]
    fruit_overlays: dict[str, Image.Image]
    droplet_base: Image.Image
    droplet_overlay: Image.Image
    banana_base: Image.Image
    banana_overlay: Image.Image
    catcher_idle: Image.Image
    combo_colors: list[tuple[int, int, int]]
    hyper_dash_color: tuple[int, int, int]
    hyper_dash_fruit_color: tuple[int, int, int]


def load_catch_skin() -> CatchSkin:
    global _skin_singleton
    if _skin_singleton is not None:
        return _skin_singleton
    skin_config = _parse_skin_config(CATCH_ASSET_DIR / "skin.ini")
    combo_colors = _parse_combo_colors(skin_config)
    _skin_singleton = CatchSkin(
        fruit_bases={
            "pear": Image.open(CATCH_ASSET_DIR / "fruit-pear@2x.png").convert("RGBA"),
            "grapes": Image.open(CATCH_ASSET_DIR / "fruit-grapes@2x.png").convert("RGBA"),
            "apple": Image.open(CATCH_ASSET_DIR / "fruit-apple@2x.png").convert("RGBA"),
            "orange": Image.open(CATCH_ASSET_DIR / "fruit-orange@2x.png").convert("RGBA"),
        },
        fruit_overlays={
            "pear": Image.open(CATCH_ASSET_DIR / "fruit-pear-overlay@2x.png").convert("RGBA"),
            "grapes": Image.open(CATCH_ASSET_DIR / "fruit-grapes-overlay@2x.png").convert("RGBA"),
            "apple": Image.open(CATCH_ASSET_DIR / "fruit-apple-overlay@2x.png").convert("RGBA"),
            "orange": Image.open(CATCH_ASSET_DIR / "fruit-orange-overlay@2x.png").convert("RGBA"),
        },
        droplet_base=Image.open(CATCH_ASSET_DIR / "fruit-drop@2x.png").convert("RGBA"),
        droplet_overlay=Image.open(CATCH_ASSET_DIR / "fruit-drop-overlay@2x.png").convert("RGBA"),
        banana_base=Image.open(CATCH_ASSET_DIR / "fruit-bananas@2x.png").convert("RGBA"),
        banana_overlay=Image.open(CATCH_ASSET_DIR / "fruit-bananas-overlay@2x.png").convert("RGBA"),
        catcher_idle=Image.open(CATCH_ASSET_DIR / "fruit-catcher-idle-0@2x.png").convert("RGBA"),
        combo_colors=combo_colors,
        hyper_dash_color=_parse_rgb(skin_config["HyperDash"]),
        hyper_dash_fruit_color=_parse_rgb(skin_config["HyperDashFruit"]),
    )
    return _skin_singleton


def _parse_skin_config(skin_path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    current_section = ""
    for raw_line in skin_path.read_text(encoding="utf-8-sig").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("//"):
            continue
        if line.startswith("[") and line.endswith("]"):
            current_section = line[1:-1]
            continue
        if ":" not in line:
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
