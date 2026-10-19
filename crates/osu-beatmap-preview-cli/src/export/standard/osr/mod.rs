//! 回放媒体导出：并行帧与音频共享不可变规则结果。
mod font;
mod gif;
mod png;
mod time_plan;
use crate::{
    application::output_plan::RenderPlan,
    media::{self, audio::AudioSourceJob, MediaBackground, MediaStoryboard},
};
pub(crate) use gif::render_gif;
use osu_beatmap_preview_core::{
    gameplay::timeline::GameplayTimeline, replay::preparation::standard::PreparedStandard, Beatmap,
    Result,
};
use osu_beatmap_preview_core::{hitsound::MusicRate, processing::timeline::TimeAxis};
use osu_beatmap_preview_core::{
    render::cpu::modes::standard::{context::*, osr},
    support::timeout::RequestDeadline,
};
pub(crate) use png::render_png;
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
    let format = crate::export::geometry::OutputFormat::Mp4;
    let context = build_replay_context(beatmap, prepared, axis, format);
    let visuals = osr::ReplayVisuals::new(
        &context,
        gameplay,
        font::load()?,
        &beatmap.break_periods,
        prepared.od,
    );
    let last = prepared.last_end.max(gameplay.settlement_time).ceil() as i64;
    let schedule = time_plan::build(plan, beatmap, first, last)?;
    let fps = schedule.fps;
    let start = schedule.start;
    let times = schedule.times;
    let frames = times.len();
    let render_plan = osr::ReplayRenderPlan::new(&context, prepared, gameplay);
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
                &render_plan,
                blank.as_ref(),
            )
        })
    };
    let overlay = |image: &mut crate::export::canvas::Img, time| {
        osr::draw_overlay(image, &context, prepared, gameplay, &visuals, time);
    };
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
