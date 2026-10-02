//! 混音输出流：决定「下一段混音从哪里、混多少」，并跟踪音频线程的消费进度。
//!
//! 环形缓冲本身在宿主（WASM 侧是 `SharedArrayBuffer`，音频线程直接读），这里只做
//! 坐标系与填充策略——音画不同步最容易出在这里，因此有三条硬约束：
//!
//! 1. **坐标系**：相对帧号（自上次 [`AudioStream::reset`] 起写了/读了多少帧）与音频线程
//!    同一坐标系；`written` 只增不减，绕回由音频线程用「已写入 - 环形容量」自行判断。
//! 2. **锚点**：`base_frame` 是相对帧 0 对应的谱面帧，只在 reset 时改变；混音位置由
//!    「锚点 + 已写入」唯一确定，不允许中途改写，否则已混好的数据会被换掉时间含义。
//! 3. **预读必须领先消费**：填充目标取「画面时钟 + 预读」与「音频线程已读位置 + 预读」中
//!    更靠后的那个，前者保证内容按谱面时间轴连续，后者保证音频线程永远有数据可读。
//!    消费位置把目标再往前拉时设上限，一次异常的旧报告不该驱动出巨量混音。

use super::clock::PreviewClock;
use crate::hitsound::HitsoundMixer;

/// 环形缓冲里预读多少毫秒。
///
/// 从混音到被音频线程取走之间必须留出余量，覆盖主线程每帧补一次（60Hz 约 17ms）、
/// 音频线程每 512 帧（48kHz 下约 11ms）才回报一次位置、以及主线程偶发卡顿。
/// 取 170ms 能盖住这些抖动；预读不推迟声音——环形里第 r 帧对应的谱面时间由锚点唯一
/// 确定，音频线程读到它就播那个时间，与写入时刻无关。
pub const LOOKAHEAD_MS: f64 = 170.0;

/// 画面时钟与消费位置之间允许的漂移（毫秒）；超出后整条流重置到画面时钟。
///
/// 触发场景是「音频停滞后恢复」「切到后台很久再回来」「seek 后尚未对齐」：此时旧数据的
/// 时间含义已经不可信，重置相当于一次从画面位置开始的 seek，音频跳到画面位置继续，
/// 画面绝不回跳。
pub const RESEEK_TOLERANCE_MS: f64 = 250.0;

/// 消费位置最多可以把填充目标提前多少毫秒。
///
/// 两条时钟的输出缓冲不同，音频线程稳定领先几十毫秒是正常的；但一次异常的位置报告
/// （例如重置消息还没落地时回报的旧位置）不该驱动出巨量混音，因此设上限。
const MAX_CONSUMER_LEAD_MS: f64 = 250.0;

/// 一次最多向混音器要多少帧：窗口太大会让所有声音都留在声音列表里逐帧遍历，
/// 长谱面会把混音拖到实时以下（表现为打击音整体消失）。
const MAX_CHUNK_FRAMES: usize = 4096;

/// 消费报告的年龄超过这个值就不再向前外推：报告断流说明音频线程停了，
/// 继续外推会把「停摆」伪装成「还在走」，填充目标会一路跑远。
const MAX_EXTRAPOLATION_MS: f64 = 100.0;

/// 混音输出流的填充状态。
#[derive(Debug, Clone)]
pub struct AudioStream {
    sample_rate: u32,
    /// 相对帧 0 对应的谱面帧；可为负（首个物件前的预卷段）。
    base_frame: i64,
    /// 已写入的相对帧数（宿主把返回的 PCM 依次写进环形缓冲）。
    written: u64,
    /// 音频线程已消费的相对帧数。
    consumed: u64,
    /// 最近一次消费报告到达时的墙钟；`None` 表示还没有报告。
    consumed_wall_ms: Option<f64>,
    /// 重置纪元：每次 reset 自增，宿主据此把环形读写指针一起归零。
    epoch: u64,
}

impl AudioStream {
    /// 新建一条输出流；`sample_rate` 是音频设备采样率（混音输出采样率）。
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
            base_frame: 0,
            written: 0,
            consumed: 0,
            consumed_wall_ms: None,
            epoch: 0,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 重置纪元；宿主看到变化时必须把环形读写指针一起归零并通知音频线程重读。
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// 已写入的相对帧数（宿主写环形缓冲的下标基准）。
    pub fn written(&self) -> u64 {
        self.written
    }

