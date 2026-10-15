//! 回放媒体导出：并行帧与音频共享不可变规则结果。
mod font;
mod gif;
mod time_plan;
use crate::{
    application::plan::{OutputFormat, RenderPlan},
    media::{self, audio::AudioSourceJob, MediaBackground, MediaStoryboard},
};
use osu_beatmap_preview_core::{
    gameplay::timeline::GameplayTimeline, preparation::standard::PreparedStandard, Beatmap, Result,
};
use osu_beatmap_preview_core::{hitsound::MusicRate, processing::timeline::TimeAxis};
use osu_beatmap_preview_core::{
    render::cpu::modes::standard::{context::*, osr},
    support::timeout::RequestDeadline,
};
use std::{cell::RefCell, path::Path};

#[allow(clippy::too_many_arguments)]
pub(crate) fn render(
    beatmap: &Beatmap,
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    plan: &RenderPlan,
    path: &Path,
    background: MediaBackground,
    storyboard: Option<MediaStoryboard>,
    audio: AudioSourceJob,
    deadline: &RequestDeadline,
) -> Result<()> {
    let first = prepared.objects[0].source.start_time;
    let axis = TimeAxis::new(first);
    let format = if plan.format == OutputFormat::Gif {
        crate::export::geometry::OutputFormat::Gif
    } else {
        crate::export::geometry::OutputFormat::Mp4
    };
    let context = build_replay_context(beatmap, prepared, axis, format);
    let visuals =
        osr::ReplayVisuals::new(&context, gameplay, font::load()?, &beatmap.break_periods);
    let last = prepared.last_end.max(gameplay.settlement_time).ceil() as i64;
    let schedule = time_plan::build(plan, beatmap, first, last)?;
    let fps = schedule.fps;
    let start = schedule.start;
    let times = schedule.times;
    let frames = times.len();
    // 回放晚打可延长圆圈寿命，不能沿用只覆盖 Autoplay 消退的可见索引。
    let indexes: Vec<Vec<_>> = times
        .iter()
        .map(|&time| {
            prepared
                .objects
                .iter()
                .enumerate()
                .filter(|(id, object)| {
                    let end = gameplay.objects[*id]
                        .end
                        .map_or(object.end + 400.0, |e| e.0.max(object.end))
                        + 800.0;
                    time >= object.source.start_time - context.settings.preempt_ms
                        && time as f64 <= end
                })
                .map(|(id, _)| id)
                .collect()
        })
        .collect();
    let blank = (!background.needs_playfield_base(storyboard.as_ref())).then(|| {
        crate::export::canvas::Img::new(
            context.frame_layout.frame_width as u32,
            context.frame_layout.frame_height as u32,
            [0; 4],
        )
    });
    thread_local! { static CACHE: RefCell<RenderCache> = RefCell::new(RenderCache::default()); }
    let draw = |index: usize| {
        CACHE.with(|cache| {
            osr::render_frame(
                &context,
                &mut cache.borrow_mut(),
                prepared,
                gameplay,
                times[index],
                &beatmap.break_periods,
                &indexes[index],
                blank.as_ref(),
            )
        })
    };
    let overlay = |image: &mut crate::export::canvas::Img, time| {
        osr::draw_overlay(image, &context, prepared, gameplay, &visuals, time);
    };
    if plan.format == OutputFormat::Gif {
        let source = audio.wait()?;
        crate::media::audio::validate_source_audio(&source, deadline)?;
        gif::save(
            &times,
            draw,
            overlay,
            context.frame_layout.frame_width as u32,
            context.frame_layout.frame_height as u32,
            background,
            storyboard.as_ref(),
            path,
            fps,
            deadline,
        )
    } else {
        media::save_mp4_streamed(
            frames,
            start,
            last,
            prepared.mods.settings.speed_multiplier,
            MusicRate::output_domain(
                prepared.mods.settings.speed_multiplier,
                prepared.mods.music_pitch(),
            ),
            prepared.mods.settings.nightcore,
            |index| Ok((draw(index), times[index])),
            path,
            fps,
            audio,
            beatmap.clone(),
            background,
            storyboard,
            axis,
            deadline,
            crate::export::geometry::GameMode::Standard,
            media::FrameComposition::ReplayCanvas(&overlay),
        )
    }
}
