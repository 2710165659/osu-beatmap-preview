from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from PIL import Image

TAIKO_ASSET_DIR = Path(__file__).resolve().parents[3] / "assets" / "taiko"

_skin_singleton: TaikoSkin | None = None


@dataclass(frozen=True)
class TaikoSkin:
    bar_left: Image.Image
    bar_right: Image.Image
    bar_line: Image.Image
    hit_circle: Image.Image
    hit_circle_overlay: Image.Image
    big_hit_circle: Image.Image
    big_hit_circle_overlay: Image.Image
    roll_middle: Image.Image
    roll_end: Image.Image
    spinner_warning: Image.Image


def load_taiko_skin() -> TaikoSkin:
    global _skin_singleton
    if _skin_singleton is not None:
        return _skin_singleton
    required_assets = {
        "bar_left": "taiko-bar-left@2x.png",
        "bar_right": "taiko-bar-right@2x.png",
        "bar_line": "taiko-barline@2x.png",
        "hit_circle": "taikohitcircle@2x.png",
        "hit_circle_overlay": "taikohitcircleoverlay-0@2x.png",
        "big_hit_circle": "taikobigcircle@2x.png",
        "big_hit_circle_overlay": "taikobigcircleoverlay-0@2x.png",
        "roll_middle": "taiko-roll-middle@2x.png",
        "roll_end": "taiko-roll-end@2x.png",
        "spinner_warning": "spinner-warning@2x.png",
    }
    images: dict[str, Image.Image] = {}
    for key, filename in required_assets.items():
        asset_path = TAIKO_ASSET_DIR / filename
        image = Image.open(asset_path).convert("RGBA")
        if key in {"roll_middle", "roll_end"}:
            image = image.crop(image.getchannel("A").getbbox())
        images[key] = image

    _skin_singleton = TaikoSkin(**images)
    return _skin_singleton
