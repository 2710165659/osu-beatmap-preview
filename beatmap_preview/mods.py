from __future__ import annotations

import re
from dataclasses import dataclass, field

from .errors import PreviewError

# ── supported mod tokens (upper-cased) ──────────────────────────────────────

_SIMPLE_MODS = frozenset({"EZ", "HR", "HD", "SW", "CS", "DS", "IN", "HO"})

_SUPPORTED_SWITCH_MODS: dict[str, dict[int, frozenset[str]]] = {
    "gif": {
        0: frozenset({"EZ", "HR", "HD", "DA"}),
        1: frozenset({"EZ", "HR", "SW", "CS"}),
        2: frozenset({"EZ", "HR"}),
        3: frozenset({"K", "DS", "CS", "IN", "HO"}),
    },
    "png": {
        0: frozenset({"EZ", "HR", "HD", "DA"}),
        1: frozenset({"EZ", "HR", "SW"}),
        2: frozenset({"EZ", "HR"}),
        3: frozenset({"K", "DS", "IN", "HO"}),
    },
}

# DA 参数范围
_DA_CS_MIN, _DA_CS_MAX = 0.0, 11.0
_DA_OD_MIN, _DA_OD_MAX = 0.0, 11.0
_DA_HP_MIN, _DA_HP_MAX = 0.0, 11.0
_DA_AR_MIN, _DA_AR_MAX = -10.0, 11.0

# 模式名称 → mode id
_MODE_NAME_MAP: dict[str, int] = {
    "standard": 0,
    "taiko": 1,
    "catch": 2,
    "ctb": 2,
    "mania": 3,
}


@dataclass
class ModSettings:
    """解析后的 mod 设置。"""

    # ── 速率 mod ──
    speed_multiplier: float = 1.0
    double_time: bool = False
    half_time: bool = False

    # ── DA 参数 ──
    da_cs: float | None = None
    da_ar: float | None = None
    da_od: float | None = None
    da_hp: float | None = None

    # ── 通用开关 mod ──
    easy: bool = False
    hard_rock: bool = False
    hidden: bool = False

    # ── taiko 专用 ──
    swap: bool = False       # SW – taiko 红蓝互换
    cs_override: bool = False  # CS – taiko/mania 整体缩放

    # ── mania 专用 ──
    mania_keys: int | None = None  # 1-10
    mania_key_mods: list[int] = field(default_factory=list)
    dual_stage: bool = False  # DS
    inverse: bool = False    # IN
    hold_off: bool = False   # HO

    # ── 原始 token 列表 (调试 / 透传) ──
    tokens: list[str] = field(default_factory=list)

    def has_da(self) -> bool:
        """是否启用了 DA 且至少设置了一个参数。"""
        return (
            self.da_cs is not None
            or self.da_ar is not None
            or self.da_od is not None
            or self.da_hp is not None
        )

    def has_any_mod(self) -> bool:
        """是否有任何 mod 被激活（速率变化也算）。"""
        return (
            self.speed_multiplier != 1.0
            or self.has_da()
            or self.easy
            or self.hard_rock
            or self.hidden
            or self.swap
            or self.cs_override
            or self.mania_keys is not None
            or self.dual_stage
            or self.inverse
            or self.hold_off
        )


# ── 公开 API ────────────────────────────────────────────────────────────────


def parse_mods(mod_str: str) -> ModSettings:
    """解析 mod 字符串，如 ``"HR+HD+DT2+DAar0od10"`` → :class:`ModSettings`。

    大小写不敏感，内部统一归一化为大写 token 名。
    """
    if not mod_str or not mod_str.strip():
        return ModSettings()

    tokens = [t.strip().upper() for t in mod_str.split("+") if t.strip()]
    settings = ModSettings(tokens=tokens)

    for token in tokens:
        _parse_one_token(token, settings)

    return settings


def validate_mods(
    settings: ModSettings,
    mode: int | None = None,
    fmt: str | None = None,
) -> list[str]:
    """校验 mod 组合是否合法，返回冲突描述列表（空列表 = 无冲突）。"""
    errors: list[str] = []

    # ── 通用冲突 ──
    if settings.double_time and settings.half_time:
        errors.append("DT and HT cannot be used together")

    if settings.easy and settings.hard_rock:
        errors.append("EZ and HR cannot be used together")

    if len(settings.mania_key_mods) > 1:
        keys = ", ".join(f"{key}K" for key in settings.mania_key_mods)
        errors.append(f"mania key mods cannot be used together: {keys}")

    # ── std 特有 ──
    if mode == 0:
        if settings.has_da() and settings.easy:
            errors.append("DA and EZ cannot be used together")
        if settings.has_da() and settings.hard_rock:
            errors.append("DA and HR cannot be used together")

    # ── mania 特有 ──
    if mode == 3 and settings.inverse and settings.hold_off:
        errors.append("IN and HO cannot be used together")

    if mode in (0, 1, 2, 3) and fmt is not None:
        errors.extend(_validate_supported_mods(settings, mode, fmt))

    return errors


