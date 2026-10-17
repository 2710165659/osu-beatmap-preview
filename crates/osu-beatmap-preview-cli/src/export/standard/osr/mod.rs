//! 回放媒体导出：并行帧与音频共享不可变规则结果。
mod font;
mod gif;
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
    // 回放晚打可延长圆圈寿命，不能沿用只覆盖 Autoplay 消退的可见索引。
    let indexes: Vec<Vec<_>> = times
        .iter()
        .map(|&time| visible_indexes(prepared, gameplay, context.settings.preempt_ms, time))
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

fn visible_indexes(
    prepared: &PreparedStandard,
    gameplay: &GameplayTimeline,
    preempt_ms: i64,
    time: i64,
) -> Vec<usize> {
    // 回放晚打可延长物件寿命，GIF 和 MP4 必须使用同一可见性边界。
    prepared
        .objects
        .iter()
        .enumerate()
        .filter(|(id, object)| {
            let end = gameplay.objects[*id]
                .end
                .map_or(object.end + 400.0, |e| e.0.max(object.end))
                + 800.0;
            time >= object.source.start_time - preempt_ms && time as f64 <= end
        })
        .map(|(id, _)| id)
        .collect()
}
