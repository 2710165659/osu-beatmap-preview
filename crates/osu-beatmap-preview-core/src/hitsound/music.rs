//! 音乐播放：在输出帧序列上做重采样（音高）与 WSOLA 时间伸缩（速度）。
//!
//! 时间伸缩与重采样是两件正交的事：WSOLA 只改内容推进速率、保持音高；重采样同时改
//! 音高与内容速率。两者组合出 osu! 的两种变速语义——「变速保调」（DT/HT 的
//! `AdjustableProperty.Tempo`）与「固定音高偏移」（NC/DC 的 `Frequency` + `Tempo`）。
//! 调用方用 [`MusicRate`] 把所在时间域的倍率换算好，本模块只按参数取源样本。
//!
//! 恒等倍率（无 Mod）走无状态快路径，取样公式与改动前的实现逐位一致；纯重采样
//! （NC/DC 默认速度、实时端的恒等倍率）也只做线性插值读。只有 `stretch != 1`
//! 才进入 WSOLA，因此「不需要时间伸缩」的路径没有任何额外开销与音质损失。

use super::sample::SampleData;

/// 只读立体声采样源：位置以源采样帧为单位，允许小数。
pub trait StereoSource {
    /// 采样帧 `frame` 的左右声道；越界、负数与非有限位置按静音处理。
    fn sample_at(&self, frame: f64) -> (f32, f32);
}

impl StereoSource for SampleData {
    fn sample_at(&self, frame: f64) -> (f32, f32) {
        self.frame_at(frame)
    }
}

/// 音乐播放倍率：重采样倍率 + 每个输出帧消费的视图帧数。
///
/// 「视图帧」是按重采样倍率看源样本的坐标系：一个视图帧对应 `resample` 个源采样帧。
/// 两者相乘就是每个输出帧推进的内容量，因此平均内容速率恒等于 `resample * stretch`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MusicRate {
    /// 视图相对源的重采样倍率（音高倍率）。
    pub resample: f64,
    /// 每个输出帧消费的视图帧数（时间伸缩倍率）。
    pub stretch: f64,
}

impl MusicRate {
    /// 不做任何处理。
    pub const IDENTITY: MusicRate = MusicRate {
        resample: 1.0,
        stretch: 1.0,
    };

    /// 输出域：输出帧就是实时帧，谱面时间由调用方按 `speed` 推进（CLI MP4 导出）。
    ///
    /// 目标：内容速率 = `speed`、音高 = `pitch`；组合「先重采样再时间伸缩」得
    /// `resample = pitch`、`stretch = speed / pitch`。
    pub fn output_domain(speed: f64, pitch: f64) -> Self {
        let (speed, pitch) = sanitize_pair(speed, pitch);
        MusicRate {
            resample: pitch,
            stretch: speed / pitch,
        }
    }

    /// 图表域：输出帧按谱面时间 1:1（实时预览，宿主音频线程再按总倍率重采样）。
    ///
    /// 宿主的总倍率 `rate = 用户倍速 × speed` 已经把内容与音高都乘过一次，因此这里
    /// 要抵消：目标变为内容速率 = 1、音高 = `pitch / speed`，即
    /// `resample = pitch / speed`、`stretch = speed / pitch`。
    ///
    /// 注意 `pitch` 是「Mod 的音高倍率」，不含用户倍速：网页端的用户倍速与游戏里的
    /// `UserPlaybackRate` 一样按 `Frequency` 处理（见
    /// `MasterGameplayClockContainer.addAdjustmentsToTrack`），因此它继续变调。
    pub fn chart_domain(speed: f64, pitch: f64) -> Self {
        let (speed, pitch) = sanitize_pair(speed, pitch);
        MusicRate {
            resample: pitch / speed,
            stretch: speed / pitch,
        }
    }

    /// 是否完全不需要处理（快路径）。
    pub fn is_identity(&self) -> bool {
        self.resample == 1.0 && self.stretch == 1.0
    }

    /// 消毒：任一倍率非法就整体回退到恒等；合法但越界（手写配置/未来 Mod）的值夹紧。
    ///
    /// 合法 Mod 组合下 `resample ∈ [0.75, 1.5]`、`stretch ∈ [0.667, 2.0]`，夹紧只用来
    /// 挡住病态输入，不会改变现有行为；宁可不处理，也不要拿坏参数去改音高。
    fn sanitized(self) -> Self {
        let valid = |value: f64| value.is_finite() && value > 0.0;
        if !valid(self.resample) || !valid(self.stretch) {
            return MusicRate::IDENTITY;
        }
        let resample = self.resample.clamp(0.25, 4.0);
        let stretch = self.stretch.clamp(0.25, 4.0);
        if resample == 1.0 && stretch == 1.0 {
            MusicRate::IDENTITY
        } else {
            MusicRate { resample, stretch }
        }
    }
}

