//! 回放 GIF 复用普通预览的选段和网格；每格独立查询同一份不可变回放时间线。
use super::{font, visible_indexes};
use crate::{
    application::{legacy::resolve_time_points, output_plan::RenderPlan},
    export::{canvas::Img, standard::draw_time_label, text::format_mmssmmm},
    media::image::save_animated_gif_with_frame_palettes,
};
use osu_beatmap_preview_core::{
    gameplay::timeline::GameplayTimeline,
    processing::timeline::{PreviewTimeSelection, TimeAxis},
    render::{
        cpu::modes::standard::{context::*, osr},
        geometry::OutputFormat,
    },
    replay::preparation::standard::PreparedStandard,
    support::timeout::RequestDeadline,
    Beatmap, PreviewError, Result,
};
use rayon::prelude::*;
use std::{cell::RefCell, collections::VecDeque, path::Path, sync::Arc};

pub(crate) fn render_gif(
    beatmap: &Beatmap,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    plan: &RenderPlan,
    path: &Path,
    deadline: &RequestDeadline,
) -> Result<()> {
    render_with_config(
        beatmap,
        prepared,
        gameplay,
        plan,
        path,
        deadline,
        crate::config::current(),
    )
}

fn render_with_config(
    beatmap: &Beatmap,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    plan: &RenderPlan,
    path: &Path,
    deadline: &RequestDeadline,
    runtime: &crate::config::RuntimeConfig,
) -> Result<()> {
    deadline.check()?;
    let axis = TimeAxis::new(prepared.objects[0].source.start_time);
    let context = build_replay_context(beatmap, prepared, axis, OutputFormat::Gif);
    let config = &runtime.render.standard.gif;
    let speed = prepared.mods.settings.speed_multiplier;
    let duration_ms = plan
        .duration_seconds
        .map_or(config.style.DURATION_MS as f64, |s| s * 1000.0);
    let fps = plan.fps.map_or(config.style.FPS as f64, f64::from);
    let cells = config
        .structure
        .ROW_COUNT
        .checked_mul(config.structure.IMAGES_PER_ROW)
        .filter(|&cells| cells > 0)
        .ok_or_else(|| PreviewError::render("回放 GIF 网格行列数必须为正数且不能溢出"))?;
    let frame_count = checked_frame_count(duration_ms, fps, speed, cells)?;
    let duration = py_round(duration_ms * speed);
    let requested = match &plan.selected_times {
        Some(times) => PreviewTimeSelection::Resolved(times.clone()),
        None => {
            PreviewTimeSelection::Requested(resolve_time_points(beatmap, axis, &plan.time_points)?)
        }
    };
    // 应用层已确定的时间点必须原样使用，只有直接调用渲染器时才补齐选段。
    let segments =
        choose_row_start_times(beatmap, &context.hit_objects, cells, 2, duration, requested)?;
    let (width, height) = gif_canvas_size();
    if !(1..=i64::from(u16::MAX)).contains(&width) || !(1..=i64::from(u16::MAX)).contains(&height) {
        return Err(PreviewError::render("回放 GIF 画布尺寸超出格式范围"));
    }
    let origins: Vec<_> = (0..segments.len()).map(gif_frame_origin).collect();
    let snapshots: Vec<Vec<i64>> = segments
        .iter()
        .map(|segment| {
            (0..frame_count)
                .map(|index| {
                    segment
                        .start_time
                        .saturating_add(py_round(index as f64 * 1000.0 * speed / fps))
                })
                .collect()
        })
        .collect();
    let indexes: Vec<Vec<Vec<usize>>> = snapshots
        .iter()
        .map(|times| {
            times
                .iter()
                .map(|&time| visible_indexes(prepared, gameplay, context.settings.preempt_ms, time))
                .collect()
        })
        .collect();
    let visuals = osr::ReplayVisuals::new(
        &context,
        gameplay,
        font::load()?,
        &beatmap.break_periods,
        prepared.od,
    );
    let core_config = Arc::new(osu_beatmap_preview_core::config::CoreConfig {
        render: runtime.render.clone(),
        skin: runtime.skin.clone(),
    });
    thread_local! { static CACHE: RefCell<RenderCache> = RefCell::new(RenderCache::default()); }
    // Rayon worker 不继承调用线程的配置作用域，每帧显式安装配置，保证缩放和颜色一致。
    let draw = |index: usize| {
        osu_beatmap_preview_core::config::with_config(Arc::clone(&core_config), || {
            let mut canvas = Img::new(
                width as u32,
                height as u32,
                config.style.CANVAS_BACKGROUND_COLOR,
            );
            for (cell, segment) in segments.iter().enumerate() {
                let time = snapshots[cell][index];
                let mut image = CACHE.with(|cache| {
                    osr::render_frame(
                        &context,
                        &mut cache.borrow_mut(),
                        prepared,
                        gameplay,
                        time,
                        &segment.break_periods,
                        &indexes[cell][index],
                        None,
                    )
                });
                osr::draw_overlay(&mut image, &context, prepared, gameplay, &visuals, time);
                let (x, y) = origins[cell];
                canvas.alpha_composite(&image, x, y);
                if config.style.SHOW_TIME_LABEL {
                    let label = format!(
                        "{} - {}",
                        format_mmssmmm(axis.to_display(segment.start_time)),
                        format_mmssmmm(
                            axis.to_display(segment.start_time.saturating_add(duration))
                        )
                    );
                    let color = if segment.is_preview {
                        config.style.PREVIEW_TIME_LABEL_COLOR
                    } else {
                        config.style.TIME_LABEL_COLOR
                    };
                    let note_color = if segment.is_preview {
                        config.style.PREVIEW_TIME_LABEL_COLOR
                    } else {
                        config.style.TIME_LABEL_NOTE_COLOR
                    };
                    draw_time_label(
                        &mut canvas,
                        &label,
                        x,
                        y + context.frame_layout.frame_height + config.sizing.TIME_LABEL_TOP_GAP,
                        segment.is_preview.then_some("Preview Time"),
                        context.frame_layout.frame_width,
                        config.sizing.TIME_LABEL_FONT_SIZE,
                        config.sizing.TIME_LABEL_NOTE_FONT_SIZE,
                        config.sizing.TIME_LABEL_NOTE_TOP_GAP,
                        color,
                        note_color,
                    );
                }
            }
            canvas
        })
    };
    let advanced = &runtime.advance.gif;
    let frame_bytes = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4)
        .max(1);
    let chunk = (advanced.MAX_PAR_FRAME_BYTES / frame_bytes).clamp(1, advanced.PAR_CHUNK_SIZE);
    let mut frames = VecDeque::new();
    // 延续回放的逐帧调色板，保留判定文字与淡出颜色；仅缓存受内存预算限制的一批画面。
    save_animated_gif_with_frame_palettes(
        frame_count,
        |index| {
            if frames.is_empty() {
                frames = (index..(index + chunk).min(frame_count))
                    .into_par_iter()
                    .map(&draw)
                    .collect::<Vec<_>>()
                    .into();
            }
            frames.pop_front().expect("每批按帧序消费")
        },
        path,
        fps,
        deadline,
    )
}

