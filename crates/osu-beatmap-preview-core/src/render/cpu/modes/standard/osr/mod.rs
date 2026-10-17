//! 回放帧消费不可变结果；玩法层和最终背景上的 HUD 查询同一时刻。
mod blend;
mod feedback;
pub mod font;
mod hud;
mod layout;
mod overlay;
mod visibility;
use super::{context::*, driver, frame, spinner};
use crate::gameplay::timeline::GameplayTimeline;
use crate::model::BreakPeriod;
use crate::render::canvas::Img;
use crate::replay::preparation::standard::PreparedStandard;
use std::sync::Arc;

pub(crate) fn replay_layout(format: crate::render::geometry::OutputFormat) -> FrameLayout {
    layout::build(format)
}

/// 每次回放导出只构建一份，worker 共享光标精灵和原版字体。
pub struct ReplayVisuals {
    pub cursor: super::autoplay::AutoplayCursor,
    pub font: Option<font::ArgonFont>,
    /// 判定条颜色轴只与 OD 和当前格式 SCALE 有关，准备阶段生成一次。
    hit_error_axis: Arc<Img>,
    health: hud::HealthAnimation,
    feedback: feedback::FeedbackVisuals,
    flashlight: Option<visibility::ReplayFlashlight>,
}
impl ReplayVisuals {
    pub fn new(
        context: &RenderContext,
        gameplay: &GameplayTimeline,
        font: Option<font::ArgonFont>,
        breaks: &[BreakPeriod],
        od: f64,
    ) -> Self {
        let cursor = super::autoplay::AutoplayCursor::from_replay(
            context,
            &gameplay.input,
            gameplay.failure_time,
        );
        let mods = crate::replay::mods::decode(&gameplay.document).expect("已校验的回放 Mod");
        let flashlight = mods
            .settings
            .flashlight
            .then(|| visibility::ReplayFlashlight::new(gameplay, &cursor, &mods, breaks));
        let scale = context.frame_layout.output_scale();
        Self {
            flashlight,
            feedback: feedback::FeedbackVisuals::new(context, font.as_ref()),
            hit_error_axis: hud::build_hit_error_axis(scale, od),
            health: hud::HealthAnimation::new(gameplay),
            cursor,
            font,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    context: &RenderContext,
    cache: &mut RenderCache,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    time: i64,
    breaks: &[BreakPeriod],
    visible: &[usize],
    background: Option<&Img>,
) -> Img {
    cache.prepare_for_context(context);
    let mut image = driver::background_frame(context, background);
    frame::draw_follow_points(&mut image, context, cache, time);
    let failed = gameplay.failure_time.is_some_and(|end| time as f64 >= end);
    for &id in visible {
        let object = &context.hit_objects[id];
        let state = &gameplay.objects[id];
        if failed {
            if object.hit_type & 8 != 0 {
                frame::draw_spinner(&mut image, context, cache, object, time);
            } else if object.hit_type & 2 != 0 {
                frame::draw_slider(&mut image, context, cache, id, time);
            } else {
                frame::draw_hit_circle(&mut image, context, cache, id, time);
            }
        } else if object.hit_type & 8 != 0 {
            let (angle, total) = state.rotation_at(time as f64);
            let (_, previous) = state.rotation_at(time as f64 - 100.0);
            spinner::draw_cpu_state(
                &mut image,
                context,
                object,
                time,
                Some((
                    angle.to_radians() as f64,
                    total as f64 / 360.0,
                    (total - previous) as f64 / 360.0 * 600.0,
                )),
            );
        } else {
            let alpha = head_alpha(
                context,
                object.start_time,
                state.head,
                time,
                gameplay.policy.fade_hit_circle_early,
                prepared.od,
            );
            if object.hit_type & 2 != 0 {
                frame::draw_slider_state(
                    &mut image,
                    context,
                    cache,
                    id,
                    time,
                    Some(alpha),
                    state.tracking_at(time as f64),
                );
            } else if !context.settings.traceable {
                let position = prepared.objects[id].position;
                frame::draw_circle_piece(
                    &mut image,
                    context,
                    cache,
                    to_frame_point(
                        position[0] as f64,
                        position[1] as f64,
                        &context.frame_layout,
                    ),
                    context.combo_info[id].color,
                    alpha,
                    &context.combo_info[id].number.to_string(),
                );
            }
        }
    }
    for &id in visible {
        if context.hit_objects[id].hit_type & 8 == 0
            && (failed || gameplay.objects[id].head.is_none_or(|h| h.0 > time as f64))
        {
            frame::draw_approach_circle(
                &mut image,
                context,
                cache,
                id,
                context.combo_info[id].color,
                time,
            );
        }
    }
    if let Some(period) = frame::current_break_period(breaks, time) {
        frame::draw_break_overlay(&mut image, period, time, context);
    }
    image
}

/// 最终背景合成之后调用，确保 Argon 的加法文字和粒子不会被 src-over 再次暗化。
pub fn draw_overlay(
    image: &mut Img,
    context: &RenderContext,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    visuals: &ReplayVisuals,
    time: i64,
) {
    feedback::draw(
        image,
        context,
        prepared,
        gameplay,
        &visuals.feedback,
        time as f64,
    );
    if let Some(flashlight) = &visuals.flashlight {
        flashlight.at(context, time).apply(
            image,
            crate::render::geometry::PixelRect {
                x: 0,
                y: 0,
                width: context.frame_layout.frame_width,
                height: context.frame_layout.frame_height,
            },
        );
    }
    hud::draw(
        image,
        context,
        gameplay,
        visuals,
        time as f64,
        prepared.last_end.max(gameplay.settlement_time).ceil() as i64,
        prepared.od,
    );
}

/// 回放判定控制消退时间，物件主体仍受 Mod 可见性约束；Miss 的掉落和文字由反馈层绘制。
pub(crate) fn head_alpha(
    context: &RenderContext,
    start: i64,
    judgement: Option<(f64, crate::gameplay::events::HitResult)>,
    time: i64,
    fade_early: bool,
    od: f64,
) -> f64 {
    let alpha = match judgement.filter(|j| j.0 <= time as f64) {
        Some((hit, result)) => (1.0
            - (time as f64 - hit) / if result.is_hit() { 120.0 } else { 100.0 })
        .clamp(0.0, 1.0),
        None => super::alpha::normal_object_alpha(start, time.max(start), time, &context.settings),
    };
    // 命中或漏判的消退动画不能重置 HD 透明度；圆圈与滑条头继续使用普通预览的消隐时间轴。
    // 仅隐藏接近圈的 HD 设置和 TC 不隐藏物件主体，仍保留原有判定淡出。
    let alpha = if context.settings.hidden
        && !context.settings.traceable
        && !context.settings.hidden_only_approach
    {
        alpha.min(super::alpha::object_alpha(
            start,
            start,
            time,
            &context.settings,
        ))
    } else {
        alpha
    };
    if fade_early && judgement.is_none_or(|(hit, result)| hit > time as f64 || !result.is_hit()) {
        // 官方 CL 从 100 窗结束开始淡出，在 50 窗结束时完全透明；命中后的动画独立。
        let windows = crate::replay::calculate::standard::circle::windows(od);
        alpha.min(
            ((start as f64 + windows[2] - time as f64) / (windows[2] - windows[1])).clamp(0.0, 1.0),
        )
    } else {
        alpha
    }
}

/// 原生场景共享 CPU 回放 HUD 与 Argon 反馈参数，不上传整帧位图。
pub fn draw_scene_overlay(
    scene: &mut crate::render::scene::FrameSceneBuilder,
    context: &RenderContext,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    visuals: &ReplayVisuals,
    time: i64,
) {
    feedback::draw(
        scene,
        context,
        prepared,
        gameplay,
        &visuals.feedback,
        time as f64,
    );
    if let Some(flashlight) = &visuals.flashlight {
        flashlight.at(context, time).draw_scene(
            scene,
            crate::render::geometry::PixelRect {
                x: 0,
                y: 0,
                width: context.frame_layout.frame_width,
                height: context.frame_layout.frame_height,
            },
        );
    }
    hud::draw(
        scene,
        context,
        gameplay,
        visuals,
        time as f64,
        prepared.last_end.max(gameplay.settlement_time).ceil() as i64,
        prepared.od,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn replay_hud_keeps_layout_scale_outside_config_scope() {
        use crate::render::geometry::OutputFormat;
        use crate::render::scene::FrameSceneBuilder;

        let (beatmap, prepared, gameplay, _) = mod_fixture(serde_json::json!([]));
        for scale in [0.75, 1.5] {
            let mut config = crate::config::CoreConfig::default();
            config.render.standard.png.SCALE = scale;
            config.render.standard.gif.SCALE = scale + 0.5;
            config.render.standard.mp4.SCALE = scale + 1.0;
            let config = Arc::new(config);
            for (format, output_scale) in [
                (OutputFormat::Png, scale),
                (OutputFormat::Gif, scale + 0.5),
                (OutputFormat::Mp4, scale + 1.0),
            ] {
                let context = crate::config::with_config(Arc::clone(&config), || {
                    build_replay_context(
                        &beatmap,
                        &prepared,
                        crate::processing::timeline::TimeAxis::new(1000),
                        format,
                    )
                });
                // 准备和逐帧绘制都可能在原配置作用域外执行，倍率应只取自已有布局。
                let visuals = ReplayVisuals::new(&context, &gameplay, None, &[], prepared.od);
                assert_eq!(
                    visuals.hit_error_axis.w,
                    (150.0 * output_scale).round() as u32
                );
                let draw = || {
                    let mut scene = FrameSceneBuilder::new(
                        context.frame_layout.frame_width as u32,
                        context.frame_layout.frame_height as u32,
                        500,
                    );
                    hud::draw(
                        &mut scene,
                        &context,
                        &gameplay,
                        &visuals,
                        500.0,
                        2000,
                        prepared.od,
                    );
                    scene.finish()
                };
                let expected = crate::config::with_config(Arc::clone(&config), draw);
                let actual = draw();
                assert_eq!(
                    format!("{:?}", actual.commands),
                    format!("{:?}", expected.commands),
                    "{format:?}, SCALE={output_scale}"
                );
            }
        }
    }

    #[test]
    fn replay_hidden_and_approach_only_settings_control_object_visibility() {
        for (mods, visible) in [
            (serde_json::json!([]), true),
            (serde_json::json!([{"acronym":"HD"}]), false),
            (
                serde_json::json!([{"acronym":"HD","settings":{"only_fade_approach_circles":true}}]),
                true,
            ),
        ] {
            let (_, prepared, _, context) = mod_fixture(mods);
            let alpha = head_alpha(&context, 1000, None, 900, false, prepared.od);
            assert_eq!(alpha > 0.0, visible);
        }
        let (_, prepared, gameplay, context) = mod_fixture(serde_json::json!([{"acronym":"TC"}]));
        let actual = render_frame(
            &context,
            &mut RenderCache::default(),
            &prepared,
            &gameplay,
            1000,
            &[],
            &[0],
            None,
        );
        let background = driver::background_frame(&context, None);
        assert_eq!(actual.data, background.data);
    }

    #[test]
    fn hidden_judgements_do_not_restart_head_visibility() {
        use crate::gameplay::events::HitResult;
        for mods in [
            serde_json::json!([{"acronym":"HD"}]),
            serde_json::json!([{"acronym":"HD"},{"acronym":"CL"}]),
        ] {
            let (_, prepared, _, context) = mod_fixture(mods);
            for result in [
                HitResult::Great,
                HitResult::Ok,
                HitResult::Meh,
                HitResult::Miss,
            ] {
                for hit in [1000.0, 950.0, 1050.0, 500.0] {
                    for time in [400, 600, 900, 999, 1000, 1001, 1050, 1119, 1200] {
                        let expected =
                            super::super::alpha::object_alpha(1000, 1000, time, &context.settings);
                        let actual = head_alpha(
                            &context,
                            1000,
                            Some((hit, result)),
                            time,
                            false,
                            prepared.od,
                        );
                        assert!(actual <= expected, "HD 判定不能让物件重新出现：{result:?}/{hit}/{time}: {actual} > {expected}");
                    }
                }
            }
        }
        // 普通模式及仅隐藏接近圈的 HD 设置仍保留命中后的淡出动画。
        for mods in [
            serde_json::json!([]),
            serde_json::json!([{"acronym":"HD","settings":{"only_fade_approach_circles":true}}]),
        ] {
            let (_, prepared, _, context) = mod_fixture(mods);
            assert_eq!(
                head_alpha(
                    &context,
                    1000,
                    Some((1000.0, HitResult::Great)),
                    1000,
                    false,
                    prepared.od
                ),
                1.0
            );
            assert!(
                (head_alpha(
                    &context,
                    1000,
                    Some((1000.0, HitResult::Great)),
                    1060,
                    false,
                    prepared.od
                ) - 0.5)
                    .abs()
                    < 1e-9
            );
        }
    }

    #[test]
    fn hidden_cpu_circle_and_slider_head_stay_invisible_after_judgement() {
        use crate::gameplay::events::HitResult;
        for object in ["100,100,1000,1,0", "100,100,1000,2,0,L|300:100,1,200"] {
            let (_, prepared, mut gameplay, context) =
                object_mod_fixture(serde_json::json!([{"acronym":"HD"}]), object);
            gameplay.failure_time = None;
            for time in [999, 1000, 1001, 1040, 1080, 1119, 1120] {
                gameplay.objects[0].head = None;
                let expected = render_frame(
                    &context,
                    &mut RenderCache::default(),
                    &prepared,
                    &gameplay,
                    time,
                    &[],
                    &[0],
                    None,
                );
                gameplay.objects[0].head = Some((1000.0, HitResult::Great));
                let actual = render_frame(
                    &context,
                    &mut RenderCache::default(),
                    &prepared,
                    &gameplay,
                    time,
                    &[],
                    &[0],
                    None,
                );
                assert_eq!(
                    actual.data, expected.data,
                    "HD 圆圈或滑条头在命中后重新出现：{object}/{time}"
                );
            }
        }
    }

    #[test]
    fn replay_flashlight_follows_recorded_cursor_combo_breaks_tracking_and_breaks() {
        let (_, _, mut gameplay, context) = mod_fixture(serde_json::json!([{"acronym":"FL"}]));
        let mut state = crate::gameplay::state::GameplayState {
            combo: 100,
            ..Default::default()
        };
        gameplay.checkpoints = vec![(1100.0, state.clone())];
        state.combo = 0;
        gameplay.checkpoints.push((1400.0, state));
        gameplay.objects[0].tracking = vec![(1150.0, true), (1250.0, false)];
        let breaks = [BreakPeriod {
            start_time: 2000,
            end_time: 3000,
        }];
        let visuals = ReplayVisuals::new(&context, &gameplay, None, &breaks, 9.0);
        let flashlight = visuals.flashlight.as_ref().unwrap();
        let initial = flashlight.at(&context, 1000);
        let expected = to_frame_point(400.0, 300.0, &context.frame_layout);
        assert_eq!(initial.center, [expected.0, expected.1]);
        assert_eq!(initial.radius, 125.0 * context.frame_layout.scale);
        let shrunk = flashlight.at(&context, 1200);
        assert_eq!(shrunk.radius, initial.radius * 0.8125);
        assert_eq!(shrunk.dim, 0.8);
        assert_eq!(flashlight.at(&context, 1300).dim, 0.0);
        assert_eq!(flashlight.at(&context, 1500).radius, initial.radius);
        assert_eq!(flashlight.at(&context, 2900).radius, initial.radius * 2.5);
        assert_eq!(flashlight.at(&context, 4000).radius, initial.radius);
        assert_eq!(flashlight.at(&context, 1200).center, shrunk.center);
        assert_eq!(flashlight.at(&context, 1200).radius, shrunk.radius);
    }

    /// 只在手工验收时导出原始像素；生产构建没有截图或样本入口。
    #[test]
    #[ignore = "需要 OSR_SAMPLE_DIR 和 OSR_SCREENSHOT_DIR 指定手工验收路径"]
    fn official_sample_renders_identically_after_seeking() {
        let root = std::path::PathBuf::from(std::env::var("OSR_SAMPLE_DIR").unwrap());
        let output = std::path::PathBuf::from(std::env::var("OSR_SCREENSHOT_DIR").unwrap());
        let beatmap =
            crate::parse_beatmap_bytes(&std::fs::read(root.join("4903207.osu")).unwrap()).unwrap();
        let replay = Arc::new(
            crate::replay::parse_osr(
                &std::fs::read(root.join("xuan_yuan-osu-6086051920.osr")).unwrap(),
            )
            .unwrap(),
        );
        let prepared = crate::replay::preparation::standard::prepare(&beatmap).unwrap();
        let gameplay = crate::gameplay::engine::compile_standard(&prepared, replay).unwrap();
        let context = build_replay_context(
            &beatmap,
            &prepared,
            crate::processing::timeline::TimeAxis::new(prepared.objects[0].source.start_time),
            crate::render::geometry::OutputFormat::Mp4,
        );
        let times = [31300, 31000, 31300];
        let visuals = ReplayVisuals::new(
            &context,
            &gameplay,
            None,
            &beatmap.break_periods,
            prepared.od,
        );
        let indexes = build_visible_indexes_by_snapshot(
            &context.hit_objects,
            &times,
            context.settings.preempt_ms,
        );
        let mut cache = RenderCache::default();
        let mut first = None;
        for (index, time) in times.into_iter().enumerate() {
            let mut image = render_frame(
                &context,
                &mut cache,
                &prepared,
                &gameplay,
                time,
                &beatmap.break_periods,
                &indexes[index],
                None,
            );
            draw_overlay(&mut image, &context, &prepared, &gameplay, &visuals, time);
            if index == 0 {
                first = Some(image.data.clone());
            }
            if index == 2 {
                assert_eq!(first.as_ref().unwrap(), &image.data);
                std::fs::create_dir_all(&output).unwrap();
                std::fs::write(output.join("osr-preview.rgba"), &image.data).unwrap();
                std::fs::write(
                    output.join("osr-preview.json"),
                    format!(
                        "{{\"width\":{},\"height\":{},\"time_ms\":{time}}}",
                        image.w, image.h
                    ),
                )
                .unwrap();
            }
        }
    }

    fn mod_fixture(
        mods: serde_json::Value,
    ) -> (
        crate::Beatmap,
        PreparedStandard,
        GameplayTimeline,
        RenderContext,
    ) {
        object_mod_fixture(mods, "100,100,1000,1,0")
    }

    fn object_mod_fixture(
        mods: serde_json::Value,
        objects: &str,
    ) -> (
        crate::Beatmap,
        PreparedStandard,
        GameplayTimeline,
        RenderContext,
    ) {
        let beatmap = crate::parse_beatmap_bytes(format!("osu file format v14\n[General]\nMode:0\n[Difficulty]\nCircleSize:5\nOverallDifficulty:5\nSliderMultiplier:1.4\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n{objects}\n").as_bytes()).unwrap();
        let mut document = crate::replay::parse_osr(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../osu-beatmap-preview-web/test/fixtures/replay-standard.osr"
        )))
        .unwrap();
        document.header.version = 30_000_016;
        document.metadata = Some(serde_json::from_value(serde_json::json!({"mods":mods})).unwrap());
        document.frames = [0, 2000]
            .into_iter()
            .enumerate()
            .map(|(sequence, time_ms)| crate::replay::ReplayFrame {
                sequence,
                time_ms,
                delta_ms: time_ms,
                cursor: [400.0, 300.0],
                keys: 0,
            })
            .collect();
        let prepared =
            crate::replay::preparation::standard::prepare_replay(&beatmap, &document).unwrap();
        let gameplay =
            crate::gameplay::engine::compile_standard(&prepared, Arc::new(document)).unwrap();
        let context = build_replay_context(
            &beatmap,
            &prepared,
            crate::processing::timeline::TimeAxis::new(1000),
            crate::render::geometry::OutputFormat::Mp4,
        );
        (beatmap, prepared, gameplay, context)
    }
}
