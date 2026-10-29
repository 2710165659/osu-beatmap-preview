//! Catch 回放 GIF 与 MP4；物件和最终 HUD 共用实时预览的绘制计划。
mod gif;
pub(crate) use gif::render_gif;

use crate::{
    application::output_plan::RenderPlan,
    export::{
        canvas::Img,
        replay::{font, time_plan},
    },
    media::{self, audio::AudioSourceJob, MediaBackground, MediaStoryboard},
};
use osu_beatmap_preview_core::{
    gameplay::timeline::GameplayTimeline, hitsound::MusicRate, processing::timeline::TimeAxis,
    render::shared::catch::ReplayRenderPlan, replay::preparation::catch::PreparedCatch,
    support::timeout::RequestDeadline, Result,
};
use std::{path::Path, sync::Arc};

#[allow(clippy::too_many_arguments)]
pub(crate) fn render(
    prepared: Arc<PreparedCatch>,
    gameplay: Arc<GameplayTimeline>,
    plan: &RenderPlan,
    path: &Path,
    background: MediaBackground,
    storyboard: Option<MediaStoryboard>,
    audio: AudioSourceJob,
    deadline: &RequestDeadline,
) -> Result<()> {
    let beatmap = &prepared.beatmap;
    let first = prepared.first;
    let last = prepared.last_end.max(gameplay.settlement_time).ceil() as i64;
    let schedule = time_plan::build(plan, beatmap, first, last)?;
    let renderer = ReplayRenderPlan::new(
        Arc::clone(&prepared),
        Arc::clone(&gameplay),
        font::load()?,
        None,
    )?;
    let mut blank = Img::new(
        renderer.layout.frame_width as u32,
        renderer.layout.frame_height as u32,
        [0; 4],
    );
    if background.needs_playfield_base(storyboard.as_ref()) {
        let rect = renderer.layout.content;
        blank.fill_rect_size(rect.x,rect.y,rect.width,rect.height,osu_beatmap_preview_core::render::cpu::modes::catch::constants::mp4::style::PLAYFIELD_BACKGROUND);
    }
    let draw = |index: usize| {
        let time = schedule.times[index];
        let mut image = blank.clone();
        renderer.draw_frame(&mut image, time);
        Ok((image, time))
    };
    let overlay = |image: &mut Img, time| renderer.draw_overlay(image, time);
    media::save_mp4_streamed(
        schedule.times.len(),
        schedule.start,
        last,
        prepared.mods.settings.speed_multiplier,
        MusicRate::output_domain(
            prepared.mods.settings.speed_multiplier,
            prepared.mods.music_pitch(),
        ),
        prepared.mods.settings.nightcore,
        draw,
        path,
        schedule.fps,
        audio,
        beatmap.clone(),
        background,
        storyboard,
        TimeAxis::new(first),
        deadline,
        crate::export::geometry::GameMode::Catch,
        media::FrameComposition::ReplayCanvas(&overlay),
    )
}
