//! 回放 GIF 复用普通预览的选段和网格；每格独立查询同一份不可变回放时间线。
use osu_beatmap_preview_core::render::cpu::modes::standard::constants as standard_constants;

use super::font;
use crate::{
    application::{legacy::resolve_time_points, output_plan::RenderPlan},
    export::{canvas::Img, standard::draw_time_label, text::format_mmssmmm},
    media::image::save_animated_gif_streamed_with_colors,
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
use std::{cell::RefCell, path::Path, sync::Arc};

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
    let standard_gif_sizing = standard_constants::gif::sizing();
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
    let render_plan = osr::ReplayRenderPlan::new(&context, prepared, gameplay);
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
                standard_constants::gif::style::CANVAS_BACKGROUND_COLOR,
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
                        &render_plan,
                        None,
                    )
                });
                osr::draw_overlay(&mut image, &context, prepared, gameplay, &visuals, time);
                let (x, y) = origins[cell];
                canvas.alpha_composite(&image, x, y);
                if standard_constants::gif::style::SHOW_TIME_LABEL {
                    let label = format!(
                        "{} - {}",
                        format_mmssmmm(axis.to_display(segment.start_time)),
                        format_mmssmmm(
                            axis.to_display(segment.start_time.saturating_add(duration))
                        )
                    );
                    let color = if segment.is_preview {
                        standard_constants::gif::style::PREVIEW_TIME_LABEL_COLOR
                    } else {
                        standard_constants::gif::style::TIME_LABEL_COLOR
                    };
                    let note_color = if segment.is_preview {
                        standard_constants::gif::style::PREVIEW_TIME_LABEL_COLOR
                    } else {
                        standard_constants::gif::style::TIME_LABEL_NOTE_COLOR
                    };
                    draw_time_label(
                        &mut canvas,
                        &label,
                        x,
                        y + context.frame_layout.frame_height
                            + standard_gif_sizing.TIME_LABEL_TOP_GAP,
                        segment.is_preview.then_some("Preview Time"),
                        context.frame_layout.frame_width,
                        standard_gif_sizing.TIME_LABEL_FONT_SIZE,
                        standard_gif_sizing.TIME_LABEL_NOTE_FONT_SIZE,
                        standard_gif_sizing.TIME_LABEL_NOTE_TOP_GAP,
                        color,
                        note_color,
                    );
                }
            }
            canvas
        })
    };
    let mut fixed_colors = super::super::super::gif_time_label_colors(
        standard_constants::gif::style::SHOW_TIME_LABEL,
        standard_constants::gif::style::CANVAS_BACKGROUND_COLOR,
        [
            standard_constants::gif::style::PREVIEW_TIME_LABEL_COLOR,
            standard_constants::gif::style::TIME_LABEL_COLOR,
            standard_constants::gif::style::TIME_LABEL_NOTE_COLOR,
        ],
    );
    // 回放包含大量接近背景的淡出颜色，量化器可能把纯黑背景偏向暗紫色。
    // 预留画布与游玩区域实际合成的底色，保证空白区域不被全局调色板染色。
    let [r, g, b, _] = standard_constants::gif::style::CANVAS_BACKGROUND_COLOR;
    fixed_colors.push([r, g, b]);
    let mut background = Img::new(
        1,
        1,
        standard_constants::gif::style::CANVAS_BACKGROUND_COLOR,
    );
    background.blend_px(0, 0, standard_constants::gif::style::IMAGE_BACKGROUND_COLOR);
    fixed_colors.push([background.data[0], background.data[1], background.data[2]]);
    // 与普通预览共用限权采样、全局调色板和差分帧，避免逐帧全画布量化。
    // 编码器统一管理并行绘图的内存预算；标签预留精确合成色，防止再次发灰。
    save_animated_gif_streamed_with_colors(
        frame_count,
        draw,
        path,
        fps,
        deadline,
        &fixed_colors,
        false,
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
        // 缩小实际光栅和编码尺寸，让集成测试覆盖四格并保持执行时间可控。
        let mut runtime = crate::config::current().clone();
        runtime.render.standard.gif.SCALE = 0.125;
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
        assert!(decoder.global_palette().is_some());
        let first = {
            let frame = decoder.read_next_frame().unwrap().unwrap();
            assert_eq!(frame.delay, 10);
            frame.buffer.to_vec()
        };
        let preview_color = standard_constants::gif::style::PREVIEW_TIME_LABEL_COLOR;
        assert!(first.chunks_exact(4).any(|pixel| pixel == preview_color));
        assert_eq!(
            &first[..4],
            &standard_constants::gif::style::CANVAS_BACKGROUND_COLOR
        );
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
        let second = decoder.read_next_frame().unwrap().unwrap();
        // 调色板须跨帧复用，后续帧以透明差分编码，同时保持播放时长。
        assert!(second.transparent.is_some());
        assert_eq!(second.delay, 10);
        assert!(u32::from(second.width) * u32::from(second.height) < (width * height) as u32);
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
