from __future__ import annotations

from pathlib import Path

from .errors import PreviewError
from .models import (
    Beatmap,
    BreakPeriod,
    CatchHitObject,
    HitObject,
    ManiaHitObject,
    StandardHitObject,
    TaikoHitObject,
    TimingPoint,
)


def parse_beatmap(beatmap_path: Path) -> Beatmap:
    try:
        content = beatmap_path.read_text(encoding="utf-8-sig")
        sections = _split_sections(content)

        # .osu 文件把 Metadata、Difficulty 等内容拆在不同 section 中；后续解析都基于这些分组。
        metadata = _parse_key_value_section(sections, "Metadata") if "Metadata" in sections else _default_metadata()
        difficulty = _parse_key_value_section(sections, "Difficulty")
        general = _parse_key_value_section(sections, "General") if "General" in sections else {"Mode": "0"}
        general["FormatVersion"] = str(_parse_format_version(content))
        timing_points = _parse_timing_points(sections)
        break_periods = _parse_break_periods(sections)
        mode = int(general["Mode"])

        # 各 mode 的 HitObjects 字段含义不同，先分派到专门解析函数再统一排序。
        if mode == 0:
            hit_objects = _parse_standard_hit_objects(sections, difficulty, timing_points)
        elif mode == 1:
            hit_objects = _parse_taiko_hit_objects(sections, difficulty, timing_points)
        elif mode == 2:
            hit_objects = _parse_catch_hit_objects(sections, difficulty, timing_points)
        elif mode == 3:
            hit_objects = _parse_mania_hit_objects(sections, difficulty, timing_points)
        else:
            raise PreviewError(f"Unsupported beatmap mode: {mode}")

        return Beatmap(
            metadata=metadata,
            difficulty=difficulty,
            general=general,
            timing_points=timing_points,
            hit_objects=hit_objects,
            break_periods=break_periods,
        )
    except PreviewError:
        raise
    except (OSError, UnicodeError, KeyError, ValueError, IndexError, ZeroDivisionError) as exc:
        raise PreviewError("Failed to parse beatmap.") from exc


def _split_sections(content: str) -> dict[str, list[str]]:
    sections: dict[str, list[str]] = {}
    current_section = ""

    for raw_line in content.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("//"):
            continue

        if line.startswith("[") and line.endswith("]"):
            current_section = line[1:-1]
            sections[current_section] = []
            continue

        if not current_section:
            continue
        sections[current_section].append(line)

    return sections


def _parse_format_version(content: str) -> int:
    lines = content.splitlines()
    first_line = lines[0].strip() if lines else ""
    if first_line.startswith("osu file format v"):
        return int(first_line.rsplit("v", 1)[1])
    return 14


def _parse_key_value_section(sections: dict[str, list[str]], section_name: str) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in sections[section_name]:
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        values[key.strip()] = value.strip()

    return values


def _default_metadata() -> dict[str, str]:
    return {
        "Title": "Unknown",
        "TitleUnicode": "",
        "Artist": "Unknown",
        "ArtistUnicode": "",
        "Creator": "Unknown",
        "Version": "Unknown",
    }


def _parse_timing_points(sections: dict[str, list[str]]) -> list[TimingPoint]:
    timing_points: list[TimingPoint] = []
    for line in sections["TimingPoints"]:
        parts = [part.strip() for part in line.split(",")]
        if len(parts) < 2:
            continue

        meter = int(parts[2]) if len(parts) > 2 and parts[2] else 4
        if meter <= 0:
            meter = 4
        # 第 7 列为 1 或缺失时是红线 BPM；0 则是 inherited velocity。
        uninherited = len(parts) < 7 or parts[6] == "1"
        # 第 8 列 effects 的最低位表示 kiai 是否开启。
        effects = int(parts[7]) if len(parts) > 7 and parts[7] else 0
        timing_points.append(
            TimingPoint(
                time=float(parts[0]),
                beat_length=float(parts[1]),
                meter=meter,
                uninherited=uninherited,
                kiai_mode=bool(effects & 1),
            )
        )

    # 同一时间点的红线 / 绿线顺序会影响后续 timing 计算，必须保留文件里的原始顺序。
    return sorted(timing_points, key=lambda point: point.time)