def mods_for_mode(settings: ModSettings, mode: int) -> ModSettings:
    """根据游戏模式过滤掉不支持的 mod（保留支持的）。"""
    filtered = ModSettings(
        speed_multiplier=settings.speed_multiplier,
        double_time=settings.double_time,
        half_time=settings.half_time,
        tokens=list(settings.tokens),
    )

    if mode == 0:  # standard
        _copy_std_mods(settings, filtered)
    elif mode == 1:  # taiko
        _copy_taiko_mods(settings, filtered)
    elif mode == 2:  # catch
        _copy_catch_mods(settings, filtered)
    elif mode == 3:  # mania
        _copy_mania_mods(settings, filtered)

    return filtered


def mode_name_to_id(name: str) -> int:
    """将模式名称（standard/taiko/ctb/catch/mania）转为 mode id。"""
    key = name.lower().strip()
    if key not in _MODE_NAME_MAP:
        raise PreviewError(
            f"unknown mode name: '{name}', expected one of {sorted(_MODE_NAME_MAP)}"
        )
    return _MODE_NAME_MAP[key]


# ── 内部解析 ─────────────────────────────────────────────────────────────────


def _parse_one_token(token: str, s: ModSettings) -> None:
    upper = token.upper()

    # -- DA + 参数：DAar0od10cs5...
    if upper.startswith("DA"):
        _parse_da_token(token[2:], s)  # 用原始大小写保留参数名
        return

    # -- DT / HT + 可选速率：dt1.5, ht0.8, dt2, ht1
    m = re.match(r"^(DT|HT)([\d.]+)?$", upper)
    if m:
        kind = m.group(1)
        raw_val = m.group(2)
        if kind == "DT":
            val = _parse_float(raw_val, token) if raw_val else 1.5
            if not (1.01 <= val <= 2.00):
                raise PreviewError("DT speed must be in [1.01, 2.0], got " + str(val))
            s.speed_multiplier = val
            s.double_time = True
        else:
            val = _parse_float(raw_val, token) if raw_val else 0.75
            if not (0.5 <= val <= 0.99):
                raise PreviewError("HT speed must be in [0.5, 0.99], got " + str(val))
            s.speed_multiplier = val
            s.half_time = True
        return

    # -- mania key 数量：1K - 10K
    m = re.match(r"^(\d+)K$", upper)
    if m:
        keys = int(m.group(1))
        if keys < 1 or keys > 10:
            raise PreviewError(f"mania keys must be 1-10, got {keys}")
        if s.mania_keys is None:
            s.mania_keys = keys
        s.mania_key_mods.append(keys)
        return

    # -- 简单开关 mod
    if upper in _SIMPLE_MODS:
        _apply_simple_mod(upper, s)
        return

    raise PreviewError(f"unknown or unsupported mod token: '{token}'")


def _apply_simple_mod(upper: str, s: ModSettings) -> None:
    if upper == "EZ":
        s.easy = True
    elif upper == "HR":
        s.hard_rock = True
    elif upper == "HD":
        s.hidden = True
    elif upper == "SW":
        s.swap = True
    elif upper == "CS":
        s.cs_override = True
    elif upper == "DS":
        s.dual_stage = True
    elif upper == "IN":
        s.inverse = True
    elif upper == "HO":
        s.hold_off = True


def _parse_da_token(tail: str, s: ModSettings) -> None:
    """解析 DA 后面的参数对，如 ``ar0od10cs5.5``。"""
    pattern = re.compile(r"(ar|cs|od|hp)(-?[\d.]+)", re.IGNORECASE)
    pos = 0
    matched = False
    while pos < len(tail):
        m = pattern.match(tail, pos)
        if not m:
            break
        matched = True
        param = m.group(1).upper()
        val = _parse_float(m.group(2), f"DA{tail}")
        _set_da_param(param, val, s)
        pos = m.end()

    if not matched:
        raise PreviewError(
            f"DA mod requires at least one parameter (ar/cs/od/hp), got: '{tail}'"
        )

    # 剩余的字符是非法的
    if pos < len(tail):
        leftover = tail[pos:]
        raise PreviewError(f"unexpected content after DA params: '{leftover}'")


