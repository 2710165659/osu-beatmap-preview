from __future__ import annotations

from pathlib import Path

from PIL import Image


def save_animated_gif(
    frames_iter,
    output_path: Path,
    frame_duration_ms: int,
    loop: int,
) -> None:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    first = next(frames_iter)
    first.save(
        output_path,
        save_all=True,
        append_images=frames_iter,
        duration=frame_duration_ms,
        loop=loop,
        optimize=True,
        disposal=2,
    )


def save_png(image: Image.Image, output_path: Path) -> None:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    image.convert("RGB").save(output_path, optimize=True)