fn sanitize_pair(speed: f64, pitch: f64) -> (f64, f64) {
    let ok = |value: f64| value.is_finite() && value > 0.0;
    if ok(speed) && ok(pitch) {
        (speed, pitch)
    } else {
        (1.0, 1.0)
    }
}

/// 判定「与上次渲染连续」的内容毫秒容差。
///
/// 宿主的混音位置可能由两条不同的浮点路径算出（累加窗口长度 vs 由锚点与帧号反算），
/// 相差在 1e-9 量级；而真正的 seek 至少是毫秒级，因此 1e-3ms 既能容忍浮点噪声，也不会
/// 把 seek 误判成连续（误判会让 WSOLA 状态错位并产生爆音）。
const CONTINUITY_EPS_MS: f64 = 1e-3;

/// WSOLA 分析窗长度（毫秒）。
///
/// 与 osu! 所用 BASS_FX 快速变速参数同量级（`TempoSequence` = 30ms、
/// `TempoOverlap` = 4ms）：窗越长越平滑但越吃 CPU，重叠越短交叉处越容易听出颗粒感。
const WSOLA_WINDOW_MS: f64 = 30.0;
/// 交叉淡变长度（毫秒）；也是相似度搜索的相关窗长度。
const WSOLA_OVERLAP_MS: f64 = 4.0;
/// 相似度搜索半径（毫秒）。
///
/// 平均内容速率由段起点 `p_k = p_{k-1} + hop * stretch + δ_k` 精确决定，`δ` 只影响局部
/// 相位对齐，因此它同时是「音乐与画面之间允许的局部抖动」上限。
const WSOLA_SEEK_MS: f64 = 4.0;

/// 一个输出采样率下的 WSOLA 网格参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WsolaParams {
    /// 分析窗长度（帧）。
    window: usize,
    /// 交叉淡变（= 相似度相关窗）长度（帧）。
    overlap: usize,
    /// 输出跳距（帧）：`window - overlap`。
    hop: usize,
    /// 相似度搜索半径（帧）。
    seek: i64,
}

fn wsola_params(output_rate: u32) -> WsolaParams {
    let per_ms = |ms: f64| ((ms * output_rate as f64 / 1000.0).round() as usize).max(1);
    let window = per_ms(WSOLA_WINDOW_MS);
    let overlap = per_ms(WSOLA_OVERLAP_MS).min(window - 1).max(1);
    WsolaParams {
        window,
        overlap,
        hop: window - overlap,
        seek: per_ms(WSOLA_SEEK_MS) as i64,
    }
}

/// 段内偏移 `offset` 处的交叉淡变权重。
///
/// 段覆盖 `[k*hop, k*hop + window)`，其中 `[0, overlap)` 淡入、`[hop, window)` 淡出，
/// 中间为 1。相邻两段的淡出区与淡入区完全重合，因此权重之和恒为 1（交叉处不会掉音量）。
fn fade_weight(offset: usize, params: &WsolaParams) -> f32 {
    let overlap = params.overlap as f32;
    if offset < params.overlap {
        offset as f32 / overlap
    } else if offset >= params.hop {
        1.0 - (offset - params.hop) as f32 / overlap
    } else {
        1.0
    }
}

/// 按输出帧顺序产生音乐样本的播放器。
///
/// 调用方按输出帧序列顺序调用 [`MusicPlayer::render_at`]（实时预览分块补缓冲、CLI 导出
/// 按编码块推进），位置跳变（seek、换速、换锚点）由内容毫秒的连续性自动识别并复位。
#[derive(Debug, Clone)]
pub struct MusicPlayer {
    output_rate: u32,
    rate: MusicRate,
    /// 上一次渲染结束时「下一帧应有的内容毫秒」；`NAN` 表示状态未初始化。
    next_content_ms: f64,
    /// 上一次渲染使用的源采样率；0 表示状态未初始化。
    source_rate: u32,
    /// 自上次复位起已交给调用方的输出帧数。
    rendered: u64,
    /// 下一个要合成的 WSOLA 段号。
    next_segment: u64,
    /// 下一个 WSOLA 段的**名义**视图起点（视图帧）。
    ///
    /// 名义位置只按 `hop * stretch` 推进，是「平均内容速率」的唯一来源；相似度搜索的偏移
    /// 只加在名义位置之上做局部对齐，绝不回头参与下一次推进，否则搜索的系统性偏向会累积
    /// 成整体变速（纯音这类周期性信号尤其容易被偏向一侧）。
    next_nominal_view: f64,
    /// 上一个已合成段的**实际**视图起点：相似度搜索的模板取自它的自然延续。
    previous_view: f64,
    /// 已合成但未吐出的输出帧（交错立体声），第 0 帧对应 `acc_start`。
    accum: Vec<f32>,
    /// `accum[0]` 对应的输出帧号（自上次复位起）。
    acc_start: u64,
    /// 已定稿的输出帧上界（不含）：这些帧不会再被后续段改写。
    final_until: u64,
}

