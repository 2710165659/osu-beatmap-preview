use crate::domain::errors::{PreviewError, Result};
use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub struct ModSettings {
    pub speed_multiplier: f64,
    pub double_time: bool,
    pub half_time: bool,
    /// Nightcore：速度与 DT 同区间，但音乐音高固定 +1.5 倍，并叠加节拍鼓点。
    pub nightcore: bool,
    /// Daycore：速度与 HT 同区间，音乐音高固定 0.75 倍（没有鼓点）。
    pub daycore: bool,

    pub da_cs: Option<f64>,
    pub da_ar: Option<f64>,
    pub da_od: Option<f64>,
    pub da_hp: Option<f64>,

    pub easy: bool,
    pub hard_rock: bool,
    pub hidden: bool,
    pub flashlight: bool,
    pub traceable: bool,

    pub swap: bool,
    pub cs_override: bool,

    pub mania_keys: Option<i32>,
    pub mania_key_mods: Vec<i32>,
    pub dual_stage: bool,
    pub inverse: bool,
    pub hold_off: bool,

    pub tokens: Vec<String>,
}

impl ModSettings {
    pub fn new() -> Self {
        ModSettings {
            speed_multiplier: 1.0,
            ..Default::default()
        }
    }

    pub fn has_da(&self) -> bool {
        self.da_cs.is_some() || self.da_ar.is_some() || self.da_od.is_some() || self.da_hp.is_some()
    }

    /// 是否启用了任何「只改倍速」的 Mod（DT / HT / NC / DC）。
    pub fn has_rate_mod(&self) -> bool {
        self.double_time || self.half_time || self.nightcore || self.daycore
    }

    /// 音乐音高倍率：DT/HT 保持原音高，NC/DC 与游戏一致地固定为 1.5 / 0.75。
    ///
    /// 游戏里 NC 的 `Frequency` 取 `SpeedChange.Default`（1.5）、DC 取 0.75，与用户设置的
    /// 速度值无关；整体速度仍由 [`ModSettings::speed_multiplier`] 决定。
    pub fn music_pitch(&self) -> f64 {
        if self.nightcore {
            1.5
        } else if self.daycore {
            0.75
        } else {
            1.0
        }
    }

    pub fn has_any_mod(&self) -> bool {
        self.speed_multiplier != 1.0
            || self.has_da()
            || self.easy
            || self.hard_rock
            || self.hidden
            || self.flashlight
            || self.traceable
            || self.swap
            || self.cs_override
            || self.mania_keys.is_some()
            || self.dual_stage
            || self.inverse
            || self.hold_off
    }
}

pub fn parse_mods(mod_tokens: &[String]) -> Result<ModSettings> {
    let mut settings = ModSettings::new();
    if mod_tokens.is_empty() {
        return Ok(settings);
    }
    if mod_tokens.iter().any(|token| token.contains('+')) {
        return Err(PreviewError::new(
            "mods must be supplied as repeated --mod values; '+' is not allowed",
        ));
    }
    let tokens: Vec<String> = mod_tokens.iter().map(|t| t.trim().to_uppercase()).collect();
    if tokens.iter().any(|token| token.is_empty()) {
        return Err(PreviewError::new("mod tokens must not be empty"));
    }
    let mut seen = HashSet::new();
    if let Some(duplicate) = tokens.iter().find(|token| !seen.insert(token.as_str())) {
        return Err(PreviewError::new(format!(
            "duplicate mod token: '{duplicate}'"
        )));
    }
    settings.tokens = tokens.clone();
    for token in &tokens {
        parse_one_token(token, &mut settings)?;
    }
    Ok(settings)
}

