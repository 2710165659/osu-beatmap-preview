use crate::errors::{PreviewError, Result};

#[derive(Debug, Clone, Default)]
pub struct ModSettings {
    pub speed_multiplier: f64,
    pub double_time: bool,
    pub half_time: bool,

    pub da_cs: Option<f64>,
    pub da_ar: Option<f64>,
    pub da_od: Option<f64>,
    pub da_hp: Option<f64>,

    pub easy: bool,
    pub hard_rock: bool,
    pub hidden: bool,

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

    pub fn has_any_mod(&self) -> bool {
        self.speed_multiplier != 1.0
            || self.has_da()
            || self.easy
            || self.hard_rock
            || self.hidden
            || self.swap
            || self.cs_override
            || self.mania_keys.is_some()
            || self.dual_stage
            || self.inverse
            || self.hold_off
    }
}

pub fn parse_mods(mod_str: &str) -> Result<ModSettings> {
    let mut settings = ModSettings::new();
    if mod_str.trim().is_empty() {
        return Ok(settings);
    }
    let tokens: Vec<String> = mod_str
        .split('+')
        .map(|t| t.trim().to_uppercase())
        .filter(|t| !t.is_empty())
        .collect();
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

    // DT/HT with optional speed value
    if token.starts_with("DT") || token.starts_with("HT") {
        let (kind, rest) = token.split_at(2);
        if rest.is_empty() || rest.chars().all(|c| c.is_ascii_digit() || c == '.') {
            let raw_val = if rest.is_empty() { None } else { Some(rest) };
            if kind == "DT" {
                let val = match raw_val {
                    Some(r) => parse_float(r, token)?,
                    None => 1.5,
                };
                if !(1.01..=2.00).contains(&val) {
                    return Err(PreviewError::new(format!(
                        "DT speed must be in [1.01, 2.0], got {}",
                        fmt_float(val)
                    )));
                }
                s.speed_multiplier = val;
                s.double_time = true;
            } else {
                let val = match raw_val {
                    Some(r) => parse_float(r, token)?,
                    None => 0.75,
                };
                if !(0.5..=0.99).contains(&val) {
                    return Err(PreviewError::new(format!(
                        "HT speed must be in [0.5, 0.99], got {}",
                        fmt_float(val)
                    )));
                }
                s.speed_multiplier = val;
                s.half_time = true;
            }
            return Ok(());
        }
    }

    // <n>K
    if let Some(num) = token.strip_suffix('K') {
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
            let keys: i32 = num
                .parse()
                .map_err(|_| PreviewError::new(format!("mania keys must be 1-10, got {num}")))?;
            if !(1..=10).contains(&keys) {
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
        // numeric part: -?[\d.]+
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
    let check = |min: f64, max: f64, name: &str| -> Result<()> {
        if val < min || val > max {
            Err(PreviewError::new(format!(
                "DA {name} must be in [{}, {}], got {}",
                fmt_float(min),
                fmt_float(max),
                fmt_float(val)
            )))
        } else {
            Ok(())
        }
    };
    match param {
        "CS" => {
            check(0.0, 11.0, "CS")?;
            s.da_cs = Some(val);
        }
        "AR" => {
            check(-10.0, 11.0, "AR")?;
            s.da_ar = Some(val);
        }
        "OD" => {
            check(0.0, 11.0, "OD")?;
            s.da_od = Some(val);
        }
        "HP" => {
            check(0.0, 11.0, "HP")?;
            s.da_hp = Some(val);
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn parse_float(raw: &str, token: &str) -> Result<f64> {
    raw.parse::<f64>()
        .map_err(|_| PreviewError::new(format!("invalid numeric value in mod token: '{token}'")))
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

    if settings.double_time && settings.half_time {
        errors.push("DT and HT cannot be used together".to_string());
    }
    if settings.easy && settings.hard_rock {
        errors.push("EZ and HR cannot be used together".to_string());
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

    if let (Some(mode), Some(fmt)) = (mode, fmt) {
        if (0..=3).contains(&mode) {
            errors.extend(validate_supported_mods(settings, mode, fmt));
        }
    }
    errors
}

fn supported_switch_mods(fmt: &str, mode: i32) -> &'static [&'static str] {
    match (fmt, mode) {
        ("gif", 0) => &["EZ", "HR", "HD", "DA"],
        ("gif", 1) => &["EZ", "HR", "SW", "CS"],
        ("gif", 2) => &["EZ", "HR"],
        ("gif", 3) => &["K", "DS", "CS", "IN", "HO"],
        ("png", 0) => &["EZ", "HR", "HD", "DA"],
        ("png", 1) => &["EZ", "HR", "SW"],
        ("png", 2) => &["EZ", "HR"],
        ("png", 3) => &["K", "DS", "IN", "HO"],
        _ => &[],
    }
}

fn validate_supported_mods(settings: &ModSettings, mode: i32, fmt: &str) -> Vec<String> {
    let fmt_key = fmt.trim().to_lowercase();
    if fmt_key != "gif" && fmt_key != "png" {
        return vec![format!("unknown output format: {fmt}")];
    }
    let mut errors = Vec::new();
    if fmt_key == "png" && (settings.double_time || settings.half_time) {
        errors.push("DT/HT are only supported for GIF output, not PNG".to_string());
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
        tokens: settings.tokens.clone(),
        ..ModSettings::new()
    };
    match mode {
        0 => {
            filtered.easy = settings.easy;
            filtered.hard_rock = settings.hard_rock;
            filtered.hidden = settings.hidden;
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

