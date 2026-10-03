//! 打击音混音器：把时间轴上的事件混合成 PCM。

use super::music::{MusicPlayer, MusicRate};
use super::{HitsoundTimeline, PlayFrequency, SampleData, SampleLibrary};

/// 正在播放的一个声音。
///
/// `end_ms` 是发声区间终点（无限 = 按样本长度播或直到显式停止）；`data_end_ms` 是
/// 样本数据播完的时刻（循环音为无限）。回收取两者较小值：时长 0 的事件 `end_ms`
/// 是无限，只看它会让声音列表无限增长，长谱面混音会掉到实时以下。
#[derive(Debug, Clone, Copy)]
struct Voice {
    id: u64,
    source_id: usize,
    gain: f64,
    start_ms: f64,
    end_ms: f64,
    data_end_ms: f64,
    /// 循环长度（采样帧）；0 表示不循环。
    loop_len: usize,
    frequency: PlayFrequency,
}

/// 循环音的句柄，用于停止 [`HitsoundMixer::start_loop`] 启动的声音。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LoopHandle(u64);

/// 基于时间轴与样本库的混音器：位置单位是谱面毫秒（与渲染共用同一条时间轴），
/// 输出固定采样率的交错立体声 f32。
///
/// 声音来自时间轴事件与显式触发（游玩/回放）；背景音乐是第三个声部，不在声音
/// 列表里、按位置逐帧采样叠加，倍速、seek、暂停共用同一处位置计算。
#[derive(Debug, Clone)]
pub struct HitsoundMixer {
    library: SampleLibrary,
    timeline: HitsoundTimeline,
    sample_rate: u32,
    master_gain: f64,
    /// 背景音乐：谱面时间 0 对应音乐第 0 帧，整段只播一次、不循环。
    music: Option<SampleData>,
    music_rate: MusicRate,
    music_player: MusicPlayer,
    /// 音乐窗口缓冲（交错立体声）：时间伸缩有状态，必须整窗口渲染后逐帧相加。
    music_scratch: Vec<f32>,
    music_gain: f64,
    position_ms: f64,
    voices: Vec<Voice>,
    next_event: usize,
    next_voice_id: u64,
}