fn parse_one_token(token: &str, s: &mut ModSettings) -> Result<()> {
    if let Some(tail) = token.strip_prefix("DA") {
        return parse_da_token(tail, s);
    }

    // DT/HT/NC/DC 可带可选速度值。
    if let Some((kind, raw_val)) = split_rate_mod(token) {
        // 加速类与减速类的取值范围来自游戏里各 Mod 的 SpeedChange 边界：
        // DT/NC 为 1.01–2.00，HT/DC 为 0.50–0.99。
        let (default, range) = if matches!(kind, "DT" | "NC") {
            (1.5, RATE_UP_RANGE)
        } else {
            (0.75, RATE_DOWN_RANGE)
        };
        let val = match raw_val {
            Some(raw) => parse_float(raw, token)?,
            None => default,
        };
        if !range.contains(&val) {
            let (min, max) = (*range.start(), *range.end());
            return Err(PreviewError::new(format!(
                "{kind} speed must be in [{}, {}], got {}",
                fmt_float(min),
                fmt_float(max),
                fmt_float(val)
            )));
        }
        s.speed_multiplier = val;
        match kind {
            "DT" => s.double_time = true,
            "HT" => s.half_time = true,
            "NC" => s.nightcore = true,
            "DC" => s.daycore = true,
            _ => unreachable!(),
        }
        return Ok(());
    }

    // <n>K。
    if let Some(num) = token.strip_suffix('K') {
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
            let keys: i32 = num
                .parse()
                .map_err(|_| PreviewError::new(format!("mania keys must be 1-10, got {num}")))?;
            if !MANIA_KEY_MOD_RANGE.contains(&keys) {
                return Err(PreviewError::new(format!(
                    "mania keys must be 1-10, got {keys}"
                )));
            }
            if s.mania_keys.is_none() {
                s.mania_keys = Some(keys);
            }
            s.mania_key_mods.push(keys);
            return Ok(());
        }
    }

    match token {
        "EZ" => s.easy = true,
        "HR" => s.hard_rock = true,
        "HD" => s.hidden = true,
        "FL" => s.flashlight = true,
        "TC" => s.traceable = true,
        "SW" => s.swap = true,
        "CS" => s.cs_override = true,
        "DS" => s.dual_stage = true,
        "IN" => s.inverse = true,
        "HO" => s.hold_off = true,
        _ => {
            return Err(PreviewError::new(format!(
                "unknown or unsupported mod token: '{token}'"
            )))
        }
    }
    Ok(())
}

/// DT/NC（加速类）与 HT/DC（减速类）允许的速度区间，来自游戏里各 Mod 的 SpeedChange 边界。
const RATE_UP_RANGE: std::ops::RangeInclusive<f64> = 1.01..=2.00;
const RATE_DOWN_RANGE: std::ops::RangeInclusive<f64> = 0.5..=0.99;

/// 拆出 DT/HT/NC/DC 的两字符前缀与可选速度后缀。
///
/// 前缀后面必须是空串或纯数字/小数点，否则返回 `None` 交给后面的分支报「未知 Mod」，
/// 这样 `DTX` 这类拼写错误不会被当成 DT。
fn split_rate_mod(token: &str) -> Option<(&'static str, Option<&str>)> {
    const KINDS: [&str; 4] = ["DT", "HT", "NC", "DC"];
    let kind = KINDS.into_iter().find(|kind| token.starts_with(kind))?;
    let rest = &token[kind.len()..];
    if rest.is_empty() {
        return Some((kind, None));
    }
    if rest.chars().all(|c| c.is_ascii_digit() || c == '.') {
        Some((kind, Some(rest)))
    } else {
        None
    }
}