fn checked_frame_count(duration_ms: f64, fps: f64, speed: f64, cells: usize) -> Result<usize> {
    let count = (duration_ms * fps / 1000.0).round().max(1.0);
    // 先检查帧数与采样总量，再分配网格时间表；片段时长与普通 GIF 一样指输出时长。
    if !duration_ms.is_finite()
        || duration_ms <= 0.0
        || !fps.is_finite()
        || !(1.0..=60.0).contains(&fps)
        || !speed.is_finite()
        || speed <= 0.0
        || !(duration_ms * speed).is_finite()
        || duration_ms * speed >= i64::MAX as f64
        || !count.is_finite()
        || count > 1_000_000.0
        || cells == 0
        || (count as usize)
            .checked_mul(cells)
            .is_none_or(|total| total > 1_000_000)
    {
        return Err(PreviewError::render(
            "回放 GIF 片段时长、帧率或总采样帧数超出范围",
        ));
    }
    Ok(count as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_duration_is_independent_of_replay_speed_and_checks_limits() {
        for speed in [0.75, 1.0, 1.5] {
            assert_eq!(checked_frame_count(5000.0, 15.0, speed, 4).unwrap(), 75);
        }
        for (duration, fps, speed, cells) in [
            (f64::NAN, 15.0, 1.0, 4),
            (0.0, 15.0, 1.0, 4),
            (5000.0, 0.0, 1.0, 4),
            (5000.0, 15.0, f64::INFINITY, 4),
            (5000.0, 15.0, 1.0, usize::MAX),
            (5000.0, 15.0, 1.0, 0),
        ] {
            assert!(checked_frame_count(duration, fps, speed, cells).is_err());
        }
    }

    #[test]
    fn real_replay_renders_four_simultaneous_cells_and_rejects_excess_time_points() {
        use osu_beatmap_preview_core::{
            gameplay::engine::compile_standard,
            processing::validation::TimePoint,
            replay::{parse_osr, preparation::standard::prepare_replay},
        };
        let beatmap = osu_beatmap_preview_core::parse_beatmap_bytes(include_bytes!(
            "../../../../../osu-beatmap-preview-web/test/fixtures/replay-standard.osu"
        ))
        .unwrap();
        let document = Arc::new(
            parse_osr(include_bytes!(
                "../../../../../osu-beatmap-preview-web/test/fixtures/replay-standard.osr"
            ))
            .unwrap(),
        );
        let prepared = prepare_replay(&beatmap, &document).unwrap();
        let gameplay = compile_standard(&prepared, document).unwrap();
        // 缩小实际光栅和编码尺寸，让集成测试覆盖四格而不承担大画布的调色板开销。
        let mut runtime = crate::config::current().clone();
        let defaults: serde_json::Value =
            serde_yaml::from_str(crate::config::default_config_yaml()).unwrap();
        let mut gif = defaults["render"]["standard"]["gif"].clone();
        gif["SCALE"] = serde_json::json!(0.125);
        for value in gif["sizing"].as_object_mut().unwrap().values_mut() {
            *value = serde_json::json!((value.as_f64().unwrap() * 0.125).round() as i64);
        }
        runtime.render.standard.gif = serde_json::from_value(gif).unwrap();
        let core_config = Arc::new(osu_beatmap_preview_core::config::CoreConfig {
            render: runtime.render.clone(),
            skin: runtime.skin.clone(),
        });
        let mut plan = RenderPlan {
            selected_times: None,
            bid: "42".into(),
            target_mode: 0,
            convert_used: false,
            mods: Some(prepared.mods.settings.clone()),
            time_points: [0.0, 0.1, 0.2, 0.3]
                .into_iter()
                .map(TimePoint::Seconds)
                .collect(),
            duration_seconds: Some(0.2),
            format: crate::application::output_plan::OutputFormat::Gif,
            fps: Some(10),
            requested_scale: None,
            no_cache: true,
        };
        let path = std::env::temp_dir().join(format!(
            "replay-grid-{}-{}.gif",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let deadline = RequestDeadline::new(
            std::time::Instant::now(),
            "gif",
            std::time::Duration::from_secs(30),
        );
        osu_beatmap_preview_core::config::with_config(Arc::clone(&core_config), || {
            render_with_config(
                &beatmap, &prepared, &gameplay, &plan, &path, &deadline, &runtime,
            )
        })
        .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(std::io::Cursor::new(bytes)).unwrap();
        let (width, height) = osu_beatmap_preview_core::config::with_config(
            Arc::clone(&core_config),
            gif_canvas_size,
        );
        assert_eq!(
            (i64::from(decoder.width()), i64::from(decoder.height())),
            (width, height)
        );
        let first = decoder.read_next_frame().unwrap().unwrap().buffer.to_vec();
        let context =
            osu_beatmap_preview_core::config::with_config(Arc::clone(&core_config), || {
                build_replay_context(&beatmap, &prepared, TimeAxis::new(1000), OutputFormat::Gif)
            });
        let crop = |cell| {
            let (x, y) =
                osu_beatmap_preview_core::config::with_config(Arc::clone(&core_config), || {
                    gif_frame_origin(cell)
                });
            (y..y + context.frame_layout.frame_height)
                .flat_map(|row| {
                    let start = ((row * width + x) * 4) as usize;
                    first[start..start + context.frame_layout.frame_width as usize * 4]
                        .iter()
                        .copied()
                })
                .collect::<Vec<_>>()
        };
        for cell in 0..4 {
            assert!(crop(cell).chunks_exact(4).any(|p| p[..3] != [0, 0, 0]));
        }
        // 四格查询不同绝对时刻；这能发现误把同一张回放帧复制到全部格子的实现。
        assert_ne!(crop(0), crop(3));
        assert!(decoder.read_next_frame().unwrap().is_some());
        assert!(decoder.read_next_frame().unwrap().is_none());
        plan.time_points.push(TimePoint::Seconds(0.4));
        assert!(
            osu_beatmap_preview_core::config::with_config(core_config, || {
                render_with_config(
                    &beatmap, &prepared, &gameplay, &plan, &path, &deadline, &runtime,
                )
            })
            .is_err()
        );
        assert!(!path.exists());
    }
}
