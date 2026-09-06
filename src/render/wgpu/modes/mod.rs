//! 四模式 WGPU 实时场景和 MP4 帧源。

mod catch;
mod mania;
mod standard;
mod taiko;

use std::path::Path;

use crate::application::plan::RenderPlan;
use crate::domain::errors::{PreviewError, Result};
use crate::domain::models::Beatmap;
use crate::domain::parser::round_half_even;
use crate::domain::shared::time_selection::TimeAxis;
use crate::domain::timeout::RequestDeadline;
use crate::infrastructure::media::audio::AudioSourceJob;
use crate::render::canvas::Img;
use crate::render::geometry::GameMode;

pub(crate) use catch::prepare_realtime as prepare_catch;
pub(crate) use mania::prepare_realtime as prepare_mania;
pub(crate) use standard::prepare_realtime as prepare_standard;
pub(crate) use taiko::prepare_realtime as prepare_taiko;

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_video(
    beatmap: &Beatmap,
    plan: &RenderPlan,
    output_path: &Path,
    background: Option<Img>,
    audio_job: AudioSourceJob,
    time_axis: TimeAxis,
    deadline: &RequestDeadline,
) -> Result<()> {
    let (first, last) =
        crate::application::engine::legacy::object_time_bounds(&beatmap.hit_objects)
            .ok_or_else(|| PreviewError::render("beatmap has no hit objects"))?;
    let speed = plan
        .mods
        .as_ref()
        .map_or(1.0, |settings| settings.speed_multiplier);
    let range = crate::infrastructure::media::resolve_video_time_range(
        beatmap,
        first,
        last,
        plan.time_points.first().copied(),
        plan.duration_seconds,
        speed,
    )?;
    let mode = match plan.target_mode {
        0 => GameMode::Standard,
        1 => GameMode::Taiko,
        2 => GameMode::Catch,
        3 => GameMode::Mania,
        _ => return Err(PreviewError::render("unsupported WGPU ruleset")),
    };
    let fps = plan
        .fps
        .unwrap_or_else(|| crate::realtime::configured_offscreen(mode).target_fps);
    let frame_count = (((range.end - range.start) as f64 * fps as f64 / (1000.0 * speed)).round()
        as usize)
        .max(1);
    let source = match mode {
        GameMode::Standard => prepare_standard(beatmap, plan.mods.as_ref(), time_axis),
        GameMode::Taiko => prepare_taiko(beatmap, plan.mods.as_ref()),
        GameMode::Catch => prepare_catch(beatmap, plan.mods.as_ref()),
        GameMode::Mania => prepare_mania(beatmap, plan.mods.as_ref()),
    }?;
    let start = range.start;
    let render = move |index: usize| {
        let absolute_time_ms = start + round_half_even(index as f64 * 1000.0 * speed / fps as f64);
        source.render(absolute_time_ms)
    };
    crate::infrastructure::media::save_mp4_streamed_wgpu(
        frame_count,
        range.start,
        last,
        speed,
        render,
        output_path,
        fps,
        audio_job,
        background,
        time_axis,
        deadline,
        mode,
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::mods::ModSettings;
    use crate::render::scene::DrawCommand;
    use crate::render::wgpu::RealtimeFrameSource;

    fn fixture() -> Beatmap {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/testdata_conversion/260177.osu");
        crate::domain::parser::parse_beatmap(&path).unwrap()
    }

    fn times(beatmap: &Beatmap) -> [i64; 6] {
        let (first, last) =
            crate::application::engine::legacy::object_time_bounds(&beatmap.hit_objects).unwrap();
        [
            last + 1000,
            first - 1000,
            first,
            (first + last) / 2,
            last,
            first + 250,
        ]
    }

    fn assert_scenes(source: RealtimeFrameSource, beatmap: &Beatmap) {
        for time in times(beatmap) {
            let scene = source.render(time).unwrap();
            assert_eq!(scene.absolute_time_ms(), time);
            assert!(scene.width() > 0 && scene.height() > 0);
            assert!(!scene.commands.is_empty());
            let has_primitive = scene.commands.iter().any(|command| {
                matches!(
                    command,
                    DrawCommand::Rectangle { .. }
                        | DrawCommand::Circle { .. }
                        | DrawCommand::Ring { .. }
                        | DrawCommand::Line { .. }
                        | DrawCommand::SliderMesh { .. }
                )
            });
            let has_visible_resource = scene
                .resources
                .values()
                .any(|image| image.data.chunks_exact(4).any(|pixel| pixel[3] != 0));
            assert!(has_primitive || has_visible_resource);
        }
    }

    #[test]
    fn standard真实谱面支持mod负时间首尾与乱序seek() {
        let beatmap = fixture();
        let mut mods = ModSettings::new();
        mods.hidden = true;
        assert_scenes(
            prepare_standard(&beatmap, Some(&mods), TimeAxis::new(times(&beatmap)[2])).unwrap(),
            &beatmap,
        );
    }

    #[test]
    fn taiko转谱支持负时间首尾与乱序seek() {
        let source = fixture();
        let beatmap =
            crate::application::engine::legacy::convert_beatmap(&source, 1, None).unwrap();
        assert_scenes(prepare_taiko(&beatmap, None).unwrap(), &beatmap);
    }

    #[test]
    fn catch转谱支持负时间首尾与乱序seek() {
        let source = fixture();
        let beatmap =
            crate::application::engine::legacy::convert_beatmap(&source, 2, None).unwrap();
        assert_scenes(prepare_catch(&beatmap, None).unwrap(), &beatmap);
    }

    #[test]
    fn mania转谱支持mod负时间首尾与乱序seek() {
        let source = fixture();
        let mut mods = ModSettings::new();
        mods.mania_keys = Some(4);
        let beatmap =
            crate::application::engine::legacy::convert_beatmap(&source, 3, Some(&mods)).unwrap();
        assert_scenes(prepare_mania(&beatmap, Some(&mods)).unwrap(), &beatmap);
    }
}