    /// 重置整条流并把锚点对齐到 `chart_ms`（谱面绝对毫秒，可为负）。
    pub fn reset(&mut self, chart_ms: f64) {
        self.base_frame = frame_of(chart_ms, self.sample_rate);
        self.written = 0;
        self.consumed = 0;
        self.consumed_wall_ms = None;
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// 记录音频线程回报的消费位置（相对帧）。
    ///
    /// 可能落在写入前沿之后——音频线程超前后读到的是静音，那些帧并没有真的取到数据，
    /// 不能算作「已经补到那儿了」，因此夹到写入范围内并保持单调（重置前迟到的旧报告
    /// 不会把位置拉回去）。
    pub fn note_consumed(&mut self, frames: u64, wall_ms: f64) {
        let clamped = frames.min(self.written);
        self.consumed = self.consumed.max(clamped);
        if wall_ms.is_finite() {
            self.consumed_wall_ms = Some(wall_ms);
        }
    }

    /// 相对帧号对应的谱面绝对毫秒。
    pub fn chart_ms_at(&self, relative_frame: u64) -> f64 {
        frame_to_ms(self.base_frame + relative_frame as i64, self.sample_rate)
    }

    /// 音频线程此刻听到的谱面毫秒。
    ///
    /// 报告最多落后一个回报间隔，按 `wall_ms` 与报告时刻之差做有限外推，消掉报告粒度
    /// 带来的系统性滞后；外推超时（音频线程停摆）后退回报告值本身，让漂移暴露出来。
    pub fn consumer_chart_ms(&self, clock: &PreviewClock, wall_ms: f64) -> f64 {
        let reported = self.chart_ms_at(self.consumed);
        let Some(reported_wall) = self.consumed_wall_ms else {
            return reported;
        };
        let age = wall_ms - reported_wall;
        if age.is_finite() && (0.0..=MAX_EXTRAPOLATION_MS).contains(&age) {
            reported + age * clock.rate()
        } else {
            reported
        }
    }

    /// 按当前时钟补足环形缓冲，返回交错立体声 PCM（可能为空）。
    ///
    /// 宿主每帧调用一次；返回的帧数可能小于 `max_frames`（按需补，不预混多余的静音）。
    /// `written` 与返回数据同步前进，宿主把数据接在环形写入前沿之后即可。
    pub fn pull(
        &mut self,
        mixer: &mut HitsoundMixer,
        clock: &PreviewClock,
        wall_ms: f64,
        max_frames: usize,
    ) -> Vec<f32> {
        let clock_frame = frame_of(clock.current(wall_ms), self.sample_rate);
        let picture_end = clock_frame + ms_to_frames(LOOKAHEAD_MS, self.sample_rate);
        let consumer_end =
            self.base_frame + self.consumed as i64 + ms_to_frames(LOOKAHEAD_MS, self.sample_rate);
        let wanted_end = picture_end.max(
            (consumer_end).min(picture_end + ms_to_frames(MAX_CONSUMER_LEAD_MS, self.sample_rate)),
        );

        let frontier = self.base_frame + self.written as i64;
        let mut remaining = (wanted_end - frontier).max(0) as usize;
        remaining = remaining.min(max_frames);
        if remaining == 0 {
            return Vec::new();
        }

        let mut output = Vec::with_capacity(remaining * 2);
        while output.len() < remaining * 2 {
            let frames = remaining.min(MAX_CHUNK_FRAMES);
            // 混音位置由「锚点 + 已写入」唯一确定；连续补段时这与混音器当前重合，
            // 重置后则是必要的重新对齐。
            mixer.set_position(self.chart_ms_at(self.written));
            output.extend_from_slice(&mixer.render(frames));
            self.written += frames as u64;
            remaining -= frames;
        }
        output
    }
}

/// 谱面毫秒 → 谱面采样帧（输出采样率下、向下取整；可为负）。
fn frame_of(chart_ms: f64, sample_rate: u32) -> i64 {
    if !chart_ms.is_finite() {
        return 0;
    }
    (chart_ms * sample_rate as f64 / 1000.0).floor() as i64
}

/// 谱面采样帧 → 谱面毫秒。
fn frame_to_ms(frame: i64, sample_rate: u32) -> f64 {
    frame as f64 * 1000.0 / sample_rate as f64
}

/// 毫秒换算成采样帧（至少 1 帧，避免极短时长在低采样率下被截成 0）。
fn ms_to_frames(milliseconds: f64, sample_rate: u32) -> i64 {
    (milliseconds * sample_rate as f64 / 1000.0)
        .round()
        .max(1.0) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hitsound::{HitsoundTimeline, SampleLibrary};

    /// 1kHz 输出：1 帧 = 1ms，方便断言。
    fn new_stream() -> AudioStream {
        AudioStream::new(1000)
    }

    fn mixer() -> HitsoundMixer {
        HitsoundMixer::new(SampleLibrary::new(), HitsoundTimeline::default(), 1000)
    }

    fn clock_at(chart_ms: f64) -> PreviewClock {
        let mut clock = PreviewClock::new();
        clock.seek(chart_ms, 0.0);
        clock.play(0.0);
        clock
    }

    /// reset 换锚点、清计数并换纪元。
    #[test]
    fn reset_moves_anchor_and_bumps_epoch() {
        let mut stream = new_stream();
        stream.note_consumed(10, 5.0);
        assert_eq!(stream.epoch(), 0);
        stream.reset(-500.0);
        assert_eq!(stream.epoch(), 1);
        assert_eq!(stream.written(), 0);
        // 锚点允许为负：预卷段也在同一条时间轴上。
        assert_eq!(stream.chart_ms_at(0), -500.0);
    }

    /// 消费位置夹在写入范围内且单调，迟到的旧报告不会把位置拉回去。
    #[test]
    fn consumed_position_is_clamped_and_monotonic() {
        let mut stream = new_stream();
        stream.reset(0.0);
        let mut mixer = mixer();
        let clock = clock_at(0.0);
        stream.pull(&mut mixer, &clock, 0.0, 100);
        assert_eq!(stream.written(), 100);

        stream.note_consumed(50, 10.0);
        assert_eq!(stream.chart_ms_at(stream.consumed), 50.0);
        // 超过写入前沿的报告被夹到写入前沿（超前读到的是静音，不算真的消费）。
        stream.note_consumed(999, 11.0);
        assert_eq!(stream.chart_ms_at(stream.consumed), 100.0);
        // 旧报告不回退。
        stream.note_consumed(10, 12.0);
        assert_eq!(stream.chart_ms_at(stream.consumed), 100.0);
    }

    /// 填充到「画面时钟 + 预读」，按需补段而不是一次混出巨量静音。
    #[test]
    fn pull_fills_up_to_clock_plus_lookahead() {
        let mut stream = new_stream();
        stream.reset(0.0);
        let mut mixer = mixer();
        let clock = clock_at(0.0);

        // 时钟 0ms：目标 = 0 + 170ms 预读 = 170 帧。
        let first = stream.pull(&mut mixer, &clock, 0.0, 1000);
        assert_eq!(first.len(), 170 * 2);
        assert_eq!(stream.written(), 170);

        // 再补同刻：已经到位，不重复混音。
        assert!(stream.pull(&mut mixer, &clock, 0.0, 1000).is_empty());

        // 时钟前进 100ms：再补 100 帧。
        let second = stream.pull(&mut mixer, &clock, 100.0, 1000);
        assert_eq!(second.len(), 100 * 2);
        assert_eq!(stream.written(), 270);
    }

    /// max_frames 限制单次返回量，剩下的留给后续调用。
    #[test]
    fn pull_respects_max_frames() {
        let mut stream = new_stream();
        stream.reset(0.0);
        let mut mixer = mixer();
        let clock = clock_at(0.0);
        let chunk = stream.pull(&mut mixer, &clock, 0.0, 64);
        assert_eq!(chunk.len(), 64 * 2);
        assert_eq!(stream.written(), 64);
    }

    /// 消费位置可以把填充目标往前拉，但设上限。
    #[test]
    fn consumer_lead_pulls_target_with_a_cap() {
        let mut stream = new_stream();
        stream.reset(0.0);
        let mut mixer = mixer();
        let clock = clock_at(0.0);
        stream.pull(&mut mixer, &clock, 0.0, 1000);
        // 音频线程追到写入前沿：填充目标被它往前拉。
        stream.note_consumed(stream.written(), 0.0);
        stream.pull(&mut mixer, &clock, 0.0, 1000);
        assert_eq!(stream.written(), 340);
        // 再追再拉，但最多到「画面时钟 + 预读 + 上限」，一次异常报告不会驱动出巨量混音。
        stream.note_consumed(stream.written(), 0.0);
        stream.pull(&mut mixer, &clock, 0.0, 1000);
        let cap = ms_to_frames(LOOKAHEAD_MS + MAX_CONSUMER_LEAD_MS, 1000);
        assert_eq!(stream.written() as i64, cap);
    }

    /// 消费位置按报告时刻做有限外推；报告断流后退回报告值，把漂移暴露给对齐逻辑。
    #[test]
    fn consumer_position_extrapolates_only_while_reports_are_fresh() {
        let mut stream = new_stream();
        stream.reset(0.0);
        let mut mixer = mixer();
        let clock = clock_at(0.0);
        stream.pull(&mut mixer, &clock, 0.0, 1000);

        stream.note_consumed(100, 100.0);
        // 报告后 50ms：外推 50 帧（1kHz、1 倍速）。
        assert_eq!(stream.consumer_chart_ms(&clock, 150.0), 150.0);
        // 报告过期：不再外推，停在报告值上。
        assert_eq!(stream.consumer_chart_ms(&clock, 100.0 + 500.0), 100.0);
        // 从未报告过：停在锚点。
        let fresh = new_stream();
        assert_eq!(fresh.consumer_chart_ms(&clock, 999.0), 0.0);
    }
}