impl HitsoundMixer {
    pub fn new(library: SampleLibrary, timeline: HitsoundTimeline, sample_rate: u32) -> Self {
        let sample_rate = sample_rate.max(1);
        Self {
            library,
            timeline,
            sample_rate,
            master_gain: 1.0,
            music: None,
            music_rate: MusicRate::IDENTITY,
            music_player: MusicPlayer::new(sample_rate),
            music_scratch: Vec::new(),
            music_gain: 1.0,
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
        self.master_gain = sanitize_gain(gain);
    }

    /// 设置背景音乐。
    pub fn set_music(&mut self, music: Option<SampleData>) {
        self.music = music;
    }

    pub fn music(&self) -> Option<&SampleData> {
        self.music.as_ref()
    }

    /// 设置音乐的变速保调倍率；变化会清空时间伸缩状态，下次渲染按当前位置重新起段。
    pub fn set_music_rate(&mut self, rate: MusicRate) {
        self.music_rate = rate;
        self.music_player.set_rate(rate);
    }

    pub fn music_rate(&self) -> MusicRate {
        self.music_rate
    }

    /// 设置音乐音量（线性增益）。非有限值按静音处理。
    pub fn set_music_gain(&mut self, gain: f64) {
        self.music_gain = sanitize_gain(gain);
    }

    pub fn music_gain(&self) -> f64 {
        self.music_gain
    }

    pub fn timeline(&self) -> &HitsoundTimeline {
        &self.timeline
    }

    pub fn library(&self) -> &SampleLibrary {
        &self.library
    }

    /// 可变访问样本库；批量放入样本后应一次性调用 [`HitsoundMixer::rebuild_timeline`]，
    /// 避免逐样本重建整条时间轴。
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

    /// 用当前样本库重新生成事件时间轴；会清空正在播放的声音，应在加载样本阶段调用。
    pub fn rebuild_timeline(&mut self, beatmap: &crate::domain::models::Beatmap) {
        let timeline = super::build_timeline(beatmap, &self.library);
        self.rebuild_timeline_events(timeline);
    }

    pub fn position_ms(&self) -> f64 {
        self.position_ms
    }

    /// 跳转到指定位置：丢弃正在播放的声音并重新定位事件游标，已越过的事件不再补播。
    pub fn seek(&mut self, position_ms: f64) {
        self.voices.clear();
        self.set_position(position_ms);
    }

    /// 只移动播放位置与事件游标，已开始的声音继续播放（真正清空声音的 seek 用 [`Self::seek`]）。
    ///
    /// 位置允许为负：离线导出可能从首个物件前的预卷开始，此时第 0 帧必须对应那个负
    /// 时刻，否则打击音相对音乐与画面提前；只有非有限值才回退到 0。
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

    /// 单调推进事件游标（不改变播放位置）：宿主重放同一窗口时已排入的事件不会被重复触发。
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

    /// 立即触发一个按名字查找的样本（只播一次），供游玩/回放的输入驱动发声使用。
    /// 声音从当前位置开始；返回 `false` 表示样本库没有这个名字（按静音处理）。
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
            // 显式触发（游玩/回放）由输入决定，不做音高调制。
            frequency: PlayFrequency::UNITY,
        });
        true
    }

    /// 开始一个循环音（滑条滑行、转盘旋转等）：样本自带循环长度时用它，否则整段循环。
    /// 返回的句柄交给 [`HitsoundMixer::stop_loop`]；名字不存在或样本为空时返回 `None`。
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
            frequency: PlayFrequency::UNITY,
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
        // 绕回时跳过 0，避免与「没有句柄」混淆。
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

        // 收集本窗口内新开始的事件：窗口按 `[start, end)` 取，正好落在 `window_end` 的
        // 留给下一个窗口（相邻窗口首尾相接不会丢）；收进来则会因游标对齐被放两次、音量翻倍。
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
                frequency: event.frequency,
            });
        }

        let master = self.master_gain;
        let music_gain = self.music_gain as f32;

        // 背景音乐先整窗口渲染到 scratch：时间伸缩（WSOLA）有状态，必须按输出帧顺序
        // 推进；音乐帧 = 谱面毫秒 × 音乐采样率 / 1000，负时间与曲末之后自然静音。
        self.music_scratch.clear();
        self.music_scratch.resize(frames * 2, 0.0);
        if let Some(music) = &self.music {
            self.music_player.render_at(
                window_start,
                music,
                music.sample_rate,
                frames,
                &mut self.music_scratch,
            );
        }

        for (index, pair) in output.chunks_exact_mut(2).enumerate() {
            let frame_time = window_start + index as f64 * ms_per_frame;
            let mut left = self.music_scratch[index * 2] * music_gain;
            let mut right = self.music_scratch[index * 2 + 1] * music_gain;

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
                let sample_rate = source.sample_rate.max(1) as f64;
                let loop_len = voice.loop_len.min(frames_in_source);
                let playable = if loop_len > 0 {
                    loop_len
                } else {
                    frames_in_source
                };

                // 按时间反推样本位置（与画面同一条时间轴，避免累计漂移）；音高倍率变化时
                // 取倍率对时间的积分，否则瞬时速度会随进度越跑越快。
                let elapsed = frame_time - voice.start_ms;
                if elapsed < 0.0 {
                    continue;
                }
                let mut position = voice.frequency.integral(elapsed) * sample_rate / 1000.0;
                if loop_len > 0 {
                    position = position.rem_euclid(playable as f64);
                } else if position >= playable as f64 {
                    continue;
                }

                let (sample_left, sample_right) = source.frame_at(position);
                let gain = (voice.gain * master) as f32;
                left += sample_left * gain;
                right += sample_right * gain;
            }

            pair[0] = soft_limit(left);
            pair[1] = soft_limit(right);
        }

        // 回收已播完的声音：本窗口才开始的先留着，其余按 min(end_ms, data_end_ms) 判断——
        // 时长 0 的事件 end_ms 是无限，只看它会让声音列表无限增长，长谱面混音掉到实时以下。
        self.voices.retain(|voice| {
            voice.start_ms >= window_start || voice.end_ms.min(voice.data_end_ms) > window_end
        });

        self.position_ms = window_end;
    }
}