fn parse_da_token(tail: &str, s: &mut ModSettings) -> Result<()> {
    let bytes = tail.as_bytes();
    let mut pos = 0;
    let mut matched = false;
    while pos < bytes.len() {
        let rest = &tail[pos..];
        let lower = rest.to_lowercase();
        let param = if lower.starts_with("ar") {
            "AR"
        } else if lower.starts_with("cs") {
            "CS"
        } else if lower.starts_with("od") {
            "OD"
        } else if lower.starts_with("hp") {
            "HP"
        } else {
            break;
        };
        // 数字部分：-?[\d.]+。
        let num_start = pos + 2;
        let mut num_end = num_start;
        let b = tail.as_bytes();
        if num_end < b.len() && b[num_end] == b'-' {
            num_end += 1;
        }
        let digits_start = num_end;
        while num_end < b.len() && (b[num_end].is_ascii_digit() || b[num_end] == b'.') {
            num_end += 1;
        }
        if num_end == digits_start {
            break;
        }
        matched = true;
        let val = parse_float(&tail[num_start..num_end], &format!("DA{tail}"))?;
        set_da_param(param, val, s)?;
        pos = num_end;
    }

    if !matched {
        return Err(PreviewError::new(format!(
            "DA mod requires at least one parameter (ar/cs/od/hp), got: '{tail}'"
        )));
    }
    if pos < tail.len() {
        return Err(PreviewError::new(format!(
            "unexpected content after DA params: '{}'",
            &tail[pos..]
        )));
    }
    Ok(())
}

fn set_da_param(param: &str, val: f64, s: &mut ModSettings) -> Result<()> {
    // 范围依赖最终目标模式，解析阶段只保存字段和值。
    match param {
        "CS" => s.da_cs = Some(val),
        "AR" => s.da_ar = Some(val),
        "OD" => s.da_od = Some(val),
        "HP" => s.da_hp = Some(val),
        _ => unreachable!(),
    }
    Ok(())
}

fn parse_float(raw: &str, token: &str) -> Result<f64> {
    let value = raw
        .parse::<f64>()
        .map_err(|_| PreviewError::new(format!("invalid numeric value in mod token: '{token}'")))?;
    if !value.is_finite() {
        return Err(PreviewError::new(format!(
            "numeric value in mod token must be finite: '{token}'"
        )));
    }
    Ok(value)
}

fn fmt_float(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e16 {
        format!("{:.1}", v)
    } else {
        format!("{}", v)
    }
}

pub fn validate_mods(settings: &ModSettings, mode: Option<i32>, fmt: Option<&str>) -> Vec<String> {
    let mut errors = Vec::new();

    if let Some(mode) = mode {
        errors.extend(validate_da_ranges(settings, mode));
    }

    if rate_mod_count(settings) > 1 {
        errors.push("only one of DT, HT, NC, DC may be used at a time".to_string());
    }
    if settings.easy && settings.hard_rock {
        errors.push("EZ and HR cannot be used together".to_string());
    }
    if settings.hidden && settings.traceable {
        errors.push("HD and TC cannot be used together".to_string());
    }
    if settings.mania_key_mods.len() > 1 {
        let keys: Vec<String> = settings
            .mania_key_mods
            .iter()
            .map(|k| format!("{k}K"))
            .collect();
        errors.push(format!(
            "mania key mods cannot be used together: {}",
            keys.join(", ")
        ));
    }

    if mode == Some(0) {
        if settings.has_da() && settings.easy {
            errors.push("DA and EZ cannot be used together".to_string());
        }
        if settings.has_da() && settings.hard_rock {
            errors.push("DA and HR cannot be used together".to_string());
        }
    }
    if mode == Some(3) && settings.inverse && settings.hold_off {
        errors.push("IN and HO cannot be used together".to_string());
    }
    if mode == Some(3) && settings.hidden && settings.flashlight {
        errors.push("HD and FL cannot be used together for mania".to_string());
    }

    if let (Some(mode), Some(fmt)) = (mode, fmt) {
        if (0..=3).contains(&mode) {
            errors.extend(validate_supported_mods(settings, mode, fmt));
        }
    }
    errors
}