impl MusicPlayer {
    /// 新建播放器；`output_sample_rate` 是宿主/编码器的输出采样率。
    pub fn new(output_sample_rate: u32) -> Self {
        Self {
            output_rate: output_sample_rate.max(1),
            rate: MusicRate::IDENTITY,
            next_content_ms: f64::NAN,
            source_rate: 0,
            rendered: 0,
            next_segment: 0,
            next_nominal_view: 0.0,
            previous_view: 0.0,
            accum: Vec::new(),
            acc_start: 0,
            final_until: 0,
        }
    }

    /// 当前倍率。
    pub fn rate(&self) -> MusicRate {
        self.rate
    }

    /// 设置倍率；倍率变化时清空时间伸缩状态，下一次渲染会在给定位置重新起段。
    pub fn set_rate(&mut self, rate: MusicRate) {
        let rate = rate.sanitized();
        if rate == self.rate {
            return;
        }
        self.rate = rate;
        // 倍率换了，旧的内容位置预期不再成立：标成未初始化，让下一次渲染复位。
        self.next_content_ms = f64::NAN;
        self.source_rate = 0;
    }

    /// 渲染 `frames` 个输出帧写入 `output`（交错立体声，至少 `frames * 2` 个元素）。
    ///
    /// `content_ms` 是本次窗口第一帧对应的源（谱面）毫秒位置，允许为负（首个物件前的
    /// 预卷）；`source_rate` 是源样本的采样率。与上次终点不连续时会自动复位。
    ///
    /// **调用约定**：同一段播放里 `content_ms` 必须按 [`MusicRate::advance_ms`] 推进
    /// （图表域 = `1000 / output_rate`，输出域 = `speed * 1000 / output_rate`），
    /// [`MusicRate::chart_domain`] 与 [`MusicRate::output_domain`] 的调用方都满足这一点。
    /// 推进量对不上会被当成 seek：位置由传入值决定，混音因此出现跳变（音乐与画面错位）。
    pub fn render_at<S: StereoSource>(
        &mut self,
        content_ms: f64,
        source: &S,
        source_rate: u32,
        frames: usize,
        output: &mut [f32],
    ) {
        let samples = (frames * 2).min(output.len());
        if samples == 0 {
            return;
        }
        let frames = samples / 2;
        let source_rate = source_rate.max(1);
        let content_ms = if content_ms.is_finite() {
            content_ms
        } else {
            0.0
        };

        let continuous = self.source_rate == source_rate
            && self.next_content_ms.is_finite()
            && (content_ms - self.next_content_ms).abs() <= CONTINUITY_EPS_MS;
        if !continuous {
            self.reset_at(content_ms, source_rate);
        }

        if self.rate.stretch == 1.0 {
            self.render_direct(content_ms, source, source_rate, &mut output[..samples]);
        } else {
            self.render_stretched(source, source_rate, frames, &mut output[..samples]);
        }
        self.next_content_ms = content_ms + self.advance_ms() * frames as f64;
    }

    /// 每个输出帧推进的内容毫秒（平均速率）。
    fn advance_ms(&self) -> f64 {
        self.rate.stretch * self.rate.resample * 1000.0 / self.output_rate as f64
    }

    /// 内容毫秒对应的视图位置（视图帧）。
    fn view_of(&self, content_ms: f64) -> f64 {
        content_ms * self.output_rate as f64 / (1000.0 * self.rate.resample)
    }

    fn reset_at(&mut self, content_ms: f64, source_rate: u32) {
        self.source_rate = source_rate;
        self.next_content_ms = content_ms;
        self.rendered = 0;
        self.next_segment = 0;
        self.accum.clear();
        self.acc_start = 0;
        self.final_until = 0;
        self.next_nominal_view = self.view_of(content_ms);
        self.previous_view = self.next_nominal_view;
    }