/// 增益消毒：非有限值按静音处理，其余夹到 [0, 8]，坏输入不能让混音输出变 NaN 或爆音。
fn sanitize_gain(gain: f64) -> f64 {
    if gain.is_finite() {
        gain.clamp(0.0, 8.0)
    } else {
        0.0
    }
}

/// 软限幅：小信号近似线性，大信号平滑压缩到 ±1；用有理函数近似 `tanh`，
/// 比逐样本调用 `tanh` 便宜且两端行为一致。
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
    use crate::domain::models::{HitObjects, SampleBank, StandardHitObject};
    use crate::hitsound::build_timeline;
    use crate::hitsound::test_support::{beatmap_with, library_with, object_sample};
    use crate::hitsound::{PlayEvent, SampleData};

    /// 构造「每 100ms 一个事件、样本 10ms」的混音器（测试辅助）：事件间隔与窗口
    /// 长度相同，正好压在窗口边界上。
    fn click_mixer(events: usize) -> HitsoundMixer {
        let mut library = SampleLibrary::new();
        // 1kHz 采样率（10 帧 = 10ms）；增益 0.1 仍在软限幅的线性区。
        library.insert("click", SampleData::stereo(vec![1.0; 20], 1000));
        let timeline = HitsoundTimeline {
            events: (0..events)
                .map(|index| PlayEvent {
                    start_ms: index as f64 * 100.0,
                    duration_ms: 0.0,
                    source_id: 0,
                    gain: 0.1,
                    looping: false,
                    frequency: PlayFrequency::UNITY,
                })
                .collect(),
        };
        HitsoundMixer::new(library, timeline, 1000)
    }

    /// 播完的声音会被回收。
    #[test]
    fn finished_voices_are_recycled() {
        // 回归：时长 0 的事件 end_ms 无限，声音列表仍必须能随播放回收。
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

    /// 落在窗口边界的事件只播一次。
    #[test]
    fn events_on_window_boundary_play_once() {
        // 回归：边界事件被两个窗口各收一次时音量会凭空翻倍。
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

    /// 显式触发的样本从当前位置开始出声。
    #[test]
    fn explicit_trigger_starts_at_current_position() {
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

    /// 循环音可以启动与停止。
    #[test]
    fn looping_voice_can_start_and_stop() {
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

    /// 触发增益非有限值时按静音处理。
    #[test]
    fn non_finite_trigger_gain_is_silent() {
        let mut mixer = click_mixer(0);
        assert!(mixer.trigger("click", f64::NAN));
        assert!(mixer.render(50).iter().all(|value| *value == 0.0));
    }

    /// 负位置把起点之前的预卷算进输出。
    #[test]
    fn negative_position_includes_pre_roll_in_output() {
        // 负位置是预卷：缓冲区第 0 帧对应负时刻，事件相应推后，否则整段打击音会提前。
        let mut mixer = click_mixer(1);
        mixer.seek(-500.0);
        let output = mixer.render(1000);
        assert!(
            output[..500 * 2].iter().all(|value| *value == 0.0),
            "预卷期间不应有声音"
        );
        assert!(output[500 * 2].abs() > 0.0, "事件应当出现在缓冲区第 500 帧");
    }

    /// 样本采样率与混音采样率不同时按小数位置插值。
    #[test]
    fn sample_rate_mismatch_interpolates_fractionally() {
        // 采样率不同（内嵌 44.1kHz、导出 48kHz）时按小数位置插值；2:1 采样率差把
        // 采样位置依次钉死在 0、0.5、1.0、1.5、2.0。
        let mut library = SampleLibrary::new();
        library.insert("ramp", SampleData::mono(vec![0.0, 0.2, 0.0], 1000));
        let timeline = HitsoundTimeline {
            events: vec![PlayEvent {
                start_ms: 0.0,
                duration_ms: 0.0,
                source_id: 0,
                gain: 1.0,
                looping: false,
                frequency: PlayFrequency::UNITY,
            }],
        };
        let mut mixer = HitsoundMixer::new(library, timeline, 2000);
        let output = mixer.render(5);
        let left: Vec<f32> = output.chunks_exact(2).map(|pair| pair[0]).collect();
        // 样本值 [0.0, 0.2, 0.0]，位置 0/0.5/1.0/1.5/2.0 → 0.0/0.1/0.2/0.1/0.0。
        for (index, expected) in [0.0, 0.1, 0.2, 0.1, 0.0].into_iter().enumerate() {
            assert!(
                (left[index] - soft_limit(expected)).abs() < 1e-5,
                "第 {index} 帧 = {}，预期 {}",
                left[index],
                soft_limit(expected)
            );
        }
    }

    /// 升调斜坡按倍率积分推进样本位置。
    #[test]
    fn pitch_ramp_advances_sample_position_by_multiplier() {
        // 回归：位置取倍率对时间的积分（按「瞬时倍率 × 时间」会越跑越快）。倍率每毫秒
        // +0.001、上限 2.0，第 500 帧的积分是 625，脉冲应恰好在输出第 500 帧被采到。
        let mut library = SampleLibrary::new();
        let mut frames = vec![0.0_f32; 2000];
        frames[625] = 1.0;
        library.insert("impulse", SampleData::mono(frames, 1000));
        let timeline = HitsoundTimeline {
            events: vec![PlayEvent {
                start_ms: 0.0,
                duration_ms: 0.0,
                source_id: 0,
                gain: 0.1,
                looping: false,
                frequency: PlayFrequency::ramp(1.0, 0.001, 2.0),
            }],
        };
        let mut mixer = HitsoundMixer::new(library, timeline, 1000);
        let left: Vec<f32> = mixer
            .render(1000)
            .chunks_exact(2)
            .map(|pair| pair[0])
            .collect();
        for (index, value) in left.iter().enumerate() {
            if index == 500 {
                assert!(
                    (value - soft_limit(0.1)).abs() < 1e-6,
                    "第 500 帧应当采到脉冲：{value}"
                );
            } else {
                assert_eq!(*value, 0.0, "第 {index} 帧不应当有声音");
            }
        }
    }

    /// 落在窗口边界的打击音不会被跳过。
    #[test]
    fn hitsound_on_window_boundary_is_not_skipped() {
        // 回归：正好落在窗口末尾的事件留给下一个窗口，不能被永久跳过。
        let mut library = SampleLibrary::new();
        library.insert(
            "normal-hitnormal",
            SampleData::stereo(vec![0.5, 0.5, 0.5, 0.5, 0.5, 0.5], 1000),
        );
        let mut beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                start_time: 1000,
                end_time: 1000,
                hit_type: 1,
                hitsound: 0,
                ..Default::default()
            }]),
        );
        beatmap.timing_points[0].sample_set = 1;
        let timeline = build_timeline(&beatmap, &library);
        assert_eq!(timeline.len(), 1);

        let mut mixer = HitsoundMixer::new(library, timeline, 1000);
        // 第一个窗口 [0, 1000ms) 内事件还没开始，第二个窗口必须能听到它。
        let first = mixer.render(1000);
        assert!(first.iter().all(|value| *value == 0.0));
        let second = mixer.render(1000);
        assert!(
            second.iter().any(|value| *value > 0.0),
            "窗口边界处的事件被跳过了"
        );
    }

    /// 混音器按时间与主音量输出。
    #[test]
    fn mixer_respects_time_and_master_volume() {
        let library = library_with(&["normal-hitnormal"]);
        let beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                start_time: 0,
                end_time: 0,
                hit_type: 1,
                hitsound: 0,
                samples: object_sample(SampleBank::Normal, 100),
                ..Default::default()
            }]),
        );
        let timeline = build_timeline(&beatmap, &library);

        // 样本 1kHz 采样率（1 帧 = 1ms），4 帧 = 4ms。
        let mut mixer = HitsoundMixer::new(library, timeline, 1000);
        let quiet = mixer.render(2);
        assert_eq!(quiet.len(), 4);
        // 事件增益 1.0 × 主音量 0.5，样本值为 1.0。
        mixer.set_master_gain(0.5);
        mixer.seek(0.0);
        let mixed = mixer.render(2);
        let expected = soft_limit(0.5);
        assert!((mixed[0] - expected).abs() < 1e-6, "left={}", mixed[0]);
        assert!((mixed[1] - expected).abs() < 1e-6, "right={}", mixed[1]);
    }

    /// 混音器 seek 后不补播已越过的事件。
    #[test]
    fn mixer_does_not_replay_events_after_seek() {
        let library = library_with(&["normal-hitnormal"]);
        let beatmap = beatmap_with(
            0,
            HitObjects::Standard(vec![StandardHitObject {
                start_time: 0,
                end_time: 0,
                hit_type: 1,
                hitsound: 0,
                samples: object_sample(SampleBank::Normal, 100),
                ..Default::default()
            }]),
        );
        let timeline = build_timeline(&beatmap, &library);
        let mut mixer = HitsoundMixer::new(library, timeline, 1000);
        mixer.seek(500.0);
        let output = mixer.render(2);
        assert!(output.iter().all(|value| *value == 0.0));
    }

    /// 混音器主音量忽略非有限值。
    #[test]
    fn mixer_ignores_non_finite_master_volume() {
        let library = SampleLibrary::new();
        let mut mixer = HitsoundMixer::new(library, HitsoundTimeline::default(), 1000);
        mixer.set_master_gain(f64::NAN);
        assert_eq!(mixer.master_gain(), 0.0);
        mixer.set_master_gain(-1.0);
        assert_eq!(mixer.master_gain(), 0.0);
    }

    /// 4 帧的递增音乐（每帧值 = 帧号），1kHz 输出下 1 帧 = 1ms。
    fn ramp_music() -> SampleData {
        SampleData::stereo(vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0], 1000)
    }

    /// 音乐按「谱面毫秒 × 采样率 / 1000」逐帧采样，音量走独立的音乐增益。
    #[test]
    fn music_mixes_at_chart_time_with_its_own_gain() {
        let mut mixer = HitsoundMixer::new(SampleLibrary::new(), HitsoundTimeline::default(), 1000);
        mixer.set_music(Some(ramp_music()));
        mixer.set_music_gain(0.5);

        let output = mixer.render(4);
        for (index, pair) in output.chunks_exact(2).enumerate() {
            let expected = soft_limit(index as f32 * 0.5);
            assert!(
                (pair[0] - expected).abs() < 1e-6,
                "frame {index}: {}",
                pair[0]
            );
            assert!(
                (pair[1] - expected).abs() < 1e-6,
                "frame {index}: {}",
                pair[1]
            );
        }
    }

    /// 音乐在负时间（预卷）与曲末之后都是静音；seek 后从对应帧继续。
    #[test]
    fn music_is_silent_outside_and_follows_seek() {
        let mut mixer = HitsoundMixer::new(SampleLibrary::new(), HitsoundTimeline::default(), 1000);
        mixer.set_music(Some(ramp_music()));

        // 预卷段：谱面时间 -2ms ~ 0ms，音乐还没开始。
        mixer.seek(-2.0);
        let pre_roll = mixer.render(2);
        assert!(pre_roll.iter().all(|value| *value == 0.0), "{pre_roll:?}");

        // seek 到 2ms：从音乐第 2 帧继续（输出经软限幅）。
        mixer.seek(2.0);
        let resumed = mixer.render(2);
        assert!(
            (resumed[0] - soft_limit(2.0)).abs() < 1e-6,
            "{}",
            resumed[0]
        );
        assert!(
            (resumed[2] - soft_limit(3.0)).abs() < 1e-6,
            "{}",
            resumed[2]
        );

        // 曲末之后继续静音，不越界、不循环。
        mixer.seek(5.0);
        let after_end = mixer.render(2);
        assert!(after_end.iter().all(|value| *value == 0.0), "{after_end:?}");
    }

    /// 音乐与打击音叠加后再统一限幅。
    #[test]
    fn music_and_hitsound_sum_before_limiting() {
        let mut library = SampleLibrary::new();
        library.insert("click", SampleData::stereo(vec![1.0; 4], 1000));
        let timeline = HitsoundTimeline {
            events: vec![PlayEvent {
                start_ms: 0.0,
                duration_ms: 0.0,
                source_id: 0,
                gain: 0.25,
                looping: false,
                frequency: PlayFrequency::UNITY,
            }],
        };
        let mut mixer = HitsoundMixer::new(library, timeline, 1000);
        mixer.set_music(Some(SampleData::stereo(vec![1.0; 4], 1000)));
        mixer.set_music_gain(0.5);

        let output = mixer.render(1);
        let expected = soft_limit(0.25 + 0.5);
        assert!((output[0] - expected).abs() < 1e-6, "{}", output[0]);
        assert!((output[1] - expected).abs() < 1e-6, "{}", output[1]);
    }

    /// 音乐增益与主音量一样拒绝非有限值。
    #[test]
    fn music_gain_rejects_non_finite_values() {
        let mut mixer = HitsoundMixer::new(SampleLibrary::new(), HitsoundTimeline::default(), 1000);
        mixer.set_music_gain(f64::NAN);
        assert_eq!(mixer.music_gain(), 0.0);
        mixer.set_music_gain(f64::INFINITY);
        assert_eq!(mixer.music_gain(), 0.0);
    }

    /// 设置时间伸缩倍率后，音乐内容按倍率推进（保调），而不是被重采样。
    #[test]
    fn music_rate_stretches_the_content() {
        // 4 秒线性斜坡（每帧值 = 帧号 / 10000，足够小使软限幅近似线性），便于断言内容位置。
        let samples: Vec<f32> = (0..4_000)
            .flat_map(|index| {
                let value = index as f32 / 10_000.0;
                [value, value]
            })
            .collect();
        let mut mixer = HitsoundMixer::new(SampleLibrary::new(), HitsoundTimeline::default(), 1000);
        mixer.set_music(Some(SampleData::stereo(samples, 1000)));
        mixer.set_music_rate(MusicRate {
            resample: 1.0,
            stretch: 2.0,
        });
        assert_eq!(
            mixer.music_rate(),
            MusicRate {
                resample: 1.0,
                stretch: 2.0
            }
        );

        let output = mixer.render(3_000);
        // 输出第 N 帧应该读到内容第 2N 帧（相似度搜索只允许几十帧的局部偏移）。
        for frame in [10_usize, 500, 1_000, 1_500] {
            let expected = soft_limit(frame as f32 * 2.0 / 10_000.0);
            assert!(
                (output[frame * 2] - expected).abs() < 0.01,
                "输出帧 {frame}：{} vs {expected}",
                output[frame * 2]
            );
        }
        // 4 秒内容在 2 倍速下 2 秒后结束，之后是静音。
        let tail = output[2_200 * 2..]
            .iter()
            .fold(0.0_f32, |acc, value| acc.max(value.abs()));
        assert!(tail < 1e-3, "内容结束后仍有 {tail} 的残留");

        // 恒等倍率下同样的源要 4 秒才结束，证明上面确实在做时间伸缩。
        let mut identity =
            HitsoundMixer::new(SampleLibrary::new(), HitsoundTimeline::default(), 1000);
        identity.set_music(Some(SampleData::stereo(
            (0..4_000)
                .flat_map(|index| {
                    let value = index as f32 / 10_000.0;
                    [value, value]
                })
                .collect(),
            1000,
        )));
        let plain = identity.render(3_000);
        assert!(
            plain[2_500 * 2] > 0.2,
            "恒等倍率下 2500ms 处应当仍有内容：{}",
            plain[2_500 * 2]
        );
    }

    /// 实时链路（图表域输出 + 宿主按总倍率重采样）下 DT 保调：1kHz 正弦经 1.5 倍速后仍是 1kHz。
    #[test]
    fn realtime_chart_domain_keeps_dt_pitch() {
        let rate = 48_000_u32;
        // 2 秒的 1kHz 正弦（源采样率 = 输出采样率，便于断言）。
        let source = SampleData::stereo(
            (0..rate as usize * 2)
                .flat_map(|index| {
                    let value = (2.0 * std::f64::consts::PI * 1000.0 * index as f64 / rate as f64)
                        .sin() as f32;
                    [value, value]
                })
                .collect(),
            rate,
        );
        let mut mixer = HitsoundMixer::new(SampleLibrary::new(), HitsoundTimeline::default(), rate);
        mixer.set_music(Some(source));
        // DT：总倍率 1.5 由宿主重采样，图表域要求「内容 1:1、音高 1/1.5」。
        mixer.set_music_rate(MusicRate::chart_domain(1.5, 1.0));

        let ring = mixer.render(rate as usize * 2);
        // 交给宿主按总倍率线性重采样（与 `hitsound-worklet.js` 的消费方式一致）。
        let ring_frames = ring.len() / 2;
        let target = (ring_frames as f64 / 1.5) as usize;
        let mut played = Vec::with_capacity(target);
        let mut cursor = 0.0_f64;
        while played.len() < target {
            let base = (cursor.floor() as usize).min(ring_frames - 2);
            let next = (base + 1).min(ring_frames - 1);
            let fraction = (cursor - base as f64) as f32;
            let left = ring[base * 2] + (ring[next * 2] - ring[base * 2]) * fraction;
            played.push(left);
            cursor += 1.5;
        }

        // 取中段（跳过伸缩起步的十几毫秒）做单频点幅度比较。
        let window = &played[played.len() / 2 - 12_000..played.len() / 2 + 12_000];
        let at_1000 = tone_magnitude(window, 1000.0, rate as f64);
        let at_1500 = tone_magnitude(window, 1500.0, rate as f64);
        assert!(
            at_1000 > at_1500 * 8.0,
            "DT 实时链路音高不保：1kHz={at_1000}, 1.5kHz={at_1500}"
        );
    }

    /// 单个频点上的幅度（cos/sin 相关求和）。
    fn tone_magnitude(samples: &[f32], frequency: f64, sample_rate: f64) -> f64 {
        let omega = 2.0 * std::f64::consts::PI * frequency / sample_rate;
        let (mut real, mut imaginary) = (0.0_f64, 0.0_f64);
        for (index, value) in samples.iter().enumerate() {
            let phase = omega * index as f64;
            real += *value as f64 * phase.cos();
            imaginary += *value as f64 * phase.sin();
        }
        (real * real + imaginary * imaginary).sqrt() / samples.len().max(1) as f64
    }
}