def _parse_break_periods(sections: dict[str, list[str]]) -> list[BreakPeriod]:
    if "Events" not in sections:
        return []

    break_periods: list[BreakPeriod] = []
    for line in sections["Events"]:
        parts = [part.strip() for part in line.split(",")]
        if len(parts) < 3 or parts[0] != "2":
            continue

        start_time = int(float(parts[1]))
        end_time = int(float(parts[2]))
        if end_time > start_time:
            break_periods.append(BreakPeriod(start_time=start_time, end_time=end_time))

    return break_periods


def _parse_standard_hit_objects(
    sections: dict[str, list[str]],
    difficulty: dict[str, str],
    timing_points: list[TimingPoint],
) -> list[HitObject]:
    hit_objects: list[HitObject] = []
    for line in sections["HitObjects"]:
        parts = [part.strip() for part in line.split(",")]
        if len(parts) < 5:
            continue

        x = int(parts[0])
        y = int(parts[1])
        start_time = int(parts[2])
        hit_type = int(parts[3])
        hitsound = int(parts[4])
        end_time = _parse_object_end_time(parts, start_time, hit_type, difficulty, timing_points)
        new_combo = bool(hit_type & 4)
        combo_offset = (hit_type & 112) >> 4
        slider_type = None
        slider_points: tuple[tuple[int, int], ...] = ()
        slider_repeats = 1
        slider_pixel_length = 0.0
        slider_edge_hitsounds: tuple[int, ...] = ()

        if hit_type & 2:
            # Slider 专有字段从第 6 列开始：类型、控制点、重复次数和像素长度。
            slider_parts = parts[5].split("|")
            slider_type = slider_parts[0]
            slider_points = tuple(
                (int(point.split(":", 1)[0]), int(point.split(":", 1)[1]))
                for point in slider_parts[1:]
            )
            slider_repeats = int(parts[6])
            slider_pixel_length = float(parts[7])
            if len(parts) > 8 and parts[8]:
                slider_edge_hitsounds = tuple(int(value) for value in parts[8].split("|") if value)

        hit_objects.append(
            StandardHitObject(
                x=x,
                y=y,
                start_time=start_time,
                end_time=end_time,
                hit_type=hit_type,
                hitsound=hitsound,
                new_combo=new_combo,
                combo_offset=combo_offset,
                slider_type=slider_type,
                slider_points=slider_points,
                slider_repeats=slider_repeats,
                slider_pixel_length=slider_pixel_length,
                slider_edge_hitsounds=slider_edge_hitsounds,
            )
        )

    return _sort_hit_objects(hit_objects)


def _parse_taiko_hit_objects(
    sections: dict[str, list[str]],
    difficulty: dict[str, str],
    timing_points: list[TimingPoint],
) -> list[HitObject]:
    hit_objects: list[HitObject] = []
    for line in sections["HitObjects"]:
        parts = [part.strip() for part in line.split(",")]
        if len(parts) < 5:
            continue

        start_time = int(parts[2])
        hit_type = int(parts[3])
        hitsound = int(parts[4])
        end_time = _parse_object_end_time(parts, start_time, hit_type, difficulty, timing_points)

        hit_objects.append(
            TaikoHitObject(
                start_time=start_time,
                end_time=end_time,
                hit_type=hit_type,
                hitsound=hitsound,
            )
        )

    return _sort_hit_objects(hit_objects)