    /// 恒等/纯重采样路径：逐帧线性插值读，无状态。
    ///
    /// 内容毫秒按 [`MusicRate::advance_ms`] 推进（输出域 = `resample * 1000 / output_rate`
    /// 毫秒每帧），源帧位置就是 `内容毫秒 × 源采样率 / 1000`：**不能再乘一次倍率**，
    /// 否则「内容推进」与「取样步长」会各乘一次倍率，实际速度变成 倍率²。
    /// 音高与内容速率在这里本来就是同一个量（纯重采样），只有当 `resample == speed`
    /// 时才走这条路径（`stretch == 1` 等价于 `pitch == speed`），因此两者一致。
    ///
    /// `resample == 1` 时本式与改动前的音乐路径完全同式（先算 `frame_time`，
    /// 再乘源采样率），无 Mod 的输出因此逐位不变。
    fn render_direct<S: StereoSource>(
        &self,
        content_ms: f64,
        source: &S,
        source_rate: u32,
        output: &mut [f32],
    ) {
        let step_ms = self.advance_ms();
        let source_per_ms = source_rate as f64 / 1000.0;
        for (index, pair) in output.chunks_exact_mut(2).enumerate() {
            let frame_time = content_ms + index as f64 * step_ms;
            let (left, right) = source.sample_at(frame_time * source_per_ms);
            pair[0] = left;
            pair[1] = right;
        }
    }

    /// WSOLA 路径：按输出跳距合成段，边定稿边吐给调用方。
    fn render_stretched<S: StereoSource>(
        &mut self,
        source: &S,
        source_rate: u32,
        frames: usize,
        output: &mut [f32],
    ) {
        let params = wsola_params(self.output_rate);
        let read_step = source_rate as f64 / self.output_rate as f64 * self.rate.resample;
        let mut written = 0usize;
        while written < frames {
            // 已经定稿的帧不会再被后续段改写：先尽量吐出去。
            if self.final_until > self.rendered {
                let available = (self.final_until - self.rendered) as usize;
                let take = available.min(frames - written);
                let from = (self.rendered - self.acc_start) as usize;
                let slice = &self.accum[from * 2..(from + take) * 2];
                output[written * 2..(written + take) * 2].copy_from_slice(slice);
                self.rendered += take as u64;
                written += take;
                continue;
            }
            self.synthesize_segment(source, read_step, &params);
        }
    }

    /// 合成下一个 WSOLA 段并累加到 `accum`。
    fn synthesize_segment<S: StereoSource>(
        &mut self,
        source: &S,
        read_step: f64,
        params: &WsolaParams,
    ) {
        // 已吐出的帧不再需要，回收掉，让缓冲长度保持在 O(window)。
        if self.rendered > self.acc_start {
            let drop_frames = (self.rendered - self.acc_start) as usize;
            self.accum.drain(..drop_frames * 2);
            self.acc_start = self.rendered;
        }

        let segment = self.next_segment;
        let hop = params.hop as f64;
        // 名义位置精确推进；相似度搜索只在它附近取偏移，偏移不参与下一次推进。
        let nominal = self.next_nominal_view;
        let start = if segment == 0 {
            nominal
        } else {
            nominal + self.best_offset(source, read_step, params, nominal) as f64
        };

        let segment_start = segment * params.hop as u64;
        let segment_end = segment_start + params.window as u64;
        let current_end = self.acc_start + (self.accum.len() / 2) as u64;
        if segment_end > current_end {
            self.accum
                .resize(((segment_end - self.acc_start) * 2) as usize, 0.0);
        }

        for offset in 0..params.window {
            let weight = fade_weight(offset, params);
            let (left, right) = source.sample_at((start + offset as f64) * read_step);
            let index = ((segment_start + offset as u64 - self.acc_start) * 2) as usize;
            self.accum[index] += left * weight;
            self.accum[index + 1] += right * weight;
        }

        self.previous_view = start;
        self.next_segment += 1;
        // 名义推进量只看 hop 与 stretch：搜索偏移不累积，平均内容速率因此精确。
        self.next_nominal_view = nominal + hop * self.rate.stretch;
        self.final_until = self.next_segment * params.hop as u64;
    }