def _set_da_param(param: str, val: float, s: ModSettings) -> None:
    if param == "CS":
        if not (_DA_CS_MIN <= val <= _DA_CS_MAX):
            raise PreviewError(f"DA CS must be in [{_DA_CS_MIN}, {_DA_CS_MAX}], got {val}")
        s.da_cs = val
    elif param == "AR":
        if not (_DA_AR_MIN <= val <= _DA_AR_MAX):
            raise PreviewError(f"DA AR must be in [{_DA_AR_MIN}, {_DA_AR_MAX}], got {val}")
        s.da_ar = val
    elif param == "OD":
        if not (_DA_OD_MIN <= val <= _DA_OD_MAX):
            raise PreviewError(f"DA OD must be in [{_DA_OD_MIN}, {_DA_OD_MAX}], got {val}")
        s.da_od = val
    elif param == "HP":
        if not (_DA_HP_MIN <= val <= _DA_HP_MAX):
            raise PreviewError(f"DA HP must be in [{_DA_HP_MIN}, {_DA_HP_MAX}], got {val}")
        s.da_hp = val


def _parse_float(raw: str, token: str) -> float:
    try:
        return float(raw)
    except ValueError:
        raise PreviewError(f"invalid numeric value in mod token: '{token}'")


def _validate_supported_mods(settings: ModSettings, mode: int, fmt: str) -> list[str]:
    fmt_key = fmt.lower().strip()
    if fmt_key not in _SUPPORTED_SWITCH_MODS:
        return [f"unknown output format: {fmt}"]

    errors: list[str] = []
    if fmt_key == "png" and (settings.double_time or settings.half_time):
        errors.append("DT/HT are only supported for GIF output, not PNG")

    supported = _SUPPORTED_SWITCH_MODS[fmt_key][mode]
    for code, label in _active_switch_mods(settings):
        if code not in supported:
            errors.append(f"{label} is not supported for {_mode_label(mode)} {fmt_key.upper()} output")

    return errors


def _active_switch_mods(settings: ModSettings) -> list[tuple[str, str]]:
    active: list[tuple[str, str]] = []
    if settings.easy:
        active.append(("EZ", "EZ"))
    if settings.hard_rock:
        active.append(("HR", "HR"))
    if settings.hidden:
        active.append(("HD", "HD"))
    if settings.has_da():
        active.append(("DA", "DA"))
    if settings.swap:
        active.append(("SW", "SW"))
    if settings.cs_override:
        active.append(("CS", "CS"))
    if settings.mania_key_mods:
        active.append(("K", "+".join(f"{key}K" for key in settings.mania_key_mods)))
    if settings.dual_stage:
        active.append(("DS", "DS"))
    if settings.inverse:
        active.append(("IN", "IN"))
    if settings.hold_off:
        active.append(("HO", "HO"))
    return active


def _mode_label(mode: int) -> str:
    if mode == 0:
        return "std"
    if mode == 1:
        return "taiko"
    if mode == 2:
        return "catch"
    if mode == 3:
        return "mania"
    return f"mode {mode}"


# ── 模式过滤辅助 ─────────────────────────────────────────────────────────────


def _copy_std_mods(src: ModSettings, dst: ModSettings) -> None:
    dst.easy = src.easy
    dst.hard_rock = src.hard_rock
    dst.hidden = src.hidden
    dst.da_cs = src.da_cs
    dst.da_ar = src.da_ar
    dst.da_od = src.da_od
    dst.da_hp = src.da_hp


def _copy_taiko_mods(src: ModSettings, dst: ModSettings) -> None:
    dst.easy = src.easy
    dst.hard_rock = src.hard_rock
    dst.swap = src.swap
    dst.cs_override = src.cs_override


def _copy_catch_mods(src: ModSettings, dst: ModSettings) -> None:
    dst.easy = src.easy
    dst.hard_rock = src.hard_rock


def _copy_mania_mods(src: ModSettings, dst: ModSettings) -> None:
    dst.mania_keys = src.mania_keys
    dst.mania_key_mods = list(src.mania_key_mods)
    dst.dual_stage = src.dual_stage
    dst.cs_override = src.cs_override
    dst.inverse = src.inverse
    dst.hold_off = src.hold_off
