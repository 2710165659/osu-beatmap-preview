//! NC（Nightcore）的节拍鼓点：按 osu! 的 `ModNightcore.NightcoreBeatContainer` 生成事件。
//!
//! 每半拍（`Divisor = 2`）算一次拍号 `beatIndex`（每条红线从 0 重计，`OmitFirstBarLine`
//! 再减 1），每 `meter × 2 × 4` 个半拍为一段：段内按拍号放 kick / clap / hat、段首叠
//! finish；`SliderTickRate` 非偶数时不放 hat（拍长非 2 的倍数没有规律的反拍）。DC 无鼓点。
//!
//! 离线按网格精确时刻落点（游戏逐帧 `Update` 最多晚一帧）；`firstBeat` 恒为段长整数倍，
//! 从谱面起点连续生成与按段对齐的相位一致。

use super::sample::SampleLibrary;
use super::timeline::{HitsoundTimeline, PlayEvent, PlayFrequency};
use crate::domain::models::{Beatmap, TimingPoint};

/// 随二进制分发、需宿主预解码的 NC 鼓点样本名。与游戏 `ModNightcore` 的
/// `SampleInfo("Gameplay/nightcore-*")` 一致（去掉皮肤查找用的 `Gameplay/` 前缀）。
pub const NIGHTCORE_SAMPLE_NAMES: [&str; 4] = [
    "nightcore-clap",
    "nightcore-finish",
    "nightcore-hat",
    "nightcore-kick",
];

const SAMPLE_HAT: &str = "nightcore-hat";
const SAMPLE_CLAP: &str = "nightcore-clap";
const SAMPLE_KICK: &str = "nightcore-kick";
const SAMPLE_FINISH: &str = "nightcore-finish";

/// 游戏 `NightcoreBeatContainer` 的 `Divisor`：每拍触发 2 次（八分音符网格）。
const DIVISOR: u32 = 2;
/// 游戏里的 `bars_per_segment`：每 4 小节重置一次分段。
const BARS_PER_SEGMENT: u32 = 4;
/// 事件数量上限：损坏谱面（极小拍长）不能生成无限事件。
const MAX_EVENTS: usize = 100_000;

