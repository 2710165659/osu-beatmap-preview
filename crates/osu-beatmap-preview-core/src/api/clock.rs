//! 实时预览时钟：画面与声音共用的唯一时间权威。
//!
//! 状态机全在这里，宿主只提供墙钟读数（WASM 传 `performance.now()`，测试传假时间），
//! 并把「音频线程已消费到哪个采样帧」转发回来做锚定；播放、暂停、seek、倍速都不再
//! 依赖 `<audio>` 元素或宿主自己的计时。
//!
//! 分工：音频消费位置（有音频输出时）就是「此刻耳朵听到的谱面时间」，画面必须贴着
//! 它；无音频或被自动播放策略拦住时退回墙钟，画面不因无声而冻结。锚定只做平滑修正
//! （见 [`PreviewClock::anchor_audio`]）——报告约 11ms 一条且可能迟到，硬对齐会把
//! 量化抖动放大成画面抖动。

/// 音频位置报告的正常间隔约 11ms（48kHz 下 512 帧），报告还可能晚到几毫秒。
/// 偏差落在这个死区内不做修正，避免画面跟着报告粒度抖动。
const ANCHOR_DEADBAND_MS: f64 = 40.0;

/// 偏差超出死区后，每次锚定只回收超出部分的这个比例：逐次收敛且不产生可见跳变。
const ANCHOR_CORRECTION: f64 = 0.25;

/// 实时预览时钟。
///
/// 时间单位一律是「谱面绝对毫秒」（0 = 音频文件 0 点，与渲染、混音同一坐标系），
/// `rate` 是**总倍速**（用户倍速 × 谱面变速 DT/HT），由调用方换算好再传入。
#[derive(Debug, Clone)]
pub struct PreviewClock {
    anchor_chart_ms: f64,
    anchor_wall_ms: f64,
    rate: f64,
    playing: bool,
}