    /// 相似度搜索：让新段头部与上一段的自然延续对齐，返回最优偏移（视图帧，整数）。
    ///
    /// 相关窗用左右声道均值（单声道混合）算一次、两声道共用同一偏移，立体声像不会散。
    /// 模板或候选能量接近 0（静音、曲末采样）时直接返回 0，避免无意义的噪声放大。
    fn best_offset<S: StereoSource>(
        &self,
        source: &S,
        read_step: f64,
        params: &WsolaParams,
        nominal: f64,
    ) -> i64 {
        let mono = |view: f64| {
            let (left, right) = source.sample_at(view * read_step);
            (left + right) as f64 * 0.5
        };
        let template_start = self.previous_view + params.hop as f64;
        let mut template = vec![0.0_f64; params.overlap];
        let mut template_energy = 0.0_f64;
        for (index, slot) in template.iter_mut().enumerate() {
            let value = mono(template_start + index as f64);
            *slot = value;
            template_energy += value * value;
        }
        if template_energy <= f64::EPSILON {
            return 0;
        }

        let mut best_offset = 0_i64;
        let mut best_score = f64::NEG_INFINITY;
        for offset in -params.seek..=params.seek {
            let candidate_start = nominal + offset as f64;
            let mut dot = 0.0_f64;
            let mut energy = 0.0_f64;
            for (index, &value) in template.iter().enumerate() {
                let candidate = mono(candidate_start + index as f64);
                dot += value * candidate;
                energy += candidate * candidate;
            }
            if energy <= f64::EPSILON {
                continue;
            }
            let score = dot / energy.sqrt();
            // 周期性信号会让多个偏移得到相同分数（1kHz 正弦在 4ms 窗内正好是整数个周期），
            // 此时取最接近名义位置的那个：名义位置就是「不伸缩时应有的内容位置」。
            let better =
                score > best_score || (score == best_score && offset.abs() < best_offset.abs());
            if better {
                best_score = score;
                best_offset = offset;
            }
        }
        best_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 48kHz 输出下的测试用源采样率。
    const RATE: u32 = 48_000;

    /// 生成 `frames` 帧的立体声正弦；左右声道相同，便于用单声道公式断言。
    fn sine_source(frames: usize, frequency: f64) -> SampleData {
        let samples: Vec<f32> = (0..frames)
            .flat_map(|index| {
                let value =
                    (2.0 * std::f64::consts::PI * frequency * index as f64 / RATE as f64).sin();
                [value as f32, value as f32]
            })
            .collect();
        SampleData::stereo(samples, RATE)
    }

    /// 生成 `frames` 帧的立体声线性斜坡（每帧值 = 帧号 / 1000）。
    fn ramp_source(frames: usize) -> SampleData {
        let samples: Vec<f32> = (0..frames)
            .flat_map(|index| {
                let value = index as f32 / 1000.0;
                [value, value]
            })
            .collect();
        SampleData::stereo(samples, RATE)
    }

    /// 恒等倍率（`resample == 1`）下直接按「内容毫秒 × 源采样率 / 1000」取样。
    ///
    /// 只用于恒等倍率的断言；非 1 倍率的取样步长由 [`MusicPlayer`] 的
    /// `advance_ms` 决定，见 `resample_only_advances_content_by_the_rate`。
    fn expected_identity(source: &SampleData, content_ms: f64, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|index| {
                let frame_time = content_ms + index as f64 * 1000.0 / RATE as f64;
                let (left, right) = source.sample_at(frame_time * RATE as f64 / 1000.0);
                [left, right]
            })
            .collect()
    }

    /// 恒等倍率与直接取样逐位一致（无 Mod 的输出不能变）。
    #[test]
    fn identity_rate_matches_direct_sampling() {
        let source = ramp_source(4800);
        let mut player = MusicPlayer::new(RATE);
        let mut output = vec![0.0_f32; 512 * 2];
        player.render_at(-100.0, &source, RATE, 512, &mut output);
        let expected = expected_identity(&source, -100.0, 512);
        assert_eq!(output, expected);
    }

    /// 纯重采样（stretch = 1）按倍率推进内容，取样步长不再额外乘倍率。
    ///
    /// 恒等以外的倍率下，音高与内容速率是同一个量：源帧位置 = 内容毫秒 × 源采样率 / 1000，
    /// 而内容毫秒本身按 `resample * 1000 / output_rate` 每帧推进。
    #[test]
    fn resample_only_advances_content_by_the_rate() {
        let source = sine_source(48_000, 440.0);
        let mut player = MusicPlayer::new(RATE);
        player.set_rate(MusicRate {
            resample: 1.5,
            stretch: 1.0,
        });
        let mut output = vec![0.0_f32; 256 * 2];
        player.render_at(250.0, &source, RATE, 256, &mut output);
        let step_ms = 1.5 * 1000.0 / RATE as f64;
        for (index, pair) in output.chunks_exact(2).enumerate() {
            let frame_time = 250.0 + index as f64 * step_ms;
            let (left, right) = source.sample_at(frame_time * RATE as f64 / 1000.0);
            assert_eq!(pair[0], left, "帧 {index}");
            assert_eq!(pair[1], right, "帧 {index}");
        }
    }

