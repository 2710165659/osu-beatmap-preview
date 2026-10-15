//! 回放会话接线；判定仅在载入时编译，时钟、资源和音频流沿用实时会话。
use super::*;
use crate::render::cpu::modes::standard::osr::font::ArgonFont;
use crate::{
    gameplay::timeline::GameplayTimeline, preparation::standard::PreparedStandard,
    replay::ReplayDocument,
};

pub(super) struct ReplaySessionData {
    pub prepared: Arc<PreparedStandard>,
    pub gameplay: Arc<GameplayTimeline>,
    font: Option<Arc<ArgonFont>>,
    config: Arc<config::CoreConfig>,
}
impl std::fmt::Debug for ReplaySessionData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplaySessionData").finish_non_exhaustive()
    }
}
impl RealtimeSession {
    pub fn from_replay_bundle(
        bundle: ResourceBundle,
        options: RealtimeOptions,
        document: Arc<ReplayDocument>,
        font: Option<ArgonFont>,
    ) -> Result<Self> {
        if options.convert.is_some() || !options.mods.is_empty() {
            return Err(PreviewError::new("OSR 回放禁止转谱和用户 Mod"));
        }
        crate::replay::compatibility::select(&document)?;
        let beatmap = bundle
            .beatmap
            .as_ref()
            .ok_or_else(|| PreviewError::new("回放缺少谱面"))?;
        let prepared = Arc::new(crate::preparation::standard::prepare(beatmap)?);
        let gameplay = Arc::new(crate::gameplay::engine::compile_standard(
            &prepared, document,
        )?);
        let mut session = Self::from_bundle(bundle, options)?;
        let first = prepared.objects[0].source.start_time;
        let last = prepared.last_end.max(gameplay.settlement_time).ceil() as i64;
        session.timeline = preview_timeline(first, last, session.beatmap.audio_lead_in_ms(), 1.0);
        session.replay = Some(Arc::new(ReplaySessionData {
            prepared,
            gameplay,
            font: font.map(Arc::new),
            config: Arc::clone(&session.options.core_config),
        }));
        session.prepare_replay_source()?;
        session.seek(0.0, session.timeline.absolute_start_ms as f64);
        Ok(session)
    }
    pub fn replay_timeline(&self) -> Option<&GameplayTimeline> {
        self.replay.as_ref().map(|r| r.gameplay.as_ref())
    }
    pub(super) fn prepare_replay_source(&mut self) -> Result<()> {
        let replay = self.replay.as_ref().expect("仅回放会话构建回放源");
        let mut cfg = (*replay.config).clone();
        let native_height = config::with_config(Arc::clone(&replay.config), || {
            crate::render::cpu::modes::standard::osr::replay_layout(
                crate::render::geometry::OutputFormat::Mp4,
            )
            .frame_height
        });
        // 分辨率切换重新计算当前格式 SCALE，物件与 HUD 在目标像素尺寸直接绘制。
        cfg.render.standard.mp4.SCALE *= self.options.render.height as f64 / native_height as f64;
        let config = Arc::new(cfg);
        self.source = config::with_config(Arc::clone(&config), || {
            crate::render::wgpu::modes::standard::replay::prepare(
                &self.beatmap,
                Arc::clone(&replay.prepared),
                Arc::clone(&replay.gameplay),
                replay.font.as_ref().map(|f| (**f).clone()),
                self.options.render.dimensions(),
            )
        })?;
        self.options.core_config = config;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        gameplay::state::ReplayStatus,
        replay::{ReplayFrame, ReplayHeader, ReplayMetadata},
    };

    #[test]
    fn realtime_replay_seek_audio_and_resize_do_not_change_results() {
        let mut session = session(false);
        let expected = session.scene_at_absolute(1500).unwrap();
        let state = session.replay_timeline().unwrap().state_at(1500.0).clone();
        session.play(0.0);
        session.pull_audio(1200.0, 4096);
        session.seek(1200.0, 900.0);
        session.set_render_size(414, 232).unwrap();
        assert_eq!(session.scene_at_absolute(1500).unwrap().size.width, 414);
        session.set_render_size(826, 464).unwrap();
        let actual = session.scene_at_absolute(1500).unwrap();
        assert_eq!(
            format!("{:?}", expected.commands),
            format!("{:?}", actual.commands)
        );
        assert_eq!(session.replay_timeline().unwrap().state_at(1500.0), &state);
        assert_eq!(state.status, ReplayStatus::Completed);
        assert_eq!(state.score, 1_000_000);
        assert!(session.set_mods(vec!["HR".into()], 1300.0).is_err());
    }