/// 按 osu! 各规则集的 Extended Limits 校验 DA 数值。
///
/// DA 当前仍只在 Standard 渲染中生效；其余模式的 DA 会继续由支持矩阵拒绝。
/// 这里保留各规则集的范围，使模式相关校验不会错误套用 Standard 的 AR/OD 边界。
fn validate_da_ranges(settings: &ModSettings, mode: i32) -> Vec<String> {
    let mut errors = Vec::new();
    let checks = [
        ("CS", settings.da_cs, da_range(mode, "CS")),
        ("AR", settings.da_ar, da_range(mode, "AR")),
        ("OD", settings.da_od, da_range(mode, "OD")),
        ("HP", settings.da_hp, da_range(mode, "HP")),
    ];
    for (name, value, range) in checks {
        let (Some(value), Some((min, max))) = (value, range) else {
            continue;
        };
        if !(min..=max).contains(&value) {
            errors.push(format!(
                "DA {name} must be in [{}, {}], got {}",
                fmt_float(min),
                fmt_float(max),
                fmt_float(value)
            ));
        }
    }
    errors
}

fn da_range(mode: i32, param: &str) -> Option<(f64, f64)> {
    match (mode, param) {
        // osu!standard：CS、OD、HP 扩展到 11，AR 扩展到 -10..11。
        (0, "CS") | (0, "OD") | (0, "HP") => Some((0.0, 11.0)),
        (0, "AR") => Some((-10.0, 11.0)),
        // osu!taiko 的 DA 当前可对应 OD/HP；两者沿用通用 Extended Limits。
        (1, "OD") | (1, "HP") => Some((0.0, 11.0)),
        // osu!catch 的 CS/AR 不支持负值，其他当前字段沿用通用范围。
        (2, "CS") | (2, "AR") | (2, "OD") | (2, "HP") => Some((0.0, 11.0)),
        // osu!mania 的 OD 使用规则集专用 -15..15 扩展，HP 仍为 0..11。
        (3, "OD") => Some((-15.0, 15.0)),
        (3, "HP") => Some((0.0, 11.0)),
        _ => None,
    }
}

fn supported_switch_mods(fmt: &str, mode: i32) -> &'static [&'static str] {
    match (fmt, mode) {
        ("gif", 0) => &["EZ", "HR", "HD", "FL", "DA", "TC"],
        ("gif", 1) => &["EZ", "HR", "HD", "FL", "SW", "CS"],
        ("gif", 2) => &["EZ", "HR", "HD", "FL"],
        ("gif", 3) => &["K", "DS", "CS", "IN", "HO", "HD", "FL"],
        ("png", 0) => &["EZ", "HR", "HD", "FL", "DA", "TC"],
        ("png", 1) => &["EZ", "HR", "SW"],
        ("png", 2) => &["EZ", "HR"],
        ("png", 3) => &["K", "DS", "IN", "HO"],
        _ => &[],
    }
}

fn validate_supported_mods(settings: &ModSettings, mode: i32, fmt: &str) -> Vec<String> {
    let fmt_key = fmt.trim().to_lowercase();
    // MP4 与 GIF 使用相同模组规则（动画输出，允许 DT/HT）。
    let fmt_key = if fmt_key == "mp4" {
        "gif".to_owned()
    } else {
        fmt_key
    };
    if fmt_key != "gif" && fmt_key != "png" {
        return vec![format!("unknown output format: {fmt}")];
    }
    let mut errors = Vec::new();
    if fmt_key == "png" && settings.has_rate_mod() {
        errors.push("DT/HT/NC/DC are only supported for GIF output, not PNG".to_string());
    }
    let supported = supported_switch_mods(&fmt_key, mode);
    for (code, label) in active_switch_mods(settings) {
        if !supported.contains(&code.as_str()) {
            errors.push(format!(
                "{} is not supported for {} {} output",
                label,
                mode_label(mode),
                fmt_key.to_uppercase()
            ));
        }
    }
    errors
}

