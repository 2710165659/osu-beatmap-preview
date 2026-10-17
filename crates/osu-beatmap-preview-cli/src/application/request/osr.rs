//! 回放来源使用枚举表达互斥关系，库调用与命令行共享同一约束。
#[derive(Debug, Clone)]
pub enum ReplaySource {
    File(String),
    Score(String),
    Player {
        user: String,
        selection: ScoreSelection,
        mode: Option<ReplayMode>,
        include_fails: bool,
    },
}

/// 使用官方 API 的模式名称；省略模式时由查询场景决定默认行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayMode {
    Osu,
    Taiko,
    Fruits,
    Mania,
}

impl ReplayMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Osu => "osu",
            Self::Taiko => "taiko",
            Self::Fruits => "fruits",
            Self::Mania => "mania",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScoreSelection {
    #[default]
    Best,
    Recent,
}