    #[test]
    fn failed_replay_freezes_result_while_scene_and_music_continue() {
        let mut session = session(true);
        let gameplay = session.replay_timeline().unwrap();
        assert_eq!(gameplay.state_at(1010.0).status, ReplayStatus::Failed);
        assert_eq!(gameplay.state_at(1010.0), gameplay.state_at(2500.0));
        assert_eq!(gameplay.input_at(2500.0).unwrap().keys, 0);
        assert!(session.timeline.last_object_ms >= 2000);
        session.set_music(Some(SampleData::mono(vec![0.5; 5000], 1000)));
        session.seek(0.0, 1700.0);
        session.play(0.0);
        assert!(session.pull_audio(0.0, 100).iter().any(|v| *v != 0.0));
        let scene = session.scene_at_absolute(1900).unwrap();
        assert!(scene.commands.iter().any(|c| matches!(
            c,
            crate::render::scene::DrawCommand::Circle { .. }
                | crate::render::scene::DrawCommand::Ring { .. }
        )));
    }

    #[test]
    fn replay_feedback_keeps_gpu_resources_across_frames_and_seek() {
        let session = session(false);
        let reference = session.scene_at_absolute(1050).unwrap();
        let resources = reference
            .commands
            .iter()
            .filter_map(|command| {
                if let crate::render::scene::DrawCommand::TransformedSprite {
                    resource,
                    additive: true,
                    ..
                } = command
                {
                    Some(Arc::clone(&reference.resources[resource]))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        // GREAT 的文字与两组各四个环都必须复用已准备图像。
        assert_eq!(resources.len(), 9);
        for time in [1100, 1200, 1050] {
            let frame = session.scene_at_absolute(time).unwrap();
            let actual = frame
                .commands
                .iter()
                .filter_map(|command| {
                    if let crate::render::scene::DrawCommand::TransformedSprite {
                        resource,
                        additive: true,
                        ..
                    } = command
                    {
                        Some(&frame.resources[resource])
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(actual.len(), resources.len());
            for (original, current) in resources.iter().zip(actual) {
                assert!(Arc::ptr_eq(original, current));
            }
        }
    }

    fn session(fail: bool) -> RealtimeSession {
        let raw = format!("osu file format v14\n[General]\nMode:0\n[Difficulty]\nCircleSize:4\nApproachRate:5\nOverallDifficulty:5\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n100,100,1000,1,0\n{}", if fail { "400,100,2000,1,0" } else { "" });
        let beatmap = crate::parse_beatmap_bytes(raw.as_bytes()).unwrap();
        let replay = ReplayDocument {
            header: ReplayHeader {
                ruleset_id: 0,
                version: 30_000_016,
                beatmap_md5: "0".repeat(32),
                player: String::new(),
                replay_hash: String::new(),
                counts: [1, 0, 0, 0, 0, 0],
                total_score: 1_000_000,
                max_combo: 1,
                perfect: true,
                mods_bits: 0,
                life_graph: String::new(),
                timestamp_ticks: 0,
                online_id: 0,
                target_practice_accuracy: None,
            },
            metadata: Some(ReplayMetadata {
                client_version: "2026.119.0-lazer-windows".into(),
                rank: "X".into(),
                mods: vec![],
                statistics: std::collections::BTreeMap::from([("great".into(), 1)]),
                maximum_statistics: std::collections::BTreeMap::from([("great".into(), 1)]),
                pauses: vec![],
                extra: Default::default(),
            }),
            frames: vec![
                ReplayFrame {
                    sequence: 0,
                    time_ms: 0,
                    delta_ms: 0,
                    cursor: [100.0; 2],
                    keys: 0,
                },
                ReplayFrame {
                    sequence: 1,
                    time_ms: 1010,
                    delta_ms: 1010,
                    cursor: [100.0; 2],
                    keys: 1,
                },
            ],
            seed: None,
            diagnostics: vec![],
        };
        let options = RealtimeOptions {
            render: crate::api::input::RenderConfig {
                width: 826,
                height: 464,
                scale: 1.0,
            },
            audio: crate::api::input::AudioConfig {
                sample_rate: 1000,
                ..Default::default()
            },
            ..Default::default()
        };
        RealtimeSession::from_replay_bundle(
            ResourceBundle::new(beatmap),
            options,
            Arc::new(replay),
            None,
        )
        .unwrap()
    }
}
