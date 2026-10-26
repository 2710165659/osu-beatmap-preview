//! Taiko 回放 GIF：每格独立采样真实回放，复用普通 GIF 的布局、选段和调色板。
use crate::{
    application::{legacy::resolve_time_points, output_plan::RenderPlan},
    export::replay::{font, time_plan::checked_gif_frame_count},
    media::image::save_animated_gif_streamed_with_colors,
};
use osu_beatmap_preview_core::{
    gameplay::timeline::GameplayTimeline,
    processing::timeline::{PreviewTimeSelection, TimeAxis},
    render::{
        canvas::Img,
        cpu::modes::taiko::{animation_render::*, constants},
        geometry::OutputFormat,
        shared::taiko::replay::ReplayRenderPlan,
    },
    replay::preparation::taiko::PreparedTaiko,
    support::timeout::RequestDeadline,
    PreviewError, Result,
};
use std::{path::Path, sync::Arc};

pub(crate) fn render_gif(
    prepared: Arc<PreparedTaiko>,
    gameplay: Arc<GameplayTimeline>,
    plan: &RenderPlan,
    path: &Path,
    deadline: &RequestDeadline,
) -> Result<()> {
    render_with_config(
        prepared,
        gameplay,
        plan,
        path,
        deadline,
        crate::config::current(),
    )
}