def _parse_catch_hit_objects(
    sections: dict[str, list[str]],
    difficulty: dict[str, str],
    timing_points: list[TimingPoint],
) -> list[HitObject]:
    hit_objects: list[HitObject] = []
    for line in sections["HitObjects"]:
        parts = [part.strip() for part in line.split(",")]
        if len(parts) < 5:
            continue

        x = int(parts[0])
        y = int(parts[1])
        start_time = int(parts[2])
        hit_type = int(parts[3])
        end_time = _parse_object_end_time(parts, start_time, hit_type, difficulty, timing_points)
        new_combo = bool(hit_type & 4)
        combo_offset = (hit_type & 112) >> 4
        slider_type = None
        slider_points: tuple[tuple[int, int], ...] = ()
        slider_repeats = 1
        slider_pixel_length = 0.0

        if hit_type & 2:
            slider_parts = parts[5].split("|")
            slider_type = slider_parts[0]
            slider_points = tuple(
                (int(point.split(":", 1)[0]), int(point.split(":", 1)[1]))
                for point in slider_parts[1:]
            )
            slider_repeats = int(parts[6])
            slider_pixel_length = float(parts[7])

        hit_objects.append(
            CatchHitObject(
                x=x,
                y=y,
                start_time=start_time,
                end_time=end_time,
                hit_type=hit_type,
                new_combo=new_combo,
                combo_offset=combo_offset,
                slider_type=slider_type,
                slider_points=slider_points,
                slider_repeats=slider_repeats,
                slider_pixel_length=slider_pixel_length,
            )
        )

    return _sort_hit_objects(hit_objects)


def _parse_mania_hit_objects(
    sections: dict[str, list[str]],
    difficulty: dict[str, str],
    timing_points: list[TimingPoint],
) -> list[HitObject]:
    key_count = int(float(difficulty["CircleSize"]))
    hit_objects: list[HitObject] = []
    for line in sections["HitObjects"]:
        parts = [part.strip() for part in line.split(",")]
        if len(parts) < 5:
            continue

        x = int(parts[0])
        start_time = int(parts[2])
        hit_type = int(parts[3])
        # mania 使用 x 坐标按键数等分出 lane，避免 renderer 再依赖原始坐标。
        lane = max(0, min(key_count - 1, x * key_count // 512))
        is_long_note = bool(hit_type & 128)
        end_time = start_time
        if is_long_note:
            end_time = int(parts[5].split(":", 1)[0])

        hit_objects.append(
            ManiaHitObject(
                lane=lane,
                start_time=start_time,
                end_time=end_time,
                is_long_note=is_long_note,
            )
        )

    return _sort_hit_objects(hit_objects)


def _parse_object_end_time(
    parts: list[str],
    start_time: int,
    hit_type: int,
    difficulty: dict[str, str],
    timing_points: list[TimingPoint],
) -> int:
    # osu! hit_type 位标志：8=spinner，2=slider；普通物件结束时间就是开始时间。
    if hit_type & 8:
        return int(parts[5])
    if hit_type & 2:
        return _parse_slider_end_time(parts, start_time, difficulty, timing_points)
    return start_time


def _parse_slider_end_time(
    parts: list[str],
    start_time: int,
    difficulty: dict[str, str],
    timing_points: list[TimingPoint],
) -> int:
    # Slider 时长由当前红线 BPM 和最近的 inherited velocity 共同决定。
    slides = int(parts[6])
    pixel_length = float(parts[7])
    slider_multiplier = float(difficulty["SliderMultiplier"])
    beat_length, slider_velocity = _resolve_slider_timing(start_time, timing_points)
    duration = pixel_length / (slider_multiplier * 100 * slider_velocity) * beat_length * slides
    return start_time + round(duration)


def _resolve_slider_timing(start_time: int, timing_points: list[TimingPoint]) -> tuple[float, float]:
    beat_length = timing_points[0].beat_length
    slider_velocity = 1.0

    # 扫到物件开始时间前的最后有效 timing：红线更新 BPM，绿线更新 slider velocity。
    for point in timing_points:
        if point.time > start_time:
            break
        if point.uninherited:
            beat_length = point.beat_length
            slider_velocity = 1.0
        elif point.beat_length < 0:
            slider_velocity = -100 / point.beat_length

    return beat_length, slider_velocity


def _sort_hit_objects(hit_objects: list[HitObject]) -> list[HitObject]:
    return sorted(hit_objects, key=lambda hit_object: (hit_object.start_time, hit_object.end_time))
