//! 判定事件是画面与声音的唯一共同来源，稳定 ID 使重复查询可去重。
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HitResult {
    Perfect,
    Good,
    Great,
    Ok,
    Meh,
    Miss,
    LargeTickHit,
    LargeTickMiss,
    SmallTickHit,
    SmallTickMiss,
    SliderTailHit,
    IgnoreHit,
    IgnoreMiss,
    SmallBonus,
    LargeBonus,
}
impl HitResult {
    pub fn value(self) -> u32 {
        match self {
            Self::Perfect => 305,
            Self::Good => 200,
            Self::Great => 300,
            Self::Ok => 100,
            Self::Meh => 50,
            Self::LargeTickHit => 30,
            Self::SmallTickHit => 10,
            Self::SliderTailHit => 150,
            Self::SmallBonus => 10,
            Self::LargeBonus => 50,
            _ => 0,
        }
    }
    pub fn is_hit(self) -> bool {
        !matches!(
            self,
            Self::Miss | Self::LargeTickMiss | Self::SmallTickMiss | Self::IgnoreMiss
        )
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Perfect => "perfect",
            Self::Good => "good",
            Self::Great => "great",
            Self::Ok => "ok",
            Self::Meh => "meh",
            Self::Miss => "miss",
            Self::LargeTickHit => "large_tick_hit",
            Self::LargeTickMiss => "large_tick_miss",
            Self::SmallTickHit => "small_tick_hit",
            Self::SmallTickMiss => "small_tick_miss",
            Self::SliderTailHit => "slider_tail_hit",
            Self::IgnoreHit => "ignore_hit",
            Self::IgnoreMiss => "ignore_miss",
            Self::SmallBonus => "small_bonus",
            Self::LargeBonus => "large_bonus",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Judgement,
    ComboBreak,
    Fail,
}
#[derive(Debug, Clone)]
pub struct GameplayEvent {
    pub id: usize,
    pub time: f64,
    pub object_id: usize,
    pub node_id: Option<usize>,
    pub result: HitResult,
    pub maximum: HitResult,
    pub kind: EventKind,
    pub error: Option<f64>,
    /// 保存触发判定的原帧坐标；同刻多帧不能从最终快照反推实际点击位置。
    pub position: Option<[f32; 2]>,
    /// 样本参数按节点的谱面时间选择，起音仍按实际事件时间。
    pub sample_time: f64,
}