fn active_switch_mods(settings: &ModSettings) -> Vec<(String, String)> {
    let mut active = Vec::new();
    if settings.easy {
        active.push(("EZ".into(), "EZ".into()));
    }
    if settings.hard_rock {
        active.push(("HR".into(), "HR".into()));
    }
    if settings.hidden {
        active.push(("HD".into(), "HD".into()));
    }
    if settings.flashlight {
        active.push(("FL".into(), "FL".into()));
    }
    if settings.traceable {
        active.push(("TC".into(), "TC".into()));
    }
    if settings.has_da() {
        active.push(("DA".into(), "DA".into()));
    }
    if settings.swap {
        active.push(("SW".into(), "SW".into()));
    }
    if settings.cs_override {
        active.push(("CS".into(), "CS".into()));
    }
    if !settings.mania_key_mods.is_empty() {
        let label: Vec<String> = settings
            .mania_key_mods
            .iter()
            .map(|k| format!("{k}K"))
            .collect();
        active.push(("K".into(), label.join("+")));
    }
    if settings.dual_stage {
        active.push(("DS".into(), "DS".into()));
    }
    if settings.inverse {
        active.push(("IN".into(), "IN".into()));
    }
    if settings.hold_off {
        active.push(("HO".into(), "HO".into()));
    }
    active
}

fn mode_label(mode: i32) -> &'static str {
    match mode {
        0 => "std",
        1 => "taiko",
        2 => "catch",
        3 => "mania",
        _ => "mode ?",
    }
}

pub fn mods_for_mode(settings: &ModSettings, mode: i32) -> ModSettings {
    let mut filtered = ModSettings {
        speed_multiplier: settings.speed_multiplier,
        double_time: settings.double_time,
        half_time: settings.half_time,
        nightcore: settings.nightcore,
        daycore: settings.daycore,
        hidden: settings.hidden && (0..=3).contains(&mode),
        flashlight: settings.flashlight && (0..=3).contains(&mode),
        tokens: settings.tokens.clone(),
        ..ModSettings::new()
    };
    match mode {
        0 => {
            filtered.easy = settings.easy;
            filtered.hard_rock = settings.hard_rock;
            filtered.traceable = settings.traceable;
            filtered.da_cs = settings.da_cs;
            filtered.da_ar = settings.da_ar;
            filtered.da_od = settings.da_od;
            filtered.da_hp = settings.da_hp;
        }
        1 => {
            filtered.easy = settings.easy;
            filtered.hard_rock = settings.hard_rock;
            filtered.swap = settings.swap;
            filtered.cs_override = settings.cs_override;
        }
        2 => {
            filtered.easy = settings.easy;
            filtered.hard_rock = settings.hard_rock;
        }
        3 => {
            filtered.mania_keys = settings.mania_keys;
            filtered.mania_key_mods = settings.mania_key_mods.clone();
            filtered.dual_stage = settings.dual_stage;
            filtered.cs_override = settings.cs_override;
            filtered.inverse = settings.inverse;
            filtered.hold_off = settings.hold_off;
        }
        _ => {}
    }
    filtered
}

/// Mania 键数 Mod 的可选范围（`1K`–`10K`）；解析与面板展示共用同一处定义。
const MANIA_KEY_MOD_RANGE: std::ops::RangeInclusive<i32> = 1..=10;

/// 已启用的「只改倍速」Mod 数量；游戏的 `ModRateAdjust` 之间互斥，因此最多只能有一个。
fn rate_mod_count(settings: &ModSettings) -> usize {
    [
        settings.double_time,
        settings.half_time,
        settings.nightcore,
        settings.daycore,
    ]
    .iter()
    .filter(|active| **active)
    .count()
}

