//! 回放导出的采样计划与帧数限制独立于规则时间线。
use crate::application::output_plan::RenderPlan;
use osu_beatmap_preview_core::{processing::validation::TimePoint, Beatmap, PreviewError, Result};

const MAX_FRAMES: usize = 1_000_000;
pub(crate) struct TimePlan {
    pub fps: u32,
    pub start: i64,
    pub times: Vec<i64>,
}

pub(crate) fn build(
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
        if plan.target_mode == 1 {
            config.render.taiko.mp4.style.FPS as u32
        } else {
            config.render.standard.mp4.style.FPS as u32
        },
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
    mp4_fps: u32,
    padding: i64,
) -> Result<TimePlan> {
    let invalid =
        || PreviewError::render("InvalidReplayTimePlan: invalid range or frame limit exceeded");
    let speed = plan.mods.as_ref().map_or(1.0, |mods| mods.speed_multiplier);
    if !speed.is_finite() || speed <= 0.0 {
        return Err(invalid());
    }
    let fps = plan.fps.unwrap_or(mp4_fps);
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
        vec![full_start]
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
    if starts.len() != 1 {
        return Err(invalid());
    }
    let mut ranges = Vec::new();
    let mut count = 0_usize;
    for start in starts {
        let duration = match plan.duration_seconds {
            Some(seconds) => seconds_ms(seconds)?,
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

pub(crate) fn checked_gif_frame_count(
    duration_ms: f64,
    fps: f64,
    speed: f64,
    cells: usize,
) -> Result<usize> {
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
    use crate::application::output_plan::OutputFormat;

    fn plan(points: Vec<TimePoint>) -> RenderPlan {
        RenderPlan {
            selected_times: None,
            bid: "1".into(),
            target_mode: 0,
            convert_used: false,
            mods: None,
            time_points: points,
            duration_seconds: Some(1.0),
            format: OutputFormat::Mp4,
            fps: Some(30),
            requested_scale: None,
            no_cache: true,
        }
    }
    #[test]
    fn output_duration_is_independent_of_replay_speed_and_checks_limits() {
        for speed in [0.75, 1.0, 1.5] {
            assert_eq!(checked_gif_frame_count(5000.0, 15.0, speed, 4).unwrap(), 75);
        }
        for (duration, fps, speed, cells) in [
            (f64::NAN, 15.0, 1.0, 4),
            (0.0, 15.0, 1.0, 4),
            (5000.0, 0.0, 1.0, 4),
            (5000.0, 15.0, f64::INFINITY, 4),
            (5000.0, 15.0, 1.0, usize::MAX),
            (5000.0, 15.0, 1.0, 0),
        ] {
            assert!(checked_gif_frame_count(duration, fps, speed, cells).is_err());
        }
    }

    #[test]
    fn mp4_rejects_multiple_segments() {
        let points = vec![TimePoint::Seconds(3.0), TimePoint::Seconds(1.0)];
        assert!(build_with(&plan(points), 1000, 6000, 0, None, 60, 1000).is_err());
    }
    #[test]
    fn checks_limits_and_includes_settlement_in_complete_video() {
        let mut plan = plan(vec![]);
        plan.duration_seconds = None;
        let result = build_with(&plan, 1000, 1140, 0, None, 60, 1000).unwrap();
        assert_eq!(result.start, -1000);
        assert!(*result.times.last().unwrap() >= 1140);
        plan.fps = Some(0);
        assert!(build_with(&plan, 1000, 1140, 0, None, 60, 1000).is_err());
        plan.fps = Some(u32::MAX);
        assert!(build_with(&plan, 1000, 1140, 0, None, 60, 1000).is_err());
    }

    #[test]
    fn replay_rate_changes_output_duration_without_changing_chart_range() {
        for (code, count, step) in [("DT", 20, 50), ("HT", 40, 25)] {
            let mut plan = plan(vec![TimePoint::Seconds(0.0)]);
            plan.mods =
                Some(osu_beatmap_preview_core::model::mods::parse_mods(&[code.into()]).unwrap());
            let result = build_with(&plan, 1000, 3000, 0, None, 60, 1000).unwrap();
            assert_eq!(result.times.len(), count);
            assert_eq!(result.times[1] - result.times[0], step);
            assert_eq!(result.times[0], 1000);
            assert!(*result.times.last().unwrap() < 2000);
        }
    }
}
