//! 打击音混音器：把时间轴上的事件混合成 PCM。

use super::{HitsoundTimeline, SampleLibrary};

/// 正在播放的一个声音。
#[derive(Debug, Clone, Copy)]
struct Voice {
    /// 声音句柄：显式触发（游玩/回放）时由调用方持有，用于停止循环音。
    id: u64,
    source_id: usize,
    gain: f64,
    /// 事件在时间轴上的起始毫秒。
    start_ms: f64,
    /// 事件结束毫秒；无限表示按样本自身长度播放。
    end_ms: f64,
    /// 样本数据播完的毫秒时刻（循环音为无限，因为它会一直绕回开头）。
    ///
    /// 非循环音的 `end_ms` 是无限（时长 0 表示只播一次样本），只用 `end_ms` 判断
    /// 是否回收会让声音列表随播放不断增长——每个输出帧都要遍历整张列表，长谱面会
    /// 把混音拖到实时以下（Web 端表现为打击音整体消失）。
    data_end_ms: f64,
    /// 循环音在样本内的循环长度（采样帧）；0 表示不循环。
    loop_len: usize,
}

/// 循环音的句柄，用于停止 [`HitsoundMixer::start_loop`] 启动的声音。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LoopHandle(u64);

/// 基于时间轴与样本库的离线混音器。
///
/// 混音位置以「谱面毫秒」为单位，与渲染共用同一条时间轴；输出为固定采样率的
/// 立体声交错 f32，宿主只需把结果送到音频接口或编码器。
///
/// 声音有两个来源：时间轴事件（预览/导出）与显式触发（游玩、回放由输入驱动）。
/// 两者写进同一张声音列表，因此增益、限幅与回收逻辑只有一份。
#[derive(Debug, Clone)]
pub struct HitsoundMixer {
    library: SampleLibrary,
    timeline: HitsoundTimeline,
    sample_rate: u32,
    master_gain: f64,
    /// 当前混音位置（谱面毫秒）。
    position_ms: f64,
    voices: Vec<Voice>,
    /// 下一个尚未开始的事件索引。
    next_event: usize,
    /// 声音句柄计数。
    next_voice_id: u64,
}

