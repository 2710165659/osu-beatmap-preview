"""standard -> catch 转谱，按 osu!lazer 的 CatchBeatmapConverter 移植。"""

from __future__ import annotations

from ..errors import PreviewError
from ..models import Beatmap, CatchHitObject, StandardHitObject
from ..mods import ModSettings


def convert_beatmap(
    beatmap: Beatmap,
    target_mode: int,
    mods: ModSettings | None = None,
) -> Beatmap:
    if beatmap.mode != 0:
        raise PreviewError("source beatmap must be osu!standard (mode=0)")
    if target_mode != 2:
        raise PreviewError("only catch (mode=2) conversion is supported here")

    std_objects = [ho for ho in beatmap.hit_objects if isinstance(ho, StandardHitObject)]
    if not std_objects:
        raise PreviewError("standard beatmap has no hit objects to convert")

    catch_objects = [_convert_hit_object(hit_object) for hit_object in std_objects]

    new_general = dict(beatmap.general)
    new_general["Mode"] = "2"

    # 和 lazer 一样，EZ/HR 等 difficulty mod 在 Convert() 后才应用。
    # 因此这里保留原 difficulty，让 renderer 使用目标 ruleset 的有效 difficulty。
    return Beatmap(
        metadata=dict(beatmap.metadata),
        difficulty=dict(beatmap.difficulty),
        general=new_general,
        timing_points=list(beatmap.timing_points),
        hit_objects=sorted(catch_objects, key=lambda ho: (ho.start_time, ho.end_time)),
        break_periods=list(beatmap.break_periods),
    )


def _convert_hit_object(hit_object: StandardHitObject) -> CatchHitObject:
    # CatchBeatmapConverter 顶层映射：
    # circle -> Fruit, slider -> JuiceStream, spinner -> BananaShower。
    # Python renderer 之后会按 hit_type 展开成 fruit / droplet / banana。
    return CatchHitObject(
        x=hit_object.x,
        y=hit_object.y,
        start_time=hit_object.start_time,
        end_time=hit_object.end_time,
        hit_type=hit_object.hit_type,
        new_combo=hit_object.new_combo,
        combo_offset=hit_object.combo_offset,
        slider_type=hit_object.slider_type,
        slider_points=hit_object.slider_points,
        slider_repeats=hit_object.slider_repeats,
        slider_pixel_length=hit_object.slider_pixel_length,
    )
