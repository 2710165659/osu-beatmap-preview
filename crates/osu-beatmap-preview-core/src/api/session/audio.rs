//! 实时会话音频资源和回放事件桥；音乐、流时钟与画面均使用谱面绝对时间。
use super::*;

impl RealtimeSession {
    // ── 音频：音乐与打击音同流混音 ──────────────────────────────────────
    //
    // 时间、倍速与 seek 由会话时钟统一驱动；宿主只需把 `pull_audio` 的输出送进
    // 音频设备，并把音频线程的消费位置用 `on_audio_clock` 转发回来。

    /// 设置背景音乐（`None` 表示清除）；与打击音共用同一时间轴，倍速、seek、暂停
    /// 只有一处位置计算。没有音乐的谱面（单独的 `.osu`）不必调用它。
    pub fn set_music(&mut self, music: Option<SampleData>) {
        self.mixer.set_music(music);
    }

    /// 更新音乐音量百分比（0..=100）。
    pub fn set_music_volume(&mut self, volume: i32) {
        self.mixer.set_music_gain(hitsound::volume_gain(volume));
    }

    /// 打开/关闭打击音；关闭时事件时间轴为空，音乐照常输出。
    pub fn set_hitsound_enabled(&mut self, enabled: bool) {
        if self.hitsound_enabled == enabled {
            return;
        }
        self.hitsound_enabled = enabled;
        self.rebuild_hitsound_timeline();
    }

    pub fn hitsound_enabled(&self) -> bool {
        self.hitsound_enabled
    }

    /// 更新打击音音量百分比（0..=100）。
    pub fn set_hitsound_volume(&mut self, volume: i32) {
        self.mixer.set_master_gain(hitsound::volume_gain(volume));
    }

    /// 当前谱面需要样本库提供的样本名（按优先级排列，含裸名回退）；缺失的样本在混音
    /// 时按静音处理，宿主只装载拿得到的名字。NC 生效时额外带上 4 个节拍鼓点样本名。
    pub fn hitsound_required_names(&self) -> Vec<String> {
        #[cfg(feature = "replay")]
        let mut names = self.replay.as_ref().map_or_else(
            || hitsound::referenced_names(&self.beatmap),
            |r| hitsound::replay::referenced_names(&self.beatmap, &r.prepared, &r.gameplay),
        );
        #[cfg(not(feature = "replay"))]
        let mut names = hitsound::referenced_names(&self.beatmap);
        if let Some((storyboard, _)) = &self.storyboard {
            names.extend(storyboard.samples.iter().map(|sample| sample.path.clone()));
        }
        if self.settings.nightcore {
            names.extend(hitsound::NIGHTCORE_SAMPLE_NAMES.map(str::to_string));
        }
        let mut seen = std::collections::HashSet::new();
        names.retain(|name| seen.insert(name.clone()));
        names
    }

    /// 故事板音效始终取自谱面包，不受皮肤打击音来源开关影响。
    pub fn storyboard_sample_names(&self) -> Vec<String> {
        self.storyboard
            .as_ref()
            .map_or_else(Vec::new, |(storyboard, _)| {
                storyboard
                    .samples
                    .iter()
                    .map(|sample| sample.path.clone())
                    .collect()
            })
    }

    /// 放入一段已解码的样本 PCM。只放进样本库、不重建时间轴：宿主应把样本全部放完后
    /// 调用一次 [`RealtimeSession::rebuild_hitsound_timeline`]，否则每个样本都遍历整张谱面。
    pub fn set_hitsound_sample(
        &mut self,
        name: &str,
        channels: hitsound::Channels,
        sample_rate: u32,
        loop_len: usize,
    ) {
        let data = SampleData {
            channels,
            sample_rate: sample_rate.max(1),
            loop_len,
        };
        self.mixer.library_mut().insert(name, data);
    }

    /// 用当前样本库重建打击音事件时间轴（样本全部放完后调用一次）。
    /// 打击音关闭时只留 NC 节拍鼓点（鼓点属于 Mod，不受该开关影响）；
    /// 重建会清空正在播放的声音，只在装载阶段调用。
    pub fn rebuild_hitsound_timeline(&mut self) {
        let mut timeline = if self.hitsound_enabled {
            #[cfg(feature = "replay")]
            let timeline = self.replay.as_ref().map_or_else(
                || hitsound::build_timeline(&self.beatmap, self.mixer.library()),
                |r| {
                    hitsound::replay::build_timeline(
                        &self.beatmap,
                        &r.prepared,
                        &r.gameplay,
                        self.mixer.library(),
                    )
                },
            );
            #[cfg(not(feature = "replay"))]
            let timeline = hitsound::build_timeline(&self.beatmap, self.mixer.library());
            timeline
        } else {
            HitsoundTimeline::default()
        };
        if self.settings.nightcore {
            let end_ms = (self.timeline.absolute_start_ms + self.timeline.duration_ms) as f64;
            timeline.merge(hitsound::nightcore_events(
                &self.beatmap,
                self.mixer.library(),
                end_ms,
            ));
        }
        if self.storyboard_enabled {
            if let Some((storyboard, _)) = &self.storyboard {
                let mut samples = storyboard.sample_timeline(self.mixer.library());
                let start = self.timeline.absolute_start_ms as f64;
                // 游戏允许故事板音效晚启动不足 100ms；开场前 98ms 的 intro.mp3
                // 应从样本头开始播放，远早于起点的事件则仍跳过，不能改变预览起点。
                for event in &mut samples.events {
                    if event.start_ms < start && start - event.start_ms < 100.0 {
                        event.start_ms = start;
                    }
                }
                timeline.merge(samples);
            }
        }
        self.mixer.rebuild_timeline_events(timeline);
        #[cfg(feature = "replay")]
        if self.replay.is_some() {
            self.mixer.seek_replay(self.mixer.position_ms());
        }
    }

    /// 清空已放入的样本并重建时间轴（保留音乐、音量与开关设置）。
    ///
    /// 切 Mod 或转谱后需要的样本集合会变，宿主重新装载即可，不必重建会话。
    pub fn reset_hitsound_samples(&mut self) {
        *self.mixer.library_mut() = SampleLibrary::new();
        self.rebuild_hitsound_timeline();
        self.mixer.stop_all();
    }

    pub(super) fn seek_mixer(&mut self, chart_ms: f64) {
        #[cfg(feature = "replay")]
        if self.replay.is_some() {
            self.mixer.seek_replay(chart_ms);
            return;
        }
        self.mixer.seek(chart_ms);
    }
}