/// 生成 NC 的节拍鼓点事件。
///
/// `end_ms` 是要覆盖的结尾（谱面绝对毫秒，不含），避免为歌曲之后的时间生成事件；
/// 返回的事件已按开始时间升序，样本缺失时跳过（与打击音一致按静音处理）。
pub fn nightcore_events(
    beatmap: &Beatmap,
    library: &SampleLibrary,
    end_ms: f64,
) -> HitsoundTimeline {
    let red_lines: Vec<&TimingPoint> = beatmap
        .timing_points
        .iter()
        .filter(|point| point.uninherited)
        .collect();
    if red_lines.is_empty() {
        return HitsoundTimeline::default();
    }
    let end_ms = if end_ms.is_finite() {
        end_ms
    } else {
        f64::INFINITY
    };

    let samples = NightcoreSamples {
        hat: library.id_of(SAMPLE_HAT),
        clap: library.id_of(SAMPLE_CLAP),
        kick: library.id_of(SAMPLE_KICK),
        finish: library.id_of(SAMPLE_FINISH),
    };
    // `Precision.AlmostEquals(..., 0)` 用的是双精度默认容差 1e-7。
    let play_hats = (beatmap.difficulty.get_f64_or("SliderTickRate", 1.0) % 2.0).abs() <= 1e-7;

    let mut events = Vec::new();
    for (index, point) in red_lines.iter().enumerate() {
        if events.len() >= MAX_EVENTS {
            break;
        }
        let beat_length = point.beat_length;
        if !beat_length.is_finite() || beat_length <= 0.0 {
            continue;
        }
        let end_time = red_lines
            .get(index + 1)
            .map_or(end_ms, |next| next.time.min(end_ms));
        // 只有严格大于区段起点才继续：NaN 时间（损坏谱面）不能让循环停不下来。
        if end_time
            .partial_cmp(&point.time)
            .is_none_or(|order| order != std::cmp::Ordering::Greater)
        {
            continue;
        }

        let meter = point.meter.max(1) as u64;
        let segment_length = meter * DIVISOR as u64 * BARS_PER_SEGMENT as u64;
        let step = beat_length / DIVISOR as f64;
        // 每条红线都按自己的起点从 beatIndex 0 重新计数（与游戏一致：拍号/相位随红线重置）。
        let shift = i64::from(point.omit_first_bar_line);

        let mut beat = 0_i64;
        loop {
            let time = point.time + beat as f64 * step;
            if time >= end_time || events.len() >= MAX_EVENTS {
                break;
            }
            let beat_index = beat - shift;
            if beat_index >= 0 {
                let beat_index = beat_index as u64;
                let beat_in_segment = beat_index % segment_length;
                let time = round_grid_time(time);
                // 段首的 finish：`OmitFirstBarLine` 时首段的第 0 拍被省略。
                if beat_in_segment == 0 && (beat_index > 0 || shift == 0) {
                    push_event(&mut events, samples.finish, time);
                }
                let pattern = match meter {
                    // 3/4：每 3 拍一次 kick，clap 落在第 2 拍的反拍上。
                    3 => match beat_in_segment % 6 {
                        0 => samples.kick,
                        3 => samples.clap,
                        _ => play_hats.then_some(samples.hat).flatten(),
                    },
                    // 4/4：kick 在 1、3 拍，clap 在 2、4 拍。
                    4 => match beat_in_segment % 4 {
                        0 => samples.kick,
                        2 => samples.clap,
                        _ => play_hats.then_some(samples.hat).flatten(),
                    },
                    // 其他拍号游戏里的 switch 没有 default：只保留段首的 finish。
                    _ => None,
                };
                if let Some(source) = pattern {
                    push_event(&mut events, Some(source), time);
                }
            }
            beat += 1;
        }
    }

    // 区段之间可能因为浮点误差略有交错，统一稳定排序；同刻事件保持生成顺序。
    events.sort_by(|a, b| {
        a.start_ms
            .partial_cmp(&b.start_ms)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    HitsoundTimeline { events }
}

/// 已解析出的四类鼓点样本 id；缺失的按 `None`（事件跳过）。
#[derive(Clone, Copy)]
struct NightcoreSamples {
    hat: Option<usize>,
    clap: Option<usize>,
    kick: Option<usize>,
    finish: Option<usize>,
}

fn push_event(events: &mut Vec<PlayEvent>, source_id: Option<usize>, start_ms: f64) {
    let Some(source_id) = source_id else {
        return;
    };
    events.push(PlayEvent {
        start_ms,
        duration_ms: 0.0,
        source_id,
        // 游戏里按皮肤音量 100% 播放、谱面音效音量不参与，因此固定 1.0。
        gain: 1.0,
        looping: false,
        // 音高来自整条混音流的倍速重采样，事件本身不做音高调制。
        frequency: PlayFrequency::UNITY,
    });
}

/// 网格时刻取整：整数毫秒的拍点不要因为浮点累加变成 `999.9999999`。
fn round_grid_time(time: f64) -> f64 {
    let rounded = time.round();
    if (time - rounded).abs() <= 1e-7 {
        rounded
    } else {
        time
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{HitObjects, TimingPoint};
    use crate::hitsound::test_support::{beatmap_with, library_with};
    use crate::hitsound::SampleLibrary;

    /// 构造一条红线的谱面（500ms 拍长 = 120BPM，半拍 250ms）。
    fn beatmap(
        beat_length: f64,
        meter: i32,
        omit_first_bar_line: bool,
        tick_rate: &str,
    ) -> Beatmap {
        let mut beatmap = beatmap_with(0, HitObjects::Standard(Vec::new()));
        beatmap.timing_points = vec![TimingPoint {
            time: 0.0,
            beat_length,
            meter,
            uninherited: true,
            kiai_mode: false,
            omit_first_bar_line,
            sample_set: 0,
            sample_index: 0,
            sample_volume: 100,
        }];
        beatmap
            .difficulty
            .insert("SliderTickRate", tick_rate.to_string());
        beatmap
    }

    fn full_library() -> SampleLibrary {
        library_with(&NIGHTCORE_SAMPLE_NAMES)
    }

    /// 测试统一覆盖到 60 秒，避免最后一条红线一直生成到事件上限。
    const END_MS: f64 = 60_000.0;

    /// 取某个样本名的事件时间（毫秒）。
    fn times(beatmap: &Beatmap, library: &SampleLibrary, name: &str) -> Vec<f64> {
        let id = library.id_of(name).expect("样本必须在库里");
        nightcore_events(beatmap, library, END_MS)
            .events
            .iter()
            .filter(|event| event.source_id == id)
            .map(|event| event.start_ms)
            .collect()
    }

    /// 4/4 加 `SliderTickRate = 2`：kick 在 1、3 拍，clap 在 2、4 拍，hat 在反拍。
    #[test]
    fn four_four_pattern_matches_the_game() {
        let beatmap = beatmap(500.0, 4, false, "2");
        let library = full_library();
        // 一小节 2000ms：半拍 250ms → [0, 250, 500, 750, 1000, 1250, 1500, 1750]。
        assert_eq!(
            &times(&beatmap, &library, SAMPLE_KICK)[..4],
            &[0.0, 1000.0, 2000.0, 3000.0]
        );
        assert_eq!(
            &times(&beatmap, &library, SAMPLE_CLAP)[..4],
            &[500.0, 1500.0, 2500.0, 3500.0]
        );
        assert_eq!(
            &times(&beatmap, &library, SAMPLE_HAT)[..4],
            &[250.0, 750.0, 1250.0, 1750.0]
        );
        // 4 小节 = 8000ms 一段：finish 只在段首（含第 0 拍）。
        let finish = times(&beatmap, &library, SAMPLE_FINISH);
        assert_eq!(&finish[..3], &[0.0, 8000.0, 16000.0]);
    }

    /// 3/4：kick 每 3 拍一次，clap 落在第 2 拍的反拍，段长 = 3*2*4 个半拍 = 12 拍。
    #[test]
    fn three_four_pattern_matches_the_game() {
        let beatmap = beatmap(500.0, 3, false, "2");
        let library = full_library();
        assert_eq!(
            &times(&beatmap, &library, SAMPLE_KICK)[..3],
            &[0.0, 1500.0, 3000.0]
        );
        assert_eq!(
            &times(&beatmap, &library, SAMPLE_CLAP)[..3],
            &[750.0, 2250.0, 3750.0]
        );
        // 段长 12 拍 = 6000ms。
        let finish = times(&beatmap, &library, SAMPLE_FINISH);
        assert_eq!(&finish[..2], &[0.0, 6000.0]);
    }

    /// 拍长不是 2 的倍数（默认 SliderTickRate = 1）时不放 hat。
    #[test]
    fn hats_require_an_even_slider_tick_rate() {
        let library = full_library();
        let odd = beatmap(500.0, 4, false, "1");
        assert!(times(&odd, &library, SAMPLE_HAT).is_empty());
        // 非偶数但非 1（3、2.5）同样不放。
        for tick_rate in ["3", "2.5", "0.5"] {
            let beatmap = beatmap(500.0, 4, false, tick_rate);
            assert!(
                times(&beatmap, &library, SAMPLE_HAT).is_empty(),
                "tickRate={tick_rate} 不该放 hat"
            );
        }
        // 2 与 4 都算偶数。
        for tick_rate in ["2", "4"] {
            let beatmap = beatmap(500.0, 4, false, tick_rate);
            assert!(
                !times(&beatmap, &library, SAMPLE_HAT).is_empty(),
                "{tick_rate}"
            );
        }
    }

    /// `OmitFirstBarLine` 让整条网格后移半个半拍（游戏里 beatIndex 减 1）。
    #[test]
    fn omit_first_bar_line_shifts_the_grid() {
        let beatmap = beatmap(500.0, 4, true, "2");
        let library = full_library();
        // 第 0 拍被省略：kick 从 250ms 开始，段首也没有 finish。
        assert_eq!(
            &times(&beatmap, &library, SAMPLE_KICK)[..3],
            &[250.0, 1250.0, 2250.0]
        );
        // 段长 4 小节 = 8000ms，位移后段首在 250 + 8000 的整数倍。
        assert_eq!(
            &times(&beatmap, &library, SAMPLE_FINISH)[..2],
            &[8250.0, 16250.0]
        );
    }

    /// 其他拍号只保留段首 finish，与游戏 switch 无 default 的行为一致。
    #[test]
    fn unsupported_meters_only_play_the_segment_finish() {
        for meter in [1, 2, 5, 7] {
            let beatmap = beatmap(500.0, meter, false, "2");
            let library = full_library();
            assert!(
                times(&beatmap, &library, SAMPLE_KICK).is_empty(),
                "meter={meter}"
            );
            assert!(
                times(&beatmap, &library, SAMPLE_CLAP).is_empty(),
                "meter={meter}"
            );
            assert!(
                times(&beatmap, &library, SAMPLE_HAT).is_empty(),
                "meter={meter}"
            );
            assert_eq!(
                times(&beatmap, &library, SAMPLE_FINISH)[0],
                0.0,
                "meter={meter}"
            );
        }
    }

    /// 每条红线都按自己的起点重新计数：BPM 或拍号变化都会重启网格与分段相位。
    #[test]
    fn each_red_line_restarts_the_grid() {
        let mut beatmap = beatmap_with(0, HitObjects::Standard(Vec::new()));
        beatmap.difficulty.insert("SliderTickRate", "2".to_string());
        beatmap.timing_points = vec![
            TimingPoint {
                time: 0.0,
                beat_length: 500.0,
                meter: 4,
                uninherited: true,
                kiai_mode: false,
                omit_first_bar_line: false,
                sample_set: 0,
                sample_index: 0,
                sample_volume: 100,
            },
            TimingPoint {
                time: 4000.0,
                beat_length: 250.0,
                meter: 4,
                uninherited: true,
                kiai_mode: false,
                omit_first_bar_line: false,
                sample_set: 0,
                sample_index: 0,
                sample_volume: 100,
            },
        ];
        let library = full_library();
        let finish = times(&beatmap, &library, SAMPLE_FINISH);
        // 第一段：0 起、8 秒一段；第二段在 4000ms 重启，因此 4000 也是段首。
        assert!(finish.contains(&0.0));
        assert!(finish.contains(&4000.0), "{finish:?}");
        // 第二段的最短间隔是新的半拍（125ms）。
        let after: Vec<f64> = finish
            .iter()
            .copied()
            .filter(|time| *time >= 4000.0)
            .collect();
        assert!(after.len() >= 2);
        assert!(
            after
                .windows(2)
                .all(|pair| (pair[1] - pair[0]) % 125.0 == 0.0),
            "{after:?}"
        );
    }

    /// 第一条红线之前不生成事件（游戏的 `firstBeat = 0`）。
    #[test]
    fn nothing_is_generated_before_the_first_red_line() {
        let mut beatmap = beatmap_with(0, HitObjects::Standard(Vec::new()));
        beatmap.difficulty.insert("SliderTickRate", "2".to_string());
        beatmap.timing_points[0].time = 1000.0;
        let library = full_library();
        let events = nightcore_events(&beatmap, &library, END_MS);
        assert!(
            events.events.iter().all(|event| event.start_ms >= 1000.0),
            "{:?}",
            events.events
        );
        assert_eq!(events.events[0].start_ms, 1000.0);
    }

    /// 没有红线、拍长非法或样本缺失时按静音处理，不 panic。
    #[test]
    fn missing_inputs_produce_silence() {
        let library = full_library();
        let mut no_red_lines = beatmap_with(0, HitObjects::Standard(Vec::new()));
        no_red_lines.timing_points.clear();
        assert!(nightcore_events(&no_red_lines, &library, END_MS).is_empty());

        let mut broken = beatmap(0.0, 4, false, "2");
        assert!(nightcore_events(&broken, &library, END_MS).is_empty());
        broken.timing_points[0].beat_length = f64::NAN;
        assert!(nightcore_events(&broken, &library, END_MS).is_empty());

        // 空样本库：全部事件都被丢弃，不产生引用不存在样本的事件。
        let empty = SampleLibrary::new();
        let beatmap = beatmap(500.0, 4, false, "2");
        assert!(nightcore_events(&beatmap, &empty, END_MS).is_empty());

        // 只有 kick 时其余三类事件不存在。
        let only_kick = library_with(&[SAMPLE_KICK]);
        let events = nightcore_events(&beatmap, &only_kick, END_MS);
        assert!(!events.is_empty());
        assert!(events
            .events
            .iter()
            .all(|event| only_kick.name_of(event.source_id) == Some(SAMPLE_KICK)));
    }

    /// 事件按时间升序且互不重复（混音器的事件游标依赖有序）。
    #[test]
    fn events_are_sorted() {
        let beatmap = beatmap(500.0, 4, false, "2");
        let events = nightcore_events(&beatmap, &full_library(), END_MS);
        assert!(events.len() > 100);
        assert!(
            events
                .events
                .windows(2)
                .all(|pair| pair[0].start_ms <= pair[1].start_ms),
            "事件未排序"
        );
    }

    /// `end_ms` 之外不再生成事件（离线导出只覆盖本次窗口）。
    #[test]
    fn generation_stops_at_the_requested_end() {
        let beatmap = beatmap(500.0, 4, false, "2");
        let events = nightcore_events(&beatmap, &full_library(), 3_000.0);
        assert!(!events.is_empty());
        assert!(
            events.events.iter().all(|event| event.start_ms < 3_000.0),
            "结尾之外仍有事件"
        );
        // 没有结尾信息时用事件上限兜底，不能死循环。
        let unbounded = nightcore_events(&beatmap, &full_library(), f64::INFINITY);
        assert_eq!(unbounded.len(), MAX_EVENTS);
    }
}
