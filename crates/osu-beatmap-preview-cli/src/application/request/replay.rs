//! 回放来源使用枚举表达互斥关系，库调用与命令行共享同一约束。
#[derive(Debug, Clone)]
pub enum ReplaySource {
    File(String),
    Score(String),
    Player {
        user: ReplayUser,
        selection: ScoreSelection,
    },
}

#[derive(Debug, Clone)]
pub enum ReplayUser {
    Name(String),
    Id(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScoreSelection {
    #[default]
    Best,
    Recent,
}