/// 实时预览 / 动画导出下某模式应当展示的可切换 Mod token 列表。
///
/// 顺序与 CLI README 的「GIF / MP4」列一致；键数 Mod 展开成 `1K`…`10K` 方便逐个渲染
/// 成按钮。每项都能被 `validate_mods(..., Some("mp4"))` 接受（`DA` 由调用方替换成带
/// 参数的 token）；网页端不再自带一份列表，避免两侧走偏。
pub fn supported_mod_tokens(mode: i32) -> Vec<String> {
    match mode {
        0 => ["EZ", "HR", "HD", "FL", "DA", "TC", "DT", "HT", "NC", "DC"]
            .map(str::to_string)
            .to_vec(),
        1 => ["EZ", "HR", "HD", "FL", "SW", "CS", "DT", "HT", "NC", "DC"]
            .map(str::to_string)
            .to_vec(),
        2 => ["EZ", "HR", "HD", "FL", "DT", "HT", "NC", "DC"]
            .map(str::to_string)
            .to_vec(),
        3 => {
            let mut tokens = ["HD", "FL", "CS", "DT", "HT", "NC", "DC"]
                .map(str::to_string)
                .to_vec();
            tokens.extend(MANIA_KEY_MOD_RANGE.map(|keys| format!("{keys}K")));
            tokens.extend(["DS", "IN", "HO"].map(str::to_string));
            tokens
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_mod_tokens_match_the_realtime_matrix() {
        for mode in 0..=3 {
            let tokens = supported_mod_tokens(mode);
            // 四种模式的实时链路都支持 HD/FL，面板必须能选到。
            for required in ["HD", "FL", "DT", "HT"] {
                assert!(
                    tokens.iter().any(|token| token == required),
                    "mode={mode} 缺少 {required}：{tokens:?}"
                );
            }
            // 列表里的每一项都必须能被实时会话接受；DA 由调用方补参数。
            for token in &tokens {
                let raw = if token == "DA" {
                    "DAAR9CS4".to_string()
                } else {
                    token.clone()
                };
                let settings =
                    parse_mods(std::slice::from_ref(&raw)).expect("面板 token 必须可解析");
                assert!(
                    validate_mods(&settings, Some(mode), Some("mp4")).is_empty(),
                    "mode={mode}, token={raw} 不被实时会话接受"
                );
            }
            // 反向：支持矩阵里的每一类可切换 Mod 都要出现在面板里（K 展开成 1K）。
            for &supported in supported_switch_mods("gif", mode) {
                let expected = if supported == "K" { "1K" } else { supported };
                assert!(
                    tokens.iter().any(|token| token == expected),
                    "mode={mode} 面板缺少支持矩阵里的 {supported}：{tokens:?}"
                );
            }
        }
        assert!(supported_mod_tokens(9).is_empty(), "未知模式不给面板项");
    }

    #[test]
    fn hidden_and_flashlight_support_all_animation_rulesets() {
        for mode in 0..=3 {
            for token in ["HD", "FL"] {
                let settings = parse_mods(&[token.into(), "DT".into()]).unwrap();
                for format in ["gif", "mp4"] {
                    assert!(
                        validate_mods(&settings, Some(mode), Some(format)).is_empty(),
                        "mode={mode}, token={token}, format={format}"
                    );
                }
                let filtered = mods_for_mode(&settings, mode);
                assert!(if token == "HD" {
                    filtered.hidden
                } else {
                    filtered.flashlight
                });
            }
        }
    }

    #[test]
    fn mania_hidden_and_flashlight_are_incompatible() {
        let settings = parse_mods(&["HD".into(), "FL".into()]).unwrap();
        for mode in 0..=3 {
            let errors = validate_mods(&settings, Some(mode), Some("gif"));
            if mode == 3 {
                assert!(errors
                    .iter()
                    .any(|error| error.contains("HD and FL cannot be used together for mania")));
            } else {
                assert!(errors.is_empty(), "mode={mode}: {errors:?}");
            }
        }
    }

    #[test]
    fn catch_hidden_is_supported_for_animations_and_preserved_by_filter() {
        for extra in [None, Some("EZ"), Some("HR"), Some("DT"), Some("HT")] {
            let mut tokens = vec!["HD".to_string()];
            if let Some(extra) = extra {
                tokens.push(extra.to_string());
            }
            let settings = parse_mods(&tokens).unwrap();
            for format in ["gif", "mp4"] {
                assert!(validate_mods(&settings, Some(2), Some(format)).is_empty());
            }
            assert!(mods_for_mode(&settings, 2).hidden);
        }
    }

    #[test]
    fn catch_hidden_remains_unsupported_for_static_charts() {
        let settings = parse_mods(&["HD".into()]).unwrap();
        assert!(validate_mods(&settings, Some(2), Some("png"))
            .iter()
            .any(|error| error.contains("HD is not supported for catch PNG")));
    }

    #[test]
    fn parses_mods_as_individual_tokens() {
        let tokens = vec!["hd".to_string(), "dt1.25".to_string()];
        let settings = parse_mods(&tokens).unwrap();
        assert_eq!(settings.tokens, vec!["HD", "DT1.25"]);
        assert!((settings.speed_multiplier - 1.25).abs() < f64::EPSILON);
    }

    /// NC/DC 与 DT/HT 共用语法：默认倍速、可带自定义倍速、区间按加速/减速两类。
    #[test]
    fn parses_nightcore_and_daycore() {
        let nc = parse_mods(&["nc".into()]).unwrap();
        assert!(nc.nightcore);
        assert!((nc.speed_multiplier - 1.5).abs() < f64::EPSILON);
        assert!((nc.music_pitch() - 1.5).abs() < f64::EPSILON);

        let nc_custom = parse_mods(&["NC1.4".into()]).unwrap();
        assert!(nc_custom.nightcore);
        assert!((nc_custom.speed_multiplier - 1.4).abs() < f64::EPSILON);
        // 音高与速度设置无关：始终是游戏里 NC 的默认 1.5 倍。
        assert!((nc_custom.music_pitch() - 1.5).abs() < f64::EPSILON);

        let dc = parse_mods(&["dc".into()]).unwrap();
        assert!(dc.daycore);
        assert!((dc.speed_multiplier - 0.75).abs() < f64::EPSILON);
        assert!((dc.music_pitch() - 0.75).abs() < f64::EPSILON);

        let dc_custom = parse_mods(&["dc0.6".into()]).unwrap();
        assert!((dc_custom.speed_multiplier - 0.6).abs() < f64::EPSILON);
        assert!((dc_custom.music_pitch() - 0.75).abs() < f64::EPSILON);

        // DT/HT 保调。
        assert!((parse_mods(&["dt".into()]).unwrap().music_pitch() - 1.0).abs() < f64::EPSILON);
        assert!((parse_mods(&["ht".into()]).unwrap().music_pitch() - 1.0).abs() < f64::EPSILON);
        assert!((ModSettings::new().music_pitch() - 1.0).abs() < f64::EPSILON);
    }

    /// NC/DC 的倍速区间与 DT/HT 一致，越界与拼写错误都要报错。
    #[test]
    fn rate_mod_ranges_are_enforced() {
        for token in ["NC1.0", "NC2.01", "DC0.49", "DC1.0"] {
            assert!(parse_mods(&[token.into()]).is_err(), "{token} 应被拒绝");
        }
        for token in ["NC1.01", "NC2.0", "DC0.5", "DC0.99"] {
            assert!(parse_mods(&[token.into()]).is_ok(), "{token} 应被接受");
        }
        // 前缀后面不是数字后缀时按「未知 Mod」处理，不会误判成 DT/NC。
        for token in ["DTX", "NCX", "DC1.5x"] {
            assert!(parse_mods(&[token.into()]).is_err(), "{token} 应被拒绝");
        }
    }

    /// DT/HT/NC/DC 四者互斥（游戏里同属 ModRateAdjust）。
    #[test]
    fn rate_mods_are_mutually_exclusive() {
        for tokens in [
            ["DT", "HT"],
            ["DT", "NC"],
            ["DT", "DC"],
            ["HT", "NC"],
            ["HT", "DC"],
            ["NC", "DC"],
        ] {
            let settings = parse_mods(&tokens.map(str::to_string)).expect("单独都能解析");
            let errors = validate_mods(&settings, Some(0), Some("gif"));
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("only one of DT, HT, NC, DC")),
                "{tokens:?}: {errors:?}"
            );
        }
        // 与其它类别的 Mod 可以共存。
        let settings = parse_mods(&["NC".into(), "HD".into(), "HR".into()]).unwrap();
        assert!(validate_mods(&settings, Some(0), Some("gif")).is_empty());
    }

    /// NC/DC 与 DT/HT 一样只支持动画输出。
    #[test]
    fn rate_mods_are_rejected_for_png() {
        for token in ["DT", "HT", "NC", "DC"] {
            let settings = parse_mods(&[token.into()]).unwrap();
            assert!(
                validate_mods(&settings, Some(0), Some("png"))
                    .iter()
                    .any(|error| error.contains("only supported for GIF output")),
                "{token} 在 PNG 下应被拒绝"
            );
        }
    }

    /// 模式过滤保留 NC/DC 的标记与倍速。
    #[test]
    fn mode_filter_keeps_rate_mods() {
        let settings = parse_mods(&["NC1.3".into()]).unwrap();
        for mode in 0..=3 {
            let filtered = mods_for_mode(&settings, mode);
            assert!(filtered.nightcore, "mode={mode}");
            assert!(!filtered.daycore);
            assert!((filtered.speed_multiplier - 1.3).abs() < f64::EPSILON);
            assert!((filtered.music_pitch() - 1.5).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn rejects_plus_joined_mod_values() {
        let tokens = vec!["hd+hr".to_string()];
        assert!(parse_mods(&tokens).is_err());
    }

    #[test]
    fn rejects_empty_and_duplicate_mod_tokens() {
        assert!(parse_mods(&[String::new()]).is_err());
        assert!(parse_mods(&["HD".into(), "hd".into()]).is_err());
    }

    #[test]
    fn standard_da_accepts_extended_limits() {
        let settings = parse_mods(&["DAcs11ar-10od11hp11".into()]).unwrap();
        assert!(validate_mods(&settings, Some(0), Some("gif")).is_empty());
    }

    #[test]
    fn standard_da_rejects_values_outside_extended_limits() {
        for token in ["DAAR-10.1", "DACS11.1", "DAOD-0.1", "DAHP11.1"] {
            let settings = parse_mods(&[token.into()]).unwrap();
            assert!(validate_mods(&settings, Some(0), Some("gif"))
                .iter()
                .any(|error| error.starts_with("DA ")));
        }
    }

    #[test]
    fn mania_da_uses_extended_overall_difficulty_limits() {
        for value in ["-15", "15"] {
            let settings = parse_mods(&[format!("DAOD{value}")]).unwrap();
            assert!(validate_mods(&settings, Some(3), Some("gif"))
                .iter()
                .all(|error| !error.starts_with("DA OD must be")));
        }

        let settings = parse_mods(&["DAOD15.1".into()]).unwrap();
        assert!(validate_mods(&settings, Some(3), Some("gif"))
            .iter()
            .any(|error| error.starts_with("DA OD must be")));
    }

    #[test]
    fn catch_da_approach_rate_does_not_allow_negative_values() {
        let settings = parse_mods(&["DAAR-1".into()]).unwrap();
        assert!(validate_mods(&settings, Some(2), Some("gif"))
            .iter()
            .any(|error| error.starts_with("DA AR must be")));
    }

    #[test]
    fn da_rejects_non_finite_values_during_parsing() {
        assert!(parse_mods(&["DAARNaN".into()]).is_err());
        assert!(parse_mods(&["DAARinf".into()]).is_err());
    }

    #[test]
    fn da_remains_unsupported_outside_standard() {
        let settings = parse_mods(&["DAOD15".into()]).unwrap();
        let errors = validate_mods(&settings, Some(3), Some("gif"));
        assert!(errors
            .iter()
            .any(|error| error.contains("DA is not supported for mania GIF output")));
    }
}