impl HitsoundMixer {
    pub fn new(library: SampleLibrary, timeline: HitsoundTimeline, sample_rate: u32) -> Self {
        Self {
            library,
            timeline,
            sample_rate: sample_rate.max(1),
            master_gain: 1.0,
            position_ms: 0.0,
            voices: Vec::new(),
            next_event: 0,
            next_voice_id: 1,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn master_gain(&self) -> f64 {
        self.master_gain
    }

    /// 设置主音量（线性增益）。非有限值按静音处理。
    pub fn set_master_gain(&mut self, gain: f64) {
        self.master_gain = if gain.is_finite() {
            gain.clamp(0.0, 8.0)
        } else {
            0.0
        };
    }

    pub fn timeline(&self) -> &HitsoundTimeline {
        &self.timeline
    }

    pub fn library(&self) -> &SampleLibrary {
        &self.library
    }

    /// 可变访问样本库。
    ///
    /// 放入多个样本时应先全部放完，再调用一次 [`HitsoundMixer::rebuild_timeline`]：
    /// 时间轴重建会遍历整张谱面，逐样本重建在样本多时是明显的浪费。
    pub fn library_mut(&mut self) -> &mut SampleLibrary {
        &mut self.library
    }

    /// 按音频事件顺序重建时间轴事件游标（保留当前播放位置）。
    pub fn rebuild_timeline_events(&mut self, timeline: HitsoundTimeline) {
        self.timeline = timeline;
        self.next_event = self
            .timeline
            .events
            .partition_point(|event| event.start_ms < self.position_ms);
        self.voices.clear();
    }

    /// 用当前样本库重新生成事件时间轴。
    ///
    /// 会清空正在播放的声音：宿主应在加载样本的阶段调用，而不是播放中途。
    pub fn rebuild_timeline(&mut self, beatmap: &crate::domain::models::Beatmap) {
        let timeline = super::build_timeline(beatmap, &self.library);
        self.rebuild_timeline_events(timeline);
    }

    pub fn position_ms(&self) -> f64 {
        self.position_ms
    }

    /// 跳转到指定位置：丢弃所有正在播放的声音，并重新定位事件游标。
    ///
    /// 播放中途 seek 时已经越过的事件不会再补播，避免瞬间堆积大量声音。
    pub fn seek(&mut self, position_ms: f64) {
        self.voices.clear();
        self.set_position(position_ms);
    }

    /// 只移动播放位置与事件游标，已开始的声音继续播放。
    ///
    /// 用于流式渲染时「把混音位置对齐到宿主提供的起点」：宿主负责决定要不要
    /// 清空声音（真正的 seek 应调用 [`HitsoundMixer::seek`]）。
    ///
    /// 位置单位是谱面绝对时间，**允许为负**：离线导出可能从「首个物件前的预卷」开始
    /// （视频区间起点为负），此时缓冲区第 0 帧必须对应那个负时刻，否则整段打击音
    /// 会相对音乐与画面提前 |起点|。只有非有限值才回退到 0。
    pub fn set_position(&mut self, position_ms: f64) {
        self.position_ms = if position_ms.is_finite() {
            position_ms
        } else {
            0.0
        };
        self.next_event = self
            .timeline
            .events
            .partition_point(|event| event.start_ms < self.position_ms);
    }

    /// 单调推进事件游标（不改变播放位置）。
    ///
    /// 流式渲染允许宿主从头重放同一个窗口（例如音频设备重排缓冲区），此时位置可能
    /// 回退；这个方法保证已经排入的事件不会被重复触发。
    pub fn advance_cursor_to(&mut self, position_ms: f64) {
        if !position_ms.is_finite() {
            return;
        }
        let index = self
            .timeline
            .events
            .partition_point(|event| event.start_ms < position_ms);
        self.next_event = self.next_event.max(index);
    }

    /// 清空所有正在播放的声音，但保留当前位置。
    pub fn stop_all(&mut self) {
        self.voices.clear();
    }

    /// 立即触发一个按名字查找的样本（只播一次）。
    ///
    /// 供游玩/回放使用：打击音由玩家输入驱动，而不是由时间轴驱动。声音从**当前
    /// 混音位置**开始；返回 `false` 表示样本库没有这个名字（按静音处理），调用方
    /// 不需要把它当成错误。
    pub fn trigger(&mut self, name: &str, gain: f64) -> bool {
        match self.library.id_of(name) {
            Some(source_id) => self.trigger_source(source_id, gain),
            None => false,
        }
    }

    /// 立即触发一个已在样本库里的样本 id（只播一次）。
    pub fn trigger_source(&mut self, source_id: usize, gain: f64) -> bool {
        let Some((frames, sample_rate)) = self.source_shape(source_id) else {
            return false;
        };
        if frames == 0 {
            return false;
        }
        let start_ms = self.position_ms;
        let id = self.take_voice_id();
        self.voices.push(Voice {
            id,
            source_id,
            gain: sanitize_gain(gain),
            start_ms,
            end_ms: f64::INFINITY,
            data_end_ms: start_ms + frames as f64 * 1000.0 / sample_rate,
            loop_len: 0,
        });
        true
    }

    /// 开始一个循环音（滑条滑行、转盘旋转，或游玩时按住不放的持续音）。
    ///
    /// 样本自带循环长度时用它，否则整段样本循环。返回的句柄交给
    /// [`HitsoundMixer::stop_loop`]；名字不存在或样本为空时返回 `None`。
    pub fn start_loop(&mut self, name: &str, gain: f64) -> Option<LoopHandle> {
        let source_id = self.library.id_of(name)?;
        let (frames, _) = self.source_shape(source_id)?;
        if frames == 0 {
            return None;
        }
        let loop_len = self
            .library
            .get(name)
            .map(|source| source.loop_len.min(frames))
            .filter(|loop_len| *loop_len > 0)
            .unwrap_or(frames);
        let start_ms = self.position_ms;
        let id = self.take_voice_id();
        self.voices.push(Voice {
            id,
            source_id,
            gain: sanitize_gain(gain),
            start_ms,
            // 循环音没有自然结束点：一直播到 stop_loop 把 end_ms 设到当前位置。
            end_ms: f64::INFINITY,
            data_end_ms: f64::INFINITY,
            loop_len,
        });
        Some(LoopHandle(id))
    }

    /// 停止由 [`HitsoundMixer::start_loop`] 启动的声音（当前位置立刻静音）。
    pub fn stop_loop(&mut self, handle: LoopHandle) {
        let position = self.position_ms;
        for voice in &mut self.voices {
            if voice.id == handle.0 {
                voice.end_ms = position;
            }
        }
    }

    /// 样本 id 的（帧数, 采样率）；id 不存在时返回 `None`。
    fn source_shape(&self, source_id: usize) -> Option<(usize, f64)> {
        self.library
            .sources
            .get(source_id)
            .map(|source| (source.frames(), source.sample_rate.max(1) as f64))
    }

    fn take_voice_id(&mut self) -> u64 {
        let id = self.next_voice_id;
        // 句柄只需要在本会话内唯一；绕回时跳过 0，避免和「没有句柄」混淆。
        self.next_voice_id = self.next_voice_id.wrapping_add(1).max(1);
        id
    }

    /// 当前仍在播放的声音数量（只给测试用的回收回归）。
    #[cfg(test)]
    fn voice_count(&self) -> usize {
        self.voices.len()
    }

    /// 从当前混音位置渲染 `frames` 个立体声采样帧，返回交错的双声道数据，并前进位置。
    pub fn render(&mut self, frames: usize) -> Vec<f32> {
        let mut output = vec![0.0_f32; frames * 2];
        self.render_into(&mut output);
        output
    }

    /// 把 `frames * 2` 个交错采样写入 `output`（长度不足时只写能写下的部分）。
    pub fn render_into(&mut self, output: &mut [f32]) {
        let frames = output.len() / 2;
        if frames == 0 {
            self.position_ms += frames as f64 * 1000.0 / self.sample_rate as f64;
            return;
        }

        let window_start = self.position_ms;
        let ms_per_frame = 1000.0 / self.sample_rate as f64;
        let window_end = window_start + frames as f64 * ms_per_frame;

        // 收集本窗口内新开始的事件。
        //
        // 窗口是 `[start, end)`：正好落在 `window_end` 的事件留给下一个窗口。相邻窗口
        // 首尾相接，所以它不会丢；反过来（在 `<=` 时收进来）会因为宿主每个窗口都用
        // 窗口起点重新对齐事件游标，让同一个事件被收进两个声音、音量凭空翻倍。
        while self.next_event < self.timeline.events.len() {
            let event = self.timeline.events[self.next_event];
            if event.start_ms >= window_end {
                break;
            }
            self.next_event += 1;
            let (loop_len, data_end_ms) = match self.library.sources.get(event.source_id) {
                Some(source) if event.looping => (source.loop_len, f64::INFINITY),
                Some(source) if source.frames() > 0 => (
                    0,
                    event.start_ms
                        + source.frames() as f64 * 1000.0 / source.sample_rate.max(1) as f64,
                ),
                // 缺失或空样本：立刻结束，不占用声音列表。
                _ => (0, event.start_ms),
            };
            let id = self.take_voice_id();
            self.voices.push(Voice {
                id,
                source_id: event.source_id,
                gain: event.gain,
                start_ms: event.start_ms,
                end_ms: if event.duration_ms > 0.0 {
                    event.start_ms + event.duration_ms
                } else {
                    f64::INFINITY
                },
                data_end_ms,
                loop_len,
            });
        }

        let master = self.master_gain;
        for (index, pair) in output.chunks_exact_mut(2).enumerate() {
            let frame_time = window_start + index as f64 * ms_per_frame;
            let mut left = 0.0_f32;
            let mut right = 0.0_f32;

            for voice in &self.voices {
                if frame_time < voice.start_ms || frame_time >= voice.end_ms {
                    continue;
                }
                let Some(source) = self.library.sources.get(voice.source_id) else {
                    continue;
                };
                let frames_in_source = source.frames();
                if frames_in_source == 0 {
                    continue;
                }
                let rate = source.sample_rate.max(1) as f64;
                let loop_len = voice.loop_len.min(frames_in_source);
                let playable = if loop_len > 0 {
                    loop_len
                } else {
                    frames_in_source
                };

                // 直接按时间反推样本位置：与画面共用同一时间轴，避免累计漂移。
                let mut position = (frame_time - voice.start_ms) * rate / 1000.0;
                if position < 0.0 {
                    continue;
                }
                if loop_len > 0 {
                    position = position.rem_euclid(playable as f64);
                } else if position >= playable as f64 {
                    continue;
                }

                let (sample_left, sample_right) = source.frame(position as usize);
                let gain = (voice.gain * master) as f32;
                left += sample_left * gain;
                right += sample_right * gain;
            }

            pair[0] = soft_limit(left);
            pair[1] = soft_limit(right);
        }

        // 播放完的声音在下一窗口便宜地回收。
        //
        // 本窗口才开始的声音先留着（它们的数据可能正好跨到下一个窗口），其余按「数据
        // 播完的时刻」判断：循环音看 `end_ms`（滑条/转盘的持续时长），普通打击音看
        // `data_end_ms`。不做这一步，时长 0 的事件（`end_ms` 是无限）会永远留在列表里，
        // 每个输出帧都要遍历一遍，长谱面会把混音拖到实时以下。
        self.voices.retain(|voice| {
            voice.start_ms >= window_start || voice.end_ms.min(voice.data_end_ms) > window_end
        });

        self.position_ms = window_end;
    }
}

/// 触发用的增益：非有限值按静音处理，其余夹到与主音量一致的范围内。
fn sanitize_gain(gain: f64) -> f64 {
    if gain.is_finite() {
        gain.clamp(0.0, 8.0)
    } else {
        0.0
    }
}

/// 软限幅：小信号近似线性，大信号平滑压缩到 ±1 以内。
///
/// osu! 允许打击音叠加，直接求和会削波；这里用有理函数近似 `tanh`，
/// 比逐样本调用 `tanh` 便宜得多，且两端行为一致。
#[inline]
pub(super) fn soft_limit(value: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    let x = value.clamp(-4.0, 4.0);
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hitsound::{PlayEvent, SampleData};

    /// 构造一个「每 100ms 一个事件、样本 10ms」的样本库与时间轴。
    ///
    /// 事件间隔与窗口长度相同，因此每个事件都正好落在窗口边界上——这正是两条回归
    /// 最容易被踩到的位置。
    fn click_mixer(events: usize) -> HitsoundMixer {
        let mut library = SampleLibrary::new();
        // 1kHz 采样率，10 帧 = 10ms；样本值 1.0，增益 0.1，仍在软限幅的线性区。
        library.insert("click", SampleData::stereo(vec![1.0; 20], 1000));
        let timeline = HitsoundTimeline {
            events: (0..events)
                .map(|index| PlayEvent {
                    start_ms: index as f64 * 100.0,
                    duration_ms: 0.0,
                    source_id: 0,
                    gain: 0.1,
                    looping: false,
                })
                .collect(),
        };
        HitsoundMixer::new(library, timeline, 1000)
    }

    #[test]
    fn 播完的声音会被回收() {
        // 回归：时长 0 的事件 `end_ms` 是无限，只按它判断会让声音列表随播放无限增长；
        // 每个输出帧都要遍历整张列表，长谱面会把混音拖到实时以下（Web 端表现为打击音
        // 整体消失）。
        let mut mixer = click_mixer(200);
        for window in 0..200 {
            // 与宿主一致：每个窗口用窗口起点重新对齐事件游标。
            mixer.set_position((window * 100) as f64);
            mixer.render(100);
        }
        assert!(
            mixer.voice_count() <= 2,
            "声音列表没有回收：{} 个声音仍挂着",
            mixer.voice_count()
        );
    }

    #[test]
    fn 落在窗口边界的事件只播一次() {
        // 窗口是 `[start, end)`：正好在窗口末尾开始的事件必须留给下一个窗口。宿主每个
        // 窗口都会用窗口起点重新对齐游标，边界事件若被收进两个窗口，音量会凭空翻倍。
        let mut mixer = click_mixer(11);
        let mut peak = 0.0_f32;
        for window in 0..20 {
            mixer.set_position((window * 100) as f64);
            for value in mixer.render(100) {
                peak = peak.max(value.abs());
            }
        }
        let expected = soft_limit(0.1);
        assert!(
            (peak - expected).abs() < 1e-6,
            "边界事件被重复播放：peak={peak} expected={expected}"
        );
    }

    #[test]
    fn 显式触发的样本从当前位置开始出声() {
        // 游玩/回放场景：没有时间轴事件，声音完全由输入触发。
        let mut mixer = click_mixer(0);
        mixer.set_position(500.0);
        assert!(mixer.trigger("click", 0.1));
        assert!(!mixer.trigger("不存在的样本", 0.1));
        let peak = mixer
            .render(20)
            .iter()
            .fold(0.0_f32, |peak, value| peak.max(value.abs()));
        let expected = soft_limit(0.1);
        assert!(
            (peak - expected).abs() < 1e-6,
            "peak={peak} expected={expected}"
        );
    }

    #[test]
    fn 循环音可以启动与停止() {
        let mut mixer = click_mixer(0);
        let handle = mixer.start_loop("click", 0.1).expect("循环音必须能启动");
        // 样本只有 10ms，但循环音要一直响到 stop_loop。
        assert!(mixer.render(50).iter().any(|value| *value != 0.0));
        assert!(mixer.render(50).iter().any(|value| *value != 0.0));
        mixer.stop_loop(handle);
        assert!(
            mixer.render(50).iter().all(|value| *value == 0.0),
            "停止后仍然出声"
        );
        assert!(mixer.start_loop("不存在的样本", 0.1).is_none());
    }

    #[test]
    fn 触发增益非有限值按静音处理() {
        let mut mixer = click_mixer(0);
        assert!(mixer.trigger("click", f64::NAN));
        assert!(mixer.render(50).iter().all(|value| *value == 0.0));
    }

    #[test]
    fn 负位置把起点之前的预卷算进输出() {
        // 离线导出可能从负的谱面时间开始（首个物件前的预卷）：位置为负时缓冲区第 0 帧
        // 对应那个负时刻，0 之后的事件必须相应推后，否则整段打击音会提前。
        let mut mixer = click_mixer(1);
        mixer.seek(-500.0);
        let output = mixer.render(1000);
        assert!(
            output[..500 * 2].iter().all(|value| *value == 0.0),
            "预卷期间不应有声音"
        );
        assert!(
            output[500 * 2].abs() > 0.0,
            "事件应当出现在缓冲区第 500 帧"
        );
    }
}
