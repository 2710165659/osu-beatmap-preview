from __future__ import annotations

from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from .errors import PreviewError


def download_beatmap_file(bid: str, temp_dir: Path) -> Path:
    temp_dir.mkdir(parents=True, exist_ok=True)
    target_path = temp_dir / f"{bid}.osu"
    if target_path.is_file() and target_path.stat().st_size > 0:
        return target_path
    request = Request(
        url=f"https://osu.ppy.sh/osu/{bid}",
        headers={"User-Agent": "osu-beatmap-preview/1.0"},
    )

    try:
        with urlopen(request, timeout=20) as response:
            data = response.read()
    except HTTPError as exc:
        if exc.code == 404:
            raise PreviewError(f"beatmap not found for bid {bid}") from exc
        raise PreviewError(f"failed to download beatmap {bid}: http {exc.code}") from exc
    except URLError as exc:
        raise PreviewError(f"failed to download beatmap {bid}: {exc.reason}") from exc

    target_path.write_bytes(data)
    return target_path
