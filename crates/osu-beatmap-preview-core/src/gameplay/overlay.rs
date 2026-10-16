//! HUD、光标和按键的纯数据叠加契约。

use super::{InputSnapshot, Judgement, ScoreSnapshot};

/// HUD 显示开关与缩放。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayStyle {
    pub show_score: bool,
    pub show_accuracy: bool,
    pub show_combo: bool,
    pub show_judgements: bool,
    pub show_hit_error: bool,
    pub show_keys: bool,
    pub show_cursor: bool,
    pub show_player: bool,
    pub scale: f64,
}

impl Default for OverlayStyle {
    fn default() -> Self {
        Self {
            show_score: false,
            show_accuracy: false,
            show_combo: false,
            show_judgements: false,
            show_hit_error: false,
            show_keys: false,
            show_cursor: false,
            show_player: false,
            scale: 1.0,
        }
    }
}

/// 一帧的叠加内容，由状态与输入快照组装。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GameplayOverlay {
    pub style: OverlayStyle,
    pub score: ScoreSnapshot,
    pub cursor: Option<[f32; 2]>,
    pub keys: u32,
    /// 回放时显示玩家名。
    pub player: Option<String>,
    /// 最近一段时间的判定弹字（谱面绝对时间、判定）。
    pub judgements: Vec<(i64, Judgement)>,
}

impl GameplayOverlay {
    /// 从状态与输入快照组装叠加内容；判定弹字由调用方按时间窗补充。
    pub fn from_snapshot(
        style: OverlayStyle,
        score: ScoreSnapshot,
        input: Option<InputSnapshot>,
    ) -> Self {
        Self {
            style,
            score,
            cursor: input.and_then(|snapshot| snapshot.cursor),
            keys: input.map_or(0, |snapshot| snapshot.keys),
            player: None,
            judgements: Vec::new(),
        }
    }
}