    /// 纯重采样（stretch = 1）按倍率推进内容：源里的静音缺口出现在「缺口位置 / 倍率」处。
    ///
    /// 这是 NC/DC 默认速度走的路径（不调用 WSOLA），缺口位置只由 `resample` 决定，
    /// 长度按 1/倍率缩放；两侧都要测：倍率 > 1 压缩、< 1 拉长。
    #[test]
    fn resample_only_moves_content_by_the_rate() {
        let source_frames = 2_000;
        let samples: Vec<f32> = (0..source_frames)
            .flat_map(|index| {
                let value = if (800..900).contains(&index) {
                    0.0
                } else {
                    (index as f64 * 0.2).sin() as f32
                };
                [value, value]
            })
            .collect();
        let source = SampleData::stereo(samples, 1_000);

        for resample in [0.75_f64, 1.5] {
            let output_frames = (source_frames as f64 / resample).ceil() as usize;
            let mut player = MusicPlayer::new(1_000);
            player.set_rate(MusicRate {
                resample,
                stretch: 1.0,
            });
            let mut output = vec![1.0_f32; output_frames * 2];
            player.render_at(0.0, &source, 1_000, output_frames, &mut output);

            let silent: Vec<usize> = output
                .chunks_exact(2)
                .enumerate()
                .filter(|(_, pair)| pair[0].abs() < 1e-6)
                .map(|(index, _)| index)
                .collect();
            // 正弦自身也有过零点，因此只认「连续多次」的静音段（缺口至少 100 帧长）。
            let mut runs: Vec<(usize, usize)> = Vec::new();
            for index in silent {
                match runs.last_mut() {
                    Some(last) if last.1 + 1 == index => last.1 = index,
                    _ => runs.push((index, index)),
                }
            }
            let (gap_start, gap_end) = *runs
                .iter()
                .max_by_key(|(start, end)| end - start)
                .expect("必须能找到静音缺口");
            let expected_start = 800.0 / resample;
            let expected_len = 100.0 / resample;
            assert!(
                (gap_start as f64 - expected_start).abs() < 3.0,
                "倍率 {resample}：缺口起点 {gap_start}，预期 {expected_start}"
            );
            assert!(
                (gap_end as f64 + 1.0 - (expected_start + expected_len)).abs() < 5.0,
                "倍率 {resample}：缺口终点 {}，预期 {}",
                gap_end + 1,
                expected_start + expected_len
            );
        }
    }

    /// 源采样率与输出采样率不同（44.1kHz 源、48kHz 输出）时，内容速率仍只由倍率决定。
    ///
    /// 导出链路就是这种组合；这里用「源里 1 秒处的静音缺口」直接量内容速率，
    /// 避免只靠公式自证。
    #[test]
    fn resample_rate_holds_for_mismatched_sample_rates() {
        let source_rate = 44_100_u32;
        let output_rate = 48_000_u32;
        let source_frames = source_rate as usize * 2;
        let gap_start = source_rate as usize;
        let gap_len = source_rate as usize / 10;
        let samples: Vec<f32> = (0..source_frames)
            .flat_map(|index| {
                let value = if (gap_start..gap_start + gap_len).contains(&index) {
                    0.0
                } else {
                    (index as f64 * 0.2).sin() as f32
                };
                [value, value]
            })
            .collect();
        let source = SampleData::stereo(samples, source_rate);

        for resample in [0.75_f64, 1.5] {
            let output_frames = (output_rate as f64 * 2.0 / resample).ceil() as usize;
            let mut player = MusicPlayer::new(output_rate);
            player.set_rate(MusicRate {
                resample,
                stretch: 1.0,
            });
            let mut output = vec![1.0_f32; output_frames * 2];
            player.render_at(0.0, &source, source_rate, output_frames, &mut output);

            let silent: Vec<usize> = output
                .chunks_exact(2)
                .enumerate()
                .filter(|(_, pair)| pair[0].abs() < 1e-6)
                .map(|(index, _)| index)
                .collect();
            let mut runs: Vec<(usize, usize)> = Vec::new();
            for index in silent {
                match runs.last_mut() {
                    Some(last) if last.1 + 1 == index => last.1 = index,
                    _ => runs.push((index, index)),
                }
            }
            let (start, end) = *runs
                .iter()
                .max_by_key(|(start, end)| end - start)
                .expect("必须能找到静音缺口");
            let expected_start = output_rate as f64 / resample;
            let expected_len = (output_rate as f64 / 10.0) / resample;
            assert!(
                (start as f64 - expected_start).abs() < 5.0,
                "倍率 {resample}：缺口起点 {start}，预期 {expected_start}"
            );
            assert!(
                (end as f64 + 1.0 - (expected_start + expected_len)).abs() < 10.0,
                "倍率 {resample}：缺口终点 {}，预期 {}",
                end + 1,
                expected_start + expected_len
            );
        }
    }

    /// 时间伸缩保调：1kHz 正弦被拉伸后主频仍是 1kHz，而不是 1.5kHz。
    #[test]
    fn time_stretch_preserves_pitch() {
        let frequency = 1_000.0;
        let source = sine_source(48_000, frequency);
        let mut player = MusicPlayer::new(RATE);
        player.set_rate(MusicRate {
            resample: 1.0,
            stretch: 1.5,
        });
        // 从内容 200ms 起取 0.5 秒（源只有 1 秒），跳过开头的淡入。
        let frames = 24_000;
        let mut output = vec![0.0_f32; frames * 2];
        player.render_at(200.0, &source, RATE, frames, &mut output);
        let peak = output
            .iter()
            .fold(0.0_f32, |acc, value| acc.max(value.abs()));
        assert!(peak > 0.3, "伸缩后输出几乎无声（peak={peak}）");

        let at_pitch = tone_magnitude(&output, frequency);
        let at_stretched = tone_magnitude(&output, frequency * 1.5);
        assert!(
            at_pitch > at_stretched * 8.0,
            "1kHz 幅值 {at_pitch} 未显著高于 1.5kHz 幅值 {at_stretched}"
        );
    }

