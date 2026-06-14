//! 统一皮肤配置模块。
//!
//! 所有模式的皮肤配置都从 `assets/skin.ini`（编译期内嵌）读取：
//! - `[Colours]`  → std / catch 的 combo 颜色、滑条边框与轨道颜色
//! - `[Fonts]`    → std 的 HitCircleOverlap（combo 数字重叠量）
//! - `[CatchTheBeat]` → catch 的 HyperDash 颜色
//! - `[Mania]`    → mania 各键数的列宽 / 判定线位置等（多段，按 Keys 区分）

use std::sync::OnceLock;

/// 编译期内嵌的皮肤配置文本。
static SKIN_INI: &str = include_str!("../assets/skin.ini");

/// 解析后的全局皮肤配置。
pub struct SkinConfig {
    /// `[Colours]` Combo1..ComboN（按编号排序）。
    pub combo_colors: Vec<[u8; 3]>,
    /// `[Fonts]` HitCircleOverlap，combo 数字之间的重叠量（相对数字高度的百分比基准）。
    pub hitcircle_overlap: i64,
    /// `[CatchTheBeat]` HyperDash 颜色（超冲水果的提示色）。
    pub hyper_dash: [u8; 3],
    /// `[Mania]` 原始键值块（每个 Keys 一块），由 mania 模块进一步解析。
    pub mania_blocks: Vec<Vec<(String, String)>>,
}

/// 获取全局皮肤配置（惰性解析，进程内只解析一次）。
pub fn skin() -> &'static SkinConfig {
    static CONFIG: OnceLock<SkinConfig> = OnceLock::new();
    CONFIG.get_or_init(parse_skin_ini)
}

/// 解析 skin.ini 全文。容忍 `//` 注释行与 `====` 分隔线。
fn parse_skin_ini() -> SkinConfig {
    let text = SKIN_INI.trim_start_matches('\u{feff}');

    // 先按节切分；[Mania] 可出现多次，需逐块保留
    let mut colours: Vec<(String, String)> = Vec::new();
    let mut fonts: Vec<(String, String)> = Vec::new();
    let mut catch_section: Vec<(String, String)> = Vec::new();
    let mut mania_blocks: Vec<Vec<(String, String)>> = Vec::new();

    enum Section {
        None,
        Colours,
        Fonts,
        Catch,
        Mania,
    }
    let mut current = Section::None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") || line.chars().all(|c| c == '=') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line[1..line.len() - 1].trim();
            current = match name.to_ascii_lowercase().as_str() {
                "colours" => Section::Colours,
                "fonts" => Section::Fonts,
                "catchthebeat" => Section::Catch,
                "mania" => {
                    mania_blocks.push(Vec::new());
                    Section::Mania
                }
                _ => Section::None,
            };
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let entry = (key.trim().to_string(), value.trim().to_string());
        match current {
            Section::Colours => colours.push(entry),
            Section::Fonts => fonts.push(entry),
            Section::Catch => catch_section.push(entry),
            Section::Mania => mania_blocks.last_mut().unwrap().push(entry),
            Section::None => {}
        }
    }

    SkinConfig {
        combo_colors: parse_combo_colors(&colours),
        hitcircle_overlap: get(&fonts, "HitCircleOverlap")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(10),
        hyper_dash: get(&catch_section, "HyperDash")
            .and_then(parse_rgb)
            .unwrap_or([255, 0, 0]),
        mania_blocks,
    }
}

/// 在键值块中查找指定键（取最后一次出现的值）。
fn get<'a>(entries: &'a [(String, String)], key: &str) -> Option<&'a str> {
    entries
        .iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// 解析 "R,G,B" 或 "R,G,B,A" 颜色（忽略 alpha 分量）。
fn parse_rgb(raw: &str) -> Option<[u8; 3]> {
    let parts: Vec<&str> = raw.split(',').map(|p| p.trim()).collect();
    if parts.len() < 3 {
        return None;
    }
    Some([
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ])
}

/// 解析 Combo1..ComboN，按编号升序返回。
fn parse_combo_colors(entries: &[(String, String)]) -> Vec<[u8; 3]> {
    let mut numbered: Vec<(u32, [u8; 3])> = Vec::new();
    for (key, value) in entries {
        let Some(num) = key.strip_prefix("Combo") else {
            continue;
        };
        let Ok(index) = num.parse::<u32>() else {
            continue;
        };
        if let Some(rgb) = parse_rgb(value) {
            numbered.push((index, rgb));
        }
    }
    numbered.sort_by_key(|e| e.0);
    numbered.into_iter().map(|e| e.1).collect()
}
