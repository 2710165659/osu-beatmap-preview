//! 回放显示规则、布局与两种绘制适配的回归验证。
#![cfg(test)]

use super::*;
use crate::render::cpu::modes::standard::osr::render_frame;
use std::sync::Arc;

#[test]
fn cpu_and_scene_keep_earlier_objects_on_top_in_every_format() {
    use crate::render::{geometry::OutputFormat, scene::DrawCommand};

    let (mut beatmap, prepared, gameplay, _) =
        object_mod_fixture(serde_json::json!([]), "100,100,1000,1,0\n120,100,1100,5,0");
    let prepared = Arc::new(prepared);
    let gameplay = Arc::new(gameplay);
    beatmap.combo_colors = vec![[0, 0, 255], [255, 0, 0]];
    for scale in [0.75, 1.5] {
        let mut config = crate::config::CoreConfig::default();
        config.render.standard.png.SCALE = scale;
        config.render.standard.gif.SCALE = scale + 0.25;
        config.render.standard.mp4.SCALE = scale + 0.5;

        let config = Arc::new(config);
        for format in [OutputFormat::Png, OutputFormat::Gif, OutputFormat::Mp4] {
            let context = crate::config::with_config(Arc::clone(&config), || {
                build_replay_context(
                    &beatmap,
                    &prepared,
                    crate::processing::timeline::TimeAxis::new(1000),
                    format,
                )
            });
            let plan = ReplayRenderPlan::new(&context, &prepared, &gameplay);
            let mut cache = RenderCache::default();
            let mut render = |time| {
                render_frame(
                    &context,
                    &mut cache,
                    &prepared,
                    &gameplay,
                    time,
                    &[],
                    &plan,
                    None,
                )
            };
            let first = render(900);
            let point = to_frame_point(110.0, 80.0, &context.frame_layout);
            let pixel = first.get(point.0.round() as u32, point.1.round() as u32);
            assert!(
                pixel[0] > pixel[2],
                "较早的红色圆圈必须覆盖蓝色圆圈：{format:?}/{scale}/{pixel:?}"
            );
            let content = context.frame_layout.content;
            assert_eq!(
                first.get(content.x as u32 + 5, content.y as u32 + 5),
                context.style.background_color
            );
            render(1400);
            assert_eq!(first.data, render(900).data);

            if format == OutputFormat::Mp4 {
                let source = crate::config::with_config(Arc::clone(&config), || {
                    crate::render::wgpu::modes::standard::osr::prepare(
                        &beatmap,
                        Arc::clone(&prepared),
                        Arc::clone(&gameplay),
                        None,
                        (first.w, first.h),
                    )
                })
                .unwrap();
                let scene = source.render(900).unwrap();
                let centers: Vec<_> = scene
                    .commands
                    .iter()
                    .filter_map(|command| match command {
                        DrawCommand::Ring {
                            center,
                            radius,
                            color,
                            ..
                        } if *color == [255; 4]
                            && (*radius - context.frame_circle_diameter as f32 / 2.0).abs()
                                < 0.001 =>
                        {
                            Some(*center)
                        }
                        _ => None,
                    })
                    .collect();
                let expected: Vec<_> = [1, 0]
                    .into_iter()
                    .map(|id| {
                        let position = prepared.objects[id].position;
                        let point = to_frame_point(
                            f64::from(position[0]),
                            f64::from(position[1]),
                            &context.frame_layout,
                        );
                        [point.0 as f32, point.1 as f32]
                    })
                    .collect();
                assert_eq!(centers, expected);
            }
        }
    }
}

#[test]
fn late_judgement_survives_nominal_culling_and_failure_uses_preview_lifetime() {
    use crate::gameplay::events::HitResult;
    use crate::render::scene::DrawCommand;

    for failure in [None, Some(1200.0)] {
        let (beatmap, prepared, mut gameplay, context) = mod_fixture(serde_json::json!([]));
        gameplay.failure_time = failure;
        gameplay.objects[0].head = Some((1300.0, HitResult::Great));
        let plan = ReplayRenderPlan::new(&context, &prepared, &gameplay);
        let image = render_frame(
            &context,
            &mut RenderCache::default(),
            &prepared,
            &gameplay,
            1320,
            &[],
            &plan,
            None,
        );
        let point = to_frame_point(100.0, 80.0, &context.frame_layout);
        let pixel = image.get(point.0.round() as u32, point.1.round() as u32);
        assert_eq!(pixel != context.style.background_color, failure.is_none());

        let source = crate::render::wgpu::modes::standard::osr::prepare(
            &beatmap,
            Arc::new(prepared),
            Arc::new(gameplay),
            None,
            (image.w, image.h),
        )
        .unwrap();
        let scene = source.render(1320).unwrap();
        let radius = context.frame_circle_diameter as f32 / 2.0;
        let has_head = scene.commands.iter().any(|command| {
            matches!(command,
                DrawCommand::Ring { radius: r, color, .. } if *r == radius && color[3] > 0
            )
        });
        assert_eq!(has_head, failure.is_none());
    }
}

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
                visuals.hud.hit_error_axis.w,
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
        &ReplayRenderPlan::new(&context, &prepared, &gameplay),
        None,
    );
    let background = crate::render::cpu::modes::standard::render_frame(
        &context,
        &mut RenderCache::default(),
        900,
        &[],
        &[],
        None,
    );
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
                    let expected = crate::render::cpu::modes::standard::alpha::object_alpha(
                        1000,
                        1000,
                        time,
                        &context.settings,
                    );
                    let actual = head_alpha(
                        &context,
                        1000,
                        Some((hit, result)),
                        time,
                        false,
                        prepared.od,
                    );
                    assert!(
                        actual <= expected,
                        "HD 判定不能让物件重新出现：{result:?}/{hit}/{time}: {actual} > {expected}"
                    );
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
                &ReplayRenderPlan::new(&context, &prepared, &gameplay),
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
                &ReplayRenderPlan::new(&context, &prepared, &gameplay),
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
    let plan = ReplayRenderPlan::new(&context, &prepared, &gameplay);
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
            &plan,
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

pub(super) fn object_mod_fixture(
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
