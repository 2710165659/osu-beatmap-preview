//! 回放选段计划独立于规则时间线；计算后检查总帧数，再分配显示采样点。
use crate::application::output_plan::{OutputFormat, RenderPlan};
use osu_beatmap_preview_core::{processing::validation::TimePoint, Beatmap, PreviewError, Result};

const MAX_FRAMES: usize = 1_000_000;
pub(super) struct TimePlan {
    pub fps: u32,
    pub start: i64,
    pub times: Vec<i64>,
}

pub(super) fn build(
    plan: &RenderPlan,
    beatmap: &Beatmap,
    first: i64,
    last: i64,
) -> Result<TimePlan> {
    let config = crate::config::current();
    build_with(
        plan,
        first,
        last,
        beatmap.audio_lead_in_ms(),
        beatmap.general.get_f64("PreviewTime"),
        config.render.standard.gif.style.FPS as u32,
        config.render.standard.mp4.style.FPS as u32,
        config.render.standard.gif.style.DURATION_MS,
        config.advance.video.VIDEO_END_PADDING_MS,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_with(
    plan: &RenderPlan,
    first: i64,
    last: i64,
    lead_in: i64,
    preview: Option<f64>,
    gif_fps: u32,
    mp4_fps: u32,
    gif_duration: i64,
    padding: i64,
) -> Result<TimePlan> {
    let invalid =
        || PreviewError::render("InvalidReplayTimePlan: invalid range or frame limit exceeded");
    let speed = plan.mods.as_ref().map_or(1.0, |mods| mods.speed_multiplier);
    if !speed.is_finite() || speed <= 0.0 {
        return Err(invalid());
    }
    let fps = plan.fps.unwrap_or(if plan.format == OutputFormat::Gif {
        gif_fps
    } else {
        mp4_fps
    });
    if fps == 0 {
        return Err(invalid());
    }
    let full_start = first.checked_sub(lead_in.max(2000)).ok_or_else(invalid)?;
    let full_end = last.checked_add(padding).ok_or_else(invalid)?;
    let seconds_ms = |seconds: f64| -> Result<i64> {
        let value = seconds * 1000.0;
        if !value.is_finite() || value.abs() > 86_400_000.0 {
            return Err(invalid());
        }
        Ok(value.round_ties_even() as i64)
    };
    let starts = if plan.time_points.is_empty() {
        vec![if plan.format == OutputFormat::Gif {
            preview
                .filter(|t| t.is_finite() && *t >= 0.0)
                .map_or(first, |t| t.round_ties_even() as i64)
        } else {
            full_start
        }]
    } else {
        plan.time_points
            .iter()
            .map(|point| match point {
                TimePoint::Seconds(seconds) => {
                    first.checked_add(seconds_ms(*seconds)?).ok_or_else(invalid)
                }
                TimePoint::Preview => Ok(preview
                    .filter(|t| t.is_finite() && *t >= 0.0)
                    .map_or(first, |t| t.round_ties_even() as i64)),
            })
            .collect::<Result<Vec<_>>>()?
    };
    if plan.format == OutputFormat::Mp4 && starts.len() != 1 {
        return Err(invalid());
    }
    let mut ranges = Vec::new();
    let mut count = 0_usize;
    for start in starts {
        let duration = match plan.duration_seconds {
            Some(seconds) => seconds_ms(seconds)?,
            None if plan.format == OutputFormat::Gif => gif_duration,
            None => full_end.checked_sub(start).ok_or_else(invalid)?,
        };
        let end = start
            .checked_add(duration)
            .ok_or_else(invalid)?
            .min(full_end);
        if start < full_start || duration <= 0 || end <= start {
            return Err(invalid());
        }
        let frames = ((end - start) as f64 * f64::from(fps) / (1000.0 * speed)).ceil();
        if !frames.is_finite() || frames > MAX_FRAMES as f64 {
            return Err(invalid());
        }
        let frames = frames as usize;
        count = count
            .checked_add(frames)
            .filter(|count| *count <= MAX_FRAMES)
            .ok_or_else(invalid)?;
        ranges.push((start, frames));
    }
    let mut times = Vec::new();
    times.try_reserve_exact(count).map_err(|_| invalid())?;
    for (start, frames) in &ranges {
        for index in 0..*frames {
            times.push(crate::media::frame_time_ms(*start, index, speed, fps));
        }
    }
    Ok(TimePlan {
        fps,
        start: ranges[0].0,
        times,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan(format: OutputFormat, points: Vec<TimePoint>) -> RenderPlan {
        RenderPlan {
            bid: "1".into(),
            target_mode: 0,
            convert_used: false,
            mods: None,
            time_points: points,
            duration_seconds: Some(1.0),
            format,
            fps: Some(30),
            requested_scale: None,
            no_cache: true,
        }
    }
    #[test]
    fn gif_preserves_all_requested_segments_and_mp4_rejects_multiple() {
        let points = vec![TimePoint::Seconds(3.0), TimePoint::Seconds(1.0)];
        let result = build_with(
            &plan(OutputFormat::Gif, points.clone()),
            1000,
            6000,
            0,
            None,
            15,
            60,
            5000,
            1000,
        )
        .unwrap();
        assert_eq!(result.times.len(), 60);
        assert_eq!((result.times[0], result.times[30]), (4000, 2000));
        assert!(build_with(
            &plan(OutputFormat::Mp4, points),
            1000,
            6000,
            0,
            None,
            15,
            60,
            5000,
            1000
        )
        .is_err());
    }
    #[test]
    fn checks_limits_and_includes_settlement_in_complete_video() {
        let mut plan = plan(OutputFormat::Mp4, vec![]);
        plan.duration_seconds = None;
        let result = build_with(&plan, 1000, 1140, 0, None, 15, 60, 5000, 1000).unwrap();
        assert_eq!(result.start, -1000);
        assert!(*result.times.last().unwrap() >= 1140);
        plan.fps = Some(0);
        assert!(build_with(&plan, 1000, 1140, 0, None, 15, 60, 5000, 1000).is_err());
        plan.fps = Some(u32::MAX);
        assert!(build_with(&plan, 1000, 1140, 0, None, 15, 60, 5000, 1000).is_err());
    }

    #[test]
    fn replay_rate_changes_output_duration_without_changing_chart_range() {
        for (code, count, step) in [("DT", 20, 50), ("HT", 40, 25)] {
            let mut plan = plan(OutputFormat::Mp4, vec![TimePoint::Seconds(0.0)]);
            plan.mods =
                Some(osu_beatmap_preview_core::model::mods::parse_mods(&[code.into()]).unwrap());
            let result = build_with(&plan, 1000, 3000, 0, None, 15, 60, 5000, 1000).unwrap();
            assert_eq!(result.times.len(), count);
            assert_eq!(result.times[1] - result.times[0], step);
            assert_eq!(result.times[0], 1000);
            assert!(*result.times.last().unwrap() < 2000);
        }
    }
}
