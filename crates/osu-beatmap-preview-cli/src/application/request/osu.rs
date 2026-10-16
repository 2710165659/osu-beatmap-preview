//! 普通 osu 请求的规则集选项。

#[derive(Debug, Clone, Default)]
pub struct RulesetOptions {
    pub convert: Option<String>,
    pub mods: Vec<String>,
}
