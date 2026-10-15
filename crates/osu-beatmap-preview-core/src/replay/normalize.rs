//! 对照官方 LegacyScoreDecoder 修复历史回放的引导帧和倒退时间。
use super::ReplayFrame;

pub(crate) fn frames(mut frames: Vec<ReplayFrame>) -> Vec<ReplayFrame> {
    if frames.len() >= 2 && frames[1].time_ms < frames[0].time_ms {
        frames[1].time_ms = frames[0].time_ms;
        frames[0].time_ms = 0;
    }
    if frames.len() >= 3 && frames[0].time_ms > frames[2].time_ms {
        frames[0].time_ms = frames[2].time_ms;
        frames[1].time_ms = frames[2].time_ms;
    }
    // stable 的跳过开场标记没有玩家输入含义；只移除最初两个位置的标记。
    if frames.get(1).is_some_and(|f| f.cursor == [256.0, -500.0]) {
        frames.remove(1);
    }
    if frames.first().is_some_and(|f| f.cursor == [256.0, -500.0]) {
        frames.remove(0);
    }
    let mut previous = None;
    frames.retain(|frame| {
        if previous.is_some_and(|time| frame.time_ms < time) {
            return false;
        }
        previous = Some(frame.time_ms);
        true
    });
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_start_markers_and_discards_backwards_frames_without_sorting() {
        let frames = [0, -100, 50, 40, 60, 60]
            .into_iter()
            .enumerate()
            .map(|(sequence, time_ms)| ReplayFrame {
                time_ms,
                delta_ms: 0,
                sequence,
                keys: sequence as u32,
                cursor: if sequence < 2 {
                    [256.0, -500.0]
                } else {
                    [0.0; 2]
                },
            })
            .collect();
        let frames = super::frames(frames);
        assert_eq!(
            frames.iter().map(|f| f.time_ms).collect::<Vec<_>>(),
            [50, 60, 60]
        );
        assert_eq!(
            frames.iter().map(|f| f.sequence).collect::<Vec<_>>(),
            [2, 4, 5]
        );
    }
}