    /// 内容速率精确：整段 1 秒的源被 1.5 倍速消费后，输出约 2/3 秒就进入静音。
    #[test]
    fn time_stretch_keeps_content_rate_exact() {
        let source = sine_source(RATE as usize, 1_000.0);
        let mut player = MusicPlayer::new(RATE);
        player.set_rate(MusicRate {
            resample: 1.0,
            stretch: 1.5,
        });
        let frames = RATE as usize;
        let mut output = vec![0.0_f32; frames * 2];
        player.render_at(0.0, &source, RATE, frames, &mut output);

        let last_loud = output
            .chunks_exact(2)
            .rposition(|pair| pair[0].abs() > 1e-3)
            .expect("源有内容，输出不应全静音");
        let expected = (RATE as f64 / 1.5) as usize;
        let tolerance = 2_000;
        assert!(
            last_loud + tolerance >= expected && last_loud <= expected + tolerance,
            "内容在 {last_loud} 帧结束，预期约 {expected} 帧"
        );
    }

    /// 分块渲染与整段渲染逐位一致（宿主/编码器都是分块调用的）。
    #[test]
    fn chunked_rendering_matches_single_call() {
        let source = sine_source(96_000, 660.0);
        let rate = MusicRate {
            resample: 1.25,
            stretch: 0.75,
        };
        let total = 3_000;

        let mut whole = MusicPlayer::new(RATE);
        whole.set_rate(rate);
        let mut whole_output = vec![0.0_f32; total * 2];
        whole.render_at(0.0, &source, RATE, total, &mut whole_output);

        let mut chunked = MusicPlayer::new(RATE);
        chunked.set_rate(rate);
        let mut chunked_output = vec![0.0_f32; total * 2];
        let advance = rate.stretch * rate.resample * 1000.0 / RATE as f64;
        let mut position = 0usize;
        for chunk in [7usize, 1, 999, 512, 3, 900, 578] {
            if position >= total {
                break;
            }
            let frames = chunk.min(total - position);
            let content_ms = advance * position as f64;
            let start = position * 2;
            chunked.render_at(
                content_ms,
                &source,
                RATE,
                frames,
                &mut chunked_output[start..start + frames * 2],
            );
            position += frames;
        }
        assert_eq!(whole_output, chunked_output);
    }

    /// 非有限或非正的倍率回退恒等，不 panic、不产生 NaN。
    #[test]
    fn invalid_rates_fall_back_to_identity() {
        for (resample, stretch) in [
            (f64::NAN, 1.0),
            (1.0, f64::NAN),
            (f64::INFINITY, 1.5),
            (0.0, 1.0),
            (-1.0, 1.0),
            (1.0, 0.0),
        ] {
            let rate = MusicRate { resample, stretch }.sanitized();
            assert_eq!(
                rate,
                MusicRate::IDENTITY,
                "resample={resample}, stretch={stretch}"
            );
        }
        assert!(MusicRate::output_domain(f64::NAN, 1.0).is_identity());
        assert!(MusicRate::chart_domain(1.5, f64::INFINITY).is_identity());

        let source = ramp_source(4_800);
        let mut player = MusicPlayer::new(RATE);
        player.set_rate(MusicRate {
            resample: -3.0,
            stretch: f64::NAN,
        });
        assert!(player.rate().is_identity());
        let mut output = vec![0.0_f32; 64 * 2];
        player.render_at(0.0, &source, RATE, 64, &mut output);
        assert!(output.iter().all(|value| value.is_finite()));
    }

    /// 越界与负时间按静音处理；曲末之后不产生噪声。
    #[test]
    fn out_of_range_content_is_silent() {
        let source = sine_source(RATE as usize, 500.0);
        let mut player = MusicPlayer::new(RATE);
        player.set_rate(MusicRate {
            resample: 1.0,
            stretch: 1.5,
        });
        // 预卷（负内容时间）：整段静音。
        let mut output = vec![0.0_f32; 1_000 * 2];
        player.render_at(-2_000.0, &source, RATE, 1_000, &mut output);
        assert!(output.iter().all(|value| *value == 0.0), "{output:?}");

        // 曲末之后：允许有交叉淡变的余量，但逐渐归零。
        player.render_at(3_000.0, &source, RATE, 1_000, &mut output);
        let tail = &output[output.len() - 200..];
        let peak = tail.iter().fold(0.0_f32, |acc, value| acc.max(value.abs()));
        assert!(peak < 1e-3, "曲末尾部仍有 {peak} 的残留");
    }

