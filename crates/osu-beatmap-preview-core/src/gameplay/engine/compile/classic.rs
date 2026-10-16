//! CL 的节点类型与 note lock；与公共输入编译流程分开，便于核对官方策略。
use super::*;

#[cfg(test)]
mod tests {
    //! 可选本地验收：真实输入转为 CL 与 stable，和官方 Headless 重播核对。
    use super::super::*;

    #[test]
    #[ignore = "需要 OSR_SAMPLE_DIR 中的 synthetic-classic/stable.osr 和官方 classic-oracle-results.json"]
    fn classic_and_implicit_stable_match_the_official_judgements() {
        let root = std::path::PathBuf::from(std::env::var("OSR_SAMPLE_DIR").unwrap());
        let beatmap =
            crate::parse_beatmap_bytes(&std::fs::read(root.join("4903207.osu")).unwrap()).unwrap();
        let prepared = crate::replay::preparation::standard::prepare(&beatmap).unwrap();
        let oracle: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("classic-oracle-results.json")).unwrap(),
        )
        .unwrap();
        let mut previous = None;
        for file in ["synthetic-classic.osr", "synthetic-stable.osr"] {
            let document = Arc::new(
                crate::replay::parse_osr(&std::fs::read(root.join(file)).unwrap()).unwrap(),
            );
            let timeline = compile_standard(&prepared, document).unwrap();
            let state = timeline.state_at(f64::INFINITY);
            eprintln!("{file}: {}", serde_json::to_string(state).unwrap());
            assert_eq!(
                serde_json::to_value(&state.statistics).unwrap(),
                oracle["Statistics"]
            );
            assert_eq!(
                u64::from(state.max_combo),
                oracle["combo"].as_u64().unwrap()
            );
            assert!((state.accuracy.unwrap() - oracle["accuracy"].as_f64().unwrap()).abs() < 1e-12);
            // 本地官方二进制使用 V1 的 0.96；当前官方源码 V2 对开启 note lock 的 CL 用 0.985。
            let current_score =
                (oracle["score"].as_f64().unwrap() / 0.96 * 0.985).round_ties_even() as u64;
            assert!(state.score.abs_diff(current_score) <= 1);
            if let Some(previous) = previous {
                assert_eq!(*state, previous);
            }
            previous = Some(state.clone());
        }
    }

    #[test]
    #[ignore = "需要 OSR_TACHYON_SAMPLE_DIR 中的 mrekk.osr 与 5774158.osu"]
    fn real_tachyon_sample_compiles_to_a_complete_timeline() {
        let root = std::path::PathBuf::from(std::env::var("OSR_TACHYON_SAMPLE_DIR").unwrap());
        let beatmap =
            crate::parse_beatmap_bytes(&std::fs::read(root.join("5774158.osu")).unwrap()).unwrap();
        let document = Arc::new(
            crate::replay::parse_osr(&std::fs::read(root.join("mrekk.osr")).unwrap()).unwrap(),
        );
        assert_eq!(
            document.metadata.as_ref().unwrap().client_version,
            "2026.821.0-tachyon-windows"
        );
        let prepared = crate::replay::preparation::standard::prepare(&beatmap).unwrap();
        let timeline = compile_standard(&prepared, document).unwrap();
        let state = timeline.state_at(f64::INFINITY);
        assert_eq!(state.status, ReplayStatus::Completed);
        assert!(state.max_combo > 0);
        assert!(state.score > 0);
        eprintln!("tachyon: {}", serde_json::to_string(state).unwrap());
    }
}

impl Compiler<'_> {
    pub(super) fn node_result(&self, kind: NodeKind, hit: bool) -> (HitResult, HitResult) {
        if kind != NodeKind::Tail {
            (
                if hit {
                    HitResult::LargeTickHit
                } else {
                    HitResult::LargeTickMiss
                },
                HitResult::LargeTickHit,
            )
        } else if self.policy.classic_sliders {
            (
                if hit {
                    HitResult::SmallTickHit
                } else {
                    HitResult::SmallTickMiss
                },
                HitResult::SmallTickHit,
            )
        } else {
            (
                if hit {
                    HitResult::SliderTailHit
                } else {
                    HitResult::IgnoreMiss
                },
                HitResult::SliderTailHit,
            )
        }
    }

    pub(super) fn classic_hittable(&self, id: usize, time: f64, cursor: [f32; 2]) -> bool {
        if (time - self.prepared.objects[id].source.start_time as f64).abs() >= 400.0 {
            return false;
        }
        if id > 0
            && self.prepared.objects[id - 1].source.stack_height > 0
            && self.objects[id - 1].end.is_none()
        {
            return false;
        }
        // 对照 LegacyHitPolicy：允许重叠的长物件；前序未结算物件的终点与目标头部
        // 相隔超过 3ms 时阻止点击。只在 CL 路径执行，普通预览不增加开销。
        let first = self.first_unjudged.min(id);
        !self.prepared.objects[first..id]
            .iter()
            .zip(&self.objects[first..id])
            .any(|(object, state)| {
                // 已判定的滑条头在父滑条结束前仍可吞掉其下方点击，包括长于 400ms 的滑条。
                state.end.is_none()
                    && (object.end + 3.0 < self.prepared.objects[id].source.start_time as f64
                        || (object.path.is_some()
                            && state.head.is_some()
                            && circle::in_range(cursor, object.position, self.prepared.radius)))
            })
    }
}