fn render_with_config(
    prepared: Arc<PreparedTaiko>,
    gameplay: Arc<GameplayTimeline>,
    plan: &RenderPlan,
    path: &Path,
    deadline: &RequestDeadline,
    runtime: &crate::config::RuntimeConfig,
) -> Result<()> {
    deadline.check()?;
    let gif = &runtime.render.taiko.gif;
    let cells = gif
        .structure
        .ROW_COUNT
        .checked_mul(gif.structure.IMAGES_PER_ROW)
        .filter(|&cells| cells > 0)
        .ok_or_else(|| PreviewError::render("回放 GIF 网格行列数必须为正数且不能溢出"))?;
    let speed = prepared.mods.settings.speed_multiplier;
    let duration_ms = plan
        .duration_seconds
        .map_or(gif.style.DURATION_MS, |seconds| seconds * 1000.0);
    let fps = plan.fps.map_or(gif.style.FPS, f64::from);
    let frame_count = checked_gif_frame_count(duration_ms, fps, speed, cells)?;
    let duration = pyround(duration_ms * speed);
    let axis = TimeAxis::new(prepared.objects[0].source.start_time);
    let requested = match &plan.selected_times {
        Some(times) => PreviewTimeSelection::Resolved(times.clone()),
        None => PreviewTimeSelection::Requested(resolve_time_points(
            &prepared.beatmap,
            axis,
            &plan.time_points,
        )?),
    };
    let spans = prepared
        .objects
        .iter()
        .map(|object| (object.source.start_time, object.source.end_time))
        .collect();
    let segments = requested.choose(&prepared.beatmap, spans, cells, duration)?;
    let renderer = ReplayRenderPlan::with_format(
        Arc::clone(&prepared),
        gameplay,
        font::load()?,
        None,
        OutputFormat::Gif,
    )?;
    let layout = &renderer.layout;
    let (width, height) = gif_canvas_size(layout);
    if !(1..=i64::from(u16::MAX)).contains(&width) || !(1..=i64::from(u16::MAX)).contains(&height) {
        return Err(PreviewError::render("回放 GIF 画布尺寸超出格式范围"));
    }
    let origins: Vec<_> = (0..segments.len())
        .map(|index| gif_frame_origin(index, layout))
        .collect();
    let blank = Img::new(
        layout.image_width as u32,
        layout.image_height as u32,
        constants::gif::style::IMAGE_BACKGROUND,
    );
    let config = Arc::new(osu_beatmap_preview_core::config::CoreConfig {
        render: runtime.render.clone(),
        skin: runtime.skin.clone(),
    });
    // 编码器并行绘帧时显式安装调用线程的配置，避免字号、样式和缩放回退。
    let draw = |index: usize| {
        osu_beatmap_preview_core::config::with_config(Arc::clone(&config), || {
            let mut canvas = Img::new(
                width as u32,
                height as u32,
                constants::gif::style::IMAGE_BACKGROUND,
            );
            let offset = pyround(index as f64 * 1000.0 * speed / fps);
            for (cell, segment) in segments.iter().enumerate() {
                let time = segment.start_time.saturating_add(offset);
                let mut image = blank.clone();
                renderer.draw_frame(&mut image, time);
                renderer.draw_overlay(&mut image, time);
                let origin = origins[cell];
                canvas.alpha_composite(&image, origin.0, origin.1);
                if constants::gif::style::SHOW_TIME_LABEL {
                    draw_time_label(
                        &mut canvas,
                        segment.start_time,
                        duration,
                        origin,
                        layout,
                        segment.is_preview,
                        axis,
                    );
                }
            }
            canvas
        })
    };
    save_animated_gif_streamed_with_colors(
        frame_count,
        draw,
        path,
        fps,
        deadline,
        &super::super::gif_fixed_colors(),
        prepared.mods.settings.hidden || prepared.mods.settings.flashlight,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use osu_beatmap_preview_core::{
        gameplay::{engine::compile_taiko, events::HitResult},
        processing::validation::TimePoint,
        replay::{parse_osr, preparation::taiko::prepare_replay, ReplayFrame},
    };
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[test]
    fn replay_gif_keeps_independent_cells_mod_clock_and_format_settings() {
        let beatmap = osu_beatmap_preview_core::parse_beatmap_bytes(
            b"osu file format v14\n[General]\nMode:1\nPreviewTime:1500\n[Difficulty]\nOverallDifficulty:5\nHPDrainRate:5\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n0,0,1000,1,0\n0,0,1500,1,2\n0,0,2000,1,4\n0,0,2500,2,0,L|140:0,1,140\n0,0,3500,8,0,4000\n0,0,4500,1,0\n0,0,5500,1,0\n",
        ).unwrap();
        let fixture = parse_osr(include_bytes!(
            "../../../../../osu-beatmap-preview-web/test/fixtures/replay-standard.osr"
        ))
        .unwrap();
        for (mods, scale, rows, columns) in [
            (0, 0.25, 2, 2),
            (64, 0.375, 1, 4),
            (256, 0.25, 4, 1),
            (8 | 1024, 0.25, 2, 2),
        ] {
            let mut document = fixture.clone();
            document.header.ruleset_id = 1;
            document.header.mods_bits = mods;
            document.frames = [
                (0, 0),
                (980, 1),
                (1000, 0),
                (1540, 2),
                (1560, 0),
                (2000, 5),
                (2020, 0),
                (2500, 1),
                (2510, 0),
                (2700, 4),
                (2710, 0),
                (3500, 1),
                (3510, 0),
                (3550, 2),
                (3560, 0),
                (3600, 4),
                (3610, 0),
                (3650, 8),
                (3660, 0),
                (5000, 0),
            ]
            .into_iter()
            .enumerate()
            .map(|(sequence, (time_ms, keys))| ReplayFrame {
                delta_ms: 0,
                time_ms,
                cursor: [0.0; 2],
                keys,
                sequence,
            })
            .collect();
            let document = Arc::new(document);
            let prepared = Arc::new(prepare_replay(&beatmap, &document).unwrap());
            let gameplay = Arc::new(compile_taiko(&prepared, document).unwrap());
            assert_eq!(gameplay.objects[0].head.unwrap().1, HitResult::Great);
            assert_eq!(gameplay.objects[1].head.unwrap().1, HitResult::Ok);
            assert_eq!(gameplay.objects[5].head.unwrap().1, HitResult::Miss);
            assert_eq!(gameplay.failure_time, Some(5000.0));
            let mut runtime = crate::config::current().clone();
            runtime.render.taiko.gif.SCALE = scale;
            runtime.render.taiko.gif.structure.ROW_COUNT = rows;
            runtime.render.taiko.gif.structure.IMAGES_PER_ROW = columns;
            runtime.render.taiko.gif.style.SHOW_MEASURE_LINES = false;
            runtime.render.taiko.mp4.SCALE = 2.5;
            runtime.render.taiko.mp4.style.SHOW_MEASURE_LINES = true;
            runtime.render.standard.gif.SCALE = 3.0;
            let core = Arc::new(osu_beatmap_preview_core::config::CoreConfig {
                render: runtime.render.clone(),
                skin: runtime.skin.clone(),
            });
            let mut plan = RenderPlan {
                selected_times: Some(vec![900, 1500, 2700, 3600]),
                bid: "42".into(),
                target_mode: 1,
                convert_used: false,
                mods: Some(prepared.mods.settings.clone()),
                time_points: vec![],
                duration_seconds: Some(0.2),
                format: crate::application::output_plan::OutputFormat::Gif,
                fps: Some(10),
                requested_scale: None,
                no_cache: true,
            };
            let path = std::env::temp_dir().join(format!(
                "taiko-replay-gif-{}-{mods}-{}.gif",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            ));
            let deadline = RequestDeadline::new(Instant::now(), "gif", Duration::from_secs(60));
            osu_beatmap_preview_core::config::with_config(Arc::clone(&core), || {
                render_with_config(
                    Arc::clone(&prepared),
                    Arc::clone(&gameplay),
                    &plan,
                    &path,
                    &deadline,
                    &runtime,
                )
            })
            .unwrap();
            let mut options = gif::DecodeOptions::new();
            options.set_color_output(gif::ColorOutput::RGBA);
            let mut decoder = options
                .read_info(std::io::Cursor::new(std::fs::read(&path).unwrap()))
                .unwrap();
            std::fs::remove_file(&path).unwrap();
            osu_beatmap_preview_core::config::with_config(Arc::clone(&core), || {
                let renderer = ReplayRenderPlan::with_format(
                    Arc::clone(&prepared),
                    Arc::clone(&gameplay),
                    font::load().unwrap(),
                    None,
                    OutputFormat::Gif,
                )
                .unwrap();
                let layout = &renderer.layout;
                let expected_layout =
                    build_video_animation_layout(compute_time_range(), OutputFormat::Gif);
                assert_eq!(
                    (layout.image_width, layout.image_height),
                    (expected_layout.image_width, expected_layout.image_height)
                );
                assert_eq!(layout.output_format, OutputFormat::Gif);
                let (width, height) = gif_canvas_size(layout);
                assert_eq!(
                    (i64::from(decoder.width()), i64::from(decoder.height())),
                    (width, height)
                );
                assert!(decoder.global_palette().is_some());
                let mut canvas = Img::new(
                    width as u32,
                    height as u32,
                    constants::gif::style::IMAGE_BACKGROUND,
                );
                for index in 0..2 {
                    let frame = decoder.read_next_frame().unwrap().unwrap();
                    assert_eq!(frame.delay, 10);
                    for y in 0..frame.height as usize {
                        for x in 0..frame.width as usize {
                            let pixel = &frame.buffer[(y * frame.width as usize + x) * 4..][..4];
                            if pixel[3] != 0 {
                                let offset = ((y + frame.top as usize) * width as usize
                                    + x
                                    + frame.left as usize)
                                    * 4;
                                canvas.data[offset..offset + 4].copy_from_slice(pixel);
                            }
                        }
                    }
                    for (cell, start) in plan.selected_times.as_ref().unwrap().iter().enumerate() {
                        let time = start
                            + pyround(
                                index as f64 * 1000.0 * prepared.mods.settings.speed_multiplier
                                    / 10.0,
                            );
                        let mut expected = Img::new(
                            layout.image_width as u32,
                            layout.image_height as u32,
                            constants::gif::style::IMAGE_BACKGROUND,
                        );
                        renderer.draw_frame(&mut expected, time);
                        renderer.draw_overlay(&mut expected, time);
                        let (x, y) = gif_frame_origin(cell, layout);
                        let error: u64 = (0..expected.h)
                            .map(|row| {
                                let offset = (((y + i64::from(row)) * width + x) * 4) as usize;
                                canvas.data[offset..offset + expected.w as usize * 4]
                                    .iter()
                                    .zip(
                                        &expected.data[row as usize * expected.w as usize * 4..]
                                            [..expected.w as usize * 4],
                                    )
                                    .map(|(a, b)| u64::from(a.abs_diff(*b)))
                                    .sum::<u64>()
                            })
                            .sum();
                        assert!(
                            error as f64 / ((expected.w * expected.h * 4) as f64) < 4.0,
                            "每格必须匹配对应回放时刻：{mods}/{cell}/{index}"
                        );
                    }
                }
                assert!(decoder.read_next_frame().unwrap().is_none());
                assert!(canvas
                    .data
                    .chunks_exact(4)
                    .any(|p| p == constants::gif::style::PREVIEW_TIME_LABEL_COLOR));
            });
            // 已选片段不得重新补点，超出容量或无效网格须在创建产物前拒绝。
            plan.selected_times = Some(vec![900]);
            osu_beatmap_preview_core::config::with_config(Arc::clone(&core), || {
                render_with_config(
                    Arc::clone(&prepared),
                    Arc::clone(&gameplay),
                    &plan,
                    &path,
                    &deadline,
                    &runtime,
                )
            })
            .unwrap();
            std::fs::remove_file(&path).unwrap();
            plan.selected_times = None;
            plan.time_points = (0..5).map(|i| TimePoint::Seconds(i as f64 * 0.1)).collect();
            assert!(
                osu_beatmap_preview_core::config::with_config(Arc::clone(&core), || {
                    render_with_config(
                        Arc::clone(&prepared),
                        Arc::clone(&gameplay),
                        &plan,
                        &path,
                        &deadline,
                        &runtime,
                    )
                })
                .is_err()
            );
            assert!(!path.exists());
            runtime.render.taiko.gif.structure.ROW_COUNT = 0;
            assert!(
                render_with_config(prepared, gameplay, &plan, &path, &deadline, &runtime).is_err()
            );
        }
    }
}