impl Default for PreviewClock {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewClock {
    /// 新建一个停在 0ms、1 倍速的时钟。
    pub fn new() -> Self {
        Self {
            anchor_chart_ms: 0.0,
            anchor_wall_ms: 0.0,
            rate: 1.0,
            playing: false,
        }
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// 当前总倍速。
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// 当前谱面绝对时间（毫秒）。墙钟非有限时退回锚点值，不产生 NaN。
    pub fn current(&self, wall_ms: f64) -> f64 {
        if !self.playing {
            return self.anchor_chart_ms;
        }
        let elapsed = if wall_ms.is_finite() {
            wall_ms - self.anchor_wall_ms
        } else {
            0.0
        };
        self.anchor_chart_ms + elapsed * self.rate
    }

    /// 开始播放：从当前时刻继续走。已经播放时是空操作。
    pub fn play(&mut self, wall_ms: f64) {
        if self.playing {
            return;
        }
        self.reanchor(self.current(wall_ms), wall_ms);
        self.playing = true;
    }

    /// 暂停：把当前时刻固化为锚点。已经暂停时是空操作。
    pub fn pause(&mut self, wall_ms: f64) {
        if !self.playing {
            return;
        }
        self.reanchor(self.current(wall_ms), wall_ms);
        self.playing = false;
    }

    /// 跳到指定谱面时间：立即换锚，播放/暂停状态保持不变。
    /// 非有限目标忽略（调用方负责拒绝非法输入）。
    pub fn seek(&mut self, chart_ms: f64, wall_ms: f64) {
        if !chart_ms.is_finite() {
            return;
        }
        self.reanchor(chart_ms, wall_ms);
    }

    /// 设置总倍速；换速瞬间保持当前时刻连续，只改变之后的推进速率。
    ///
    /// 非有限或非正的倍速拒绝（保持原值），坏输入不能让时钟倒走或变 NaN。
    pub fn set_rate(&mut self, rate: f64, wall_ms: f64) {
        if !rate.is_finite() || rate <= 0.0 {
            return;
        }
        self.reanchor(self.current(wall_ms), wall_ms);
        self.rate = rate;
    }

    /// 平滑锚定到音频消费位置（此刻耳朵听到的谱面毫秒）。
    ///
    /// 偏差在 [`ANCHOR_DEADBAND_MS`] 内不动（吸收报告粒度），超出后按比例回收，
    /// 不做硬对齐：硬对齐会让画面在每条位置报告上跳一下。
    pub fn anchor_audio(&mut self, chart_ms: f64, wall_ms: f64) {
        if !chart_ms.is_finite() {
            return;
        }
        let drift = chart_ms - self.current(wall_ms);
        if !drift.is_finite() || drift.abs() <= ANCHOR_DEADBAND_MS {
            return;
        }
        let excess = (drift.abs() - ANCHOR_DEADBAND_MS) * drift.signum();
        self.reanchor(self.current(wall_ms) + excess * ANCHOR_CORRECTION, wall_ms);
    }

    fn reanchor(&mut self, chart_ms: f64, wall_ms: f64) {
        self.anchor_chart_ms = chart_ms;
        if wall_ms.is_finite() {
            self.anchor_wall_ms = wall_ms;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 暂停时时间不走，播放时按倍速推进。
    #[test]
    fn paused_clock_is_frozen_and_playing_clock_advances_by_rate() {
        let mut clock = PreviewClock::new();
        assert!(!clock.playing());
        assert_eq!(clock.current(1_000.0), 0.0);

        clock.play(1_000.0);
        assert_eq!(clock.current(1_500.0), 500.0);

        clock.set_rate(2.0, 1_500.0);
        // 换速瞬间时刻连续，之后按新倍速推进。
        assert_eq!(clock.current(1_500.0), 500.0);
        assert_eq!(clock.current(2_000.0), 1_500.0);

        clock.pause(2_000.0);
        assert_eq!(clock.current(9_999.0), 1_500.0);
    }

    /// seek 立即换锚，播放状态保持。
    #[test]
    fn seek_jumps_immediately_and_keeps_playing_state() {
        let mut clock = PreviewClock::new();
        clock.play(0.0);
        clock.seek(-1_500.0, 200.0);
        assert!(clock.playing());
        assert_eq!(clock.current(200.0), -1_500.0);
        assert_eq!(clock.current(700.0), -1_000.0);

        clock.pause(700.0);
        clock.seek(42.0, 700.0);
        assert!(!clock.playing());
        assert_eq!(clock.current(12_345.0), 42.0);
    }

    /// 坏输入不能破坏时钟：非法倍速、非法 seek、非有限墙钟都保持有意义的状态。
    #[test]
    fn bad_inputs_are_rejected_without_breaking_the_clock() {
        let mut clock = PreviewClock::new();
        clock.play(100.0);
        clock.set_rate(f64::NAN, 100.0);
        clock.set_rate(0.0, 100.0);
        clock.set_rate(-2.0, 100.0);
        assert_eq!(clock.rate(), 1.0);
        clock.seek(f64::NAN, 100.0);
        // 非有限墙钟按「时间不走」处理，而不是产出 NaN。
        assert_eq!(clock.current(f64::NAN), 0.0);
        assert_eq!(clock.current(300.0), 200.0);
    }

    /// 平滑锚定：死区内不动，超界按比例回收，不产生跳变。
    #[test]
    fn audio_anchoring_pulls_back_smoothly() {
        let mut clock = PreviewClock::new();
        clock.play(0.0);
        // 死区内的偏差（画面 200ms、音频 220ms）不动。
        clock.anchor_audio(220.0, 200.0);
        assert_eq!(clock.current(200.0), 200.0);

        // 超出死区：只回收超出部分的 25%（(60-40)*0.25 = 5）。
        clock.anchor_audio(260.0, 200.0);
        assert_eq!(clock.current(200.0), 205.0);

        // 多次锚定逐步收敛到音频位置的死区边缘（死区内不再修正），每步修正都小于偏差本身。
        for _ in 0..40 {
            let before = clock.current(200.0);
            clock.anchor_audio(260.0, 200.0);
            let after = clock.current(200.0);
            assert!(
                after > before && after <= 260.0,
                "before={before} after={after}"
            );
        }
        assert!((clock.current(200.0) - 260.0).abs() <= ANCHOR_DEADBAND_MS + 1e-3);
    }

    /// 时钟停顿时墙钟外推的偏差由锚定回收（音频才是准）。
    #[test]
    fn anchor_recovers_wall_clock_drift() {
        let mut clock = PreviewClock::new();
        clock.play(0.0);
        // 墙钟走到了 500ms，但音频只播到 300ms：画面要往回贴。
        clock.anchor_audio(300.0, 500.0);
        let corrected = clock.current(500.0);
        assert!(corrected < 500.0 && corrected > 300.0);
    }
}
