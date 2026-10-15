//! 回放跳转恢复当前循环的原始起点与相位；普通预览的 seek 行为不变。
use super::*;

impl HitsoundMixer {
    pub fn seek_replay(&mut self, position_ms: f64) {
        self.seek(position_ms);
        for index in 0..self.next_event {
            let event = self.timeline.events[index];
            if !event.looping
                || event.duration_ms <= 0.0
                || event.start_ms + event.duration_ms <= self.position_ms
            {
                continue;
            }
            let Some(source) = self.library.sources.get(event.source_id) else {
                continue;
            };
            if source.frames() == 0 {
                continue;
            }
            let loop_len = source.loop_len;
            let id = self.take_voice_id();
            // 保留 start_ms 与 frequency.phase；把起点改为 seek 时刻会错误地重启音色。
            self.voices.push(Voice {
                id,
                source_id: event.source_id,
                gain: event.gain,
                start_ms: event.start_ms,
                end_ms: event.start_ms + event.duration_ms,
                data_end_ms: f64::INFINITY,
                loop_len,
                frequency: event.frequency,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hitsound::PlayEvent;

    #[test]
    fn seek_preserves_loop_phase_omits_old_hits_and_stops_at_end() {
        let mut library = SampleLibrary::new();
        library.insert(
            "loop",
            SampleData::mono(vec![0.1, 0.2, 0.3, 0.4], 1000).with_loop(4),
        );
        library.insert("hit", SampleData::mono(vec![0.8; 20], 1000));
        let timeline = HitsoundTimeline {
            events: vec![
                PlayEvent {
                    source_id: 0,
                    start_ms: 0.0,
                    duration_ms: 10.0,
                    gain: 1.0,
                    looping: true,
                    frequency: PlayFrequency {
                        phase: 1.0,
                        ..PlayFrequency::UNITY
                    },
                },
                PlayEvent {
                    source_id: 1,
                    start_ms: 0.0,
                    duration_ms: 0.0,
                    gain: 1.0,
                    looping: false,
                    frequency: PlayFrequency::UNITY,
                },
            ],
        };
        let mut mixer = HitsoundMixer::new(library, timeline, 1000);
        mixer.seek_replay(5.0);
        let first = mixer.render(5);
        assert!((first[0] - super::super::soft_limit(0.3)).abs() < 1e-6);
        mixer.seek_replay(5.0);
        assert_eq!(first, mixer.render(5));
        mixer.seek_replay(10.0);
        assert!(mixer.render(4).iter().all(|&value| value == 0.0));
        // 原 seek 仍不恢复任何旧循环。
        mixer.seek(5.0);
        assert!(mixer.render(4).iter().all(|&value| value == 0.0));
    }
}
