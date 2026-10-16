//! 游玩判定的公共类型与宿主接口。
//!
//! 这里刻意只放**契约与纯数据**，不碰文件、网络、输入设备，也不做判定算法：宿主
//! 把「某一时刻的输入快照」喂进来，由判定引擎算出分数/ACC/COMBO，再由绘制层把状态
//! 快照画到画面上。这样三种驱动方式共用同一条链：
//!
//! - `Preview`：没有输入，打击音完全由时间轴驱动（当前预览的行为）；
//! - `Replay`：输入来自 OSR 解析出的帧；
//! - `Play`：输入来自宿主的键盘/鼠标/触摸。
//!
//! 输入和画面叠加契约分别位于同目录的 `input`、`overlay` 模块；这里保留判定结果和
//! 请求配置，避免规则实现依赖渲染细节。

#[cfg(test)]
use super::{GameplayOverlay, InputSource};
use super::{InputSnapshot, OverlayStyle};

/// 会话/导出请求的驱动方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GameplayMode {
    /// 纯预览：打击音完全由时间轴驱动。
    #[default]
    Preview,
    /// 回放：输入来自 OSR 解析出的帧。
    Replay,
    /// 实况游玩：输入来自宿主设备。
    Play,
}

/// 判定等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judgement {
    /// mania 的 MAX（320 分档）；其它模式不会产生。
    Max,
    /// 300
    Great,
    /// 100
    Good,
    /// 50
    Meh,
    /// Miss
    Miss,
}

/// 各判定等级的计数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JudgementCounts {
    pub max: u32,
    pub great: u32,
    pub good: u32,
    pub meh: u32,
    pub miss: u32,
}

impl JudgementCounts {
    /// 累计一次判定。
    pub fn add(&mut self, judgement: Judgement) {
        let slot = match judgement {
            Judgement::Max => &mut self.max,
            Judgement::Great => &mut self.great,
            Judgement::Good => &mut self.good,
            Judgement::Meh => &mut self.meh,
            Judgement::Miss => &mut self.miss,
        };
        *slot = slot.saturating_add(1);
    }

    /// 某个判定等级的计数。
    pub fn count(&self, judgement: Judgement) -> u32 {
        match judgement {
            Judgement::Max => self.max,
            Judgement::Great => self.great,
            Judgement::Good => self.good,
            Judgement::Meh => self.meh,
            Judgement::Miss => self.miss,
        }
    }

    /// 全部判定次数（含 Miss），也就是已判定物件数。
    pub fn total(&self) -> u32 {
        self.max
            .saturating_add(self.great)
            .saturating_add(self.good)
            .saturating_add(self.meh)
            .saturating_add(self.miss)
    }
}

/// 一帧要展示的游玩状态。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScoreSnapshot {
    pub score: u32,
    /// 0～1 的准确率。
    pub accuracy: f64,
    pub combo: u32,
    pub max_combo: u32,
    pub counts: JudgementCounts,
    /// 最近一次击打的误差毫秒数（正数表示偏晚）；没有击打时为 `None`。
    pub hit_error_ms: Option<f64>,
}

/// 判定引擎：宿主按帧推进，绘制与 UI 读快照。
///
/// `absolute_ms` 是谱面绝对时间；引擎内部按自己的进度结算「到该时刻为止」的输入与
/// 物件，因此 seek、暂停、倍速都只体现为调用参数，不需要额外接口。
pub trait JudgementEngine: Send {
    /// 结算到 `absolute_ms`（含）为止的输入与物件；`input` 为该时刻的输入快照。
    fn advance(&mut self, absolute_ms: i64, input: Option<InputSnapshot>);
    /// 当前状态快照。
    fn snapshot(&self) -> ScoreSnapshot;
    /// 回到起点（重开或 seek 之后调用）。
    fn reset(&mut self);
}

/// 会话与导出请求共用的游玩配置。
#[derive(Debug, Clone, PartialEq)]
pub struct GameplayOptions {
    pub mode: GameplayMode,
    pub overlay: OverlayStyle,
    /// 输入相对音频时钟的偏移（毫秒）：正值表示输入应当更早结算，
    /// 用于补偿设备/显示延迟。
    pub input_offset_ms: i64,
}