    /// 单声道源左右一致；立体声源插值不越界。
    #[test]
    fn mono_source_renders_both_channels() {
        let mono = SampleData::mono(
            (0..2_000).map(|index| index as f32 / 2_000.0).collect(),
            RATE,
        );
        let mut player = MusicPlayer::new(RATE);
        player.set_rate(MusicRate {
            resample: 1.0,
            stretch: 2.0,
        });
        let mut output = vec![0.0_f32; 500 * 2];
        player.render_at(0.0, &mono, RATE, 500, &mut output);
        for pair in output.chunks_exact(2) {
            assert!((pair[0] - pair[1]).abs() < 1e-6, "{pair:?}");
        }
        assert!(output[100..].iter().any(|value| value.abs() > 1e-3));
    }

    /// 换速后重新起段：位置由调用方给定，不复用旧状态。
    #[test]
    fn rate_change_restarts_at_the_requested_position() {
        let source = sine_source(96_000, 880.0);
        let mut player = MusicPlayer::new(RATE);
        player.set_rate(MusicRate {
            resample: 1.0,
            stretch: 1.5,
        });
        let mut output = vec![0.0_f32; 1_000 * 2];
        player.render_at(0.0, &source, RATE, 1_000, &mut output);

        player.set_rate(MusicRate::IDENTITY);
        // 换速后即使内容时间连续，也要按当前倍率重新读取。
        let mut after = vec![0.0_f32; 64 * 2];
        player.render_at(500.0, &source, RATE, 64, &mut after);
        assert_eq!(after, expected_identity(&source, 500.0, 64));
    }

    /// 图表域/输出域换算：无 Mod 与 NC/DC 默认速度都不需要时间伸缩。
    #[test]
    fn domain_rates_match_the_game_semantics() {
        // 无 Mod。
        assert!(MusicRate::output_domain(1.0, 1.0).is_identity());
        assert!(MusicRate::chart_domain(1.0, 1.0).is_identity());
        // DT/HT：保调，平均内容速率 = 速度。
        let dt = MusicRate::output_domain(1.5, 1.0);
        assert_eq!(
            dt,
            MusicRate {
                resample: 1.0,
                stretch: 1.5
            }
        );
        assert!((dt.resample * dt.stretch - 1.5).abs() < 1e-9);
        // 实时端：工作台域要抵消宿主总倍率（1.5），只留下「保调」。
        let chart = MusicRate::chart_domain(1.5, 1.0);
        assert!((chart.resample * chart.stretch - 1.0).abs() < 1e-9);
        assert_eq!(
            chart,
            MusicRate {
                resample: 1.0 / 1.5,
                stretch: 1.5
            }
        );
        // NC 默认速度：纯重采样，不需要时间伸缩。
        let nc = MusicRate::output_domain(1.5, 1.5);
        assert_eq!(
            nc,
            MusicRate {
                resample: 1.5,
                stretch: 1.0
            }
        );
        // NC 非默认速度：仍是固定音高，靠时间伸缩补齐速度。
        let nc_fast = MusicRate::output_domain(2.0, 1.5);
        assert_eq!(
            nc_fast,
            MusicRate {
                resample: 1.5,
                stretch: 2.0 / 1.5
            }
        );
        // DC 默认速度同样不需要时间伸缩。
        let dc = MusicRate::chart_domain(0.75, 0.75);
        assert_eq!(
            dc,
            MusicRate {
                resample: 1.0,
                stretch: 1.0
            }
        );
    }

    /// 单个频率上的幅度（单个 DFT 频点），用于「保调」断言。
    ///
    /// 直接用相位递推的 Goertzel 在数万样本上会累积浮点误差（功率甚至可能算出负数），
    /// 这里用显式 cos/sin 相关求和，测试里不需要考虑速度。
    fn tone_magnitude(samples: &[f32], frequency: f64) -> f64 {
        let omega = 2.0 * std::f64::consts::PI * frequency / RATE as f64;
        let (mut real, mut imaginary) = (0.0_f64, 0.0_f64);
        let mut frames = 0_u64;
        for (index, pair) in samples.chunks_exact(2).enumerate() {
            let value = pair[0] as f64;
            let phase = omega * index as f64;
            real += value * phase.cos();
            imaginary += value * phase.sin();
            frames += 1;
        }
        if frames == 0 {
            return 0.0;
        }
        (real * real + imaginary * imaginary).sqrt() / frames as f64
    }
}