impl Default for GameplayOptions {
    fn default() -> Self {
        Self {
            mode: GameplayMode::Preview,
            overlay: OverlayStyle::default(),
            input_offset_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最简单的时间轴输入源：按时间查找预置帧。
    ///
    /// 它同时是后续 OSR 回放适配器的模板：把解析出的帧排好序，二分查找即可。
    struct FrameTimeline(Vec<(i64, InputSnapshot)>);

    impl InputSource for FrameTimeline {
        fn snapshot_at(&self, absolute_ms: i64) -> Option<InputSnapshot> {
            let index = self
                .0
                .partition_point(|(time, _)| *time <= absolute_ms)
                .checked_sub(1)?;
            self.0.get(index).map(|(_, snapshot)| *snapshot)
        }
    }

    /// 默认是纯预览模式，不改变现有画面。
    #[test]
    fn defaults_to_preview_without_changing_frames() {
        assert_eq!(GameplayMode::default(), GameplayMode::Preview);
        let options = GameplayOptions::default();
        assert_eq!(options.mode, GameplayMode::Preview);
        assert_eq!(options.input_offset_ms, 0);
        // 所有 HUD 开关默认关闭，缩放为 1，因此接入后现有产物像素不变。
        let style = options.overlay;
        assert!(!style.show_score);
        assert!(!style.show_accuracy);
        assert!(!style.show_combo);
        assert!(!style.show_judgements);
        assert!(!style.show_hit_error);
        assert!(!style.show_keys);
        assert!(!style.show_cursor);
        assert!(!style.show_player);
        assert_eq!(style.scale, 1.0);
    }

    /// 判定计数按等级分别累加。
    #[test]
    fn judgement_counts_accumulate_per_grade() {
        let mut counts = JudgementCounts::default();
        counts.add(Judgement::Max);
        counts.add(Judgement::Great);
        counts.add(Judgement::Great);
        counts.add(Judgement::Good);
        counts.add(Judgement::Meh);
        counts.add(Judgement::Miss);
        assert_eq!(counts.count(Judgement::Max), 1);
        assert_eq!(counts.count(Judgement::Great), 2);
        assert_eq!(counts.count(Judgement::Good), 1);
        assert_eq!(counts.count(Judgement::Meh), 1);
        assert_eq!(counts.count(Judgement::Miss), 1);
        assert_eq!(counts.total(), 6);
    }

    /// 叠加内容来自状态与输入快照。
    #[test]
    fn overlay_comes_from_state_and_input_snapshot() {
        let score = ScoreSnapshot {
            score: 12_345,
            accuracy: 0.9876,
            combo: 42,
            max_combo: 108,
            counts: JudgementCounts {
                great: 10,
                miss: 1,
                ..JudgementCounts::default()
            },
            hit_error_ms: Some(-12.5),
        };
        let input = InputSnapshot {
            cursor: Some([256.0, 192.0]),
            keys: 0b1,
            pressed: 0b1,
            released: 0,
        };
        let overlay = GameplayOverlay::from_snapshot(OverlayStyle::default(), score, Some(input));
        assert_eq!(overlay.score, score);
        assert_eq!(overlay.cursor, Some([256.0, 192.0]));
        assert_eq!(overlay.keys, 0b1);
        assert!(overlay.player.is_none());
        assert!(overlay.judgements.is_empty());

        // 没有输入时（预览/纯读谱面）也不该 panic。
        let overlay = GameplayOverlay::from_snapshot(OverlayStyle::default(), score, None);
        assert_eq!(overlay.cursor, None);
        assert_eq!(overlay.keys, 0);
    }

    /// 输入时间轴按时间取最近一帧。
    #[test]
    fn input_timeline_picks_latest_frame_at_time() {
        let timeline = FrameTimeline(vec![
            (0, InputSnapshot::default()),
            (
                100,
                InputSnapshot {
                    cursor: Some([10.0, 20.0]),
                    keys: 0b1,
                    pressed: 0b1,
                    released: 0,
                },
            ),
            (
                200,
                InputSnapshot {
                    cursor: Some([30.0, 40.0]),
                    keys: 0,
                    pressed: 0,
                    released: 0b1,
                },
            ),
        ]);
        // 第一帧之前没有输入。
        assert!(timeline.snapshot_at(-1).is_none());
        assert_eq!(timeline.snapshot_at(0).map(|s| s.keys), Some(0));
        assert_eq!(
            timeline.snapshot_at(150).and_then(|s| s.cursor),
            Some([10.0, 20.0])
        );
        assert_eq!(
            timeline.snapshot_at(10_000).and_then(|s| s.cursor),
            Some([30.0, 40.0])
        );
    }
}
