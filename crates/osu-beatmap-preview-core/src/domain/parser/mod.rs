//! .osu 谱面文件解析器。
//!
//! 子模块：
//! - `sections`：区段拆分、键值解析、连击颜色
//! - `timing`：timing points、break 时段与背景/视频事件
//! - `hit_objects`：按模式解析音符、滑条时长与舍入

mod hit_objects;
mod sections;
mod timing;

pub use hit_objects::{resolve_slider_timing, round_half_even};
pub use sections::{
    default_metadata, parse_combo_colors, parse_format_version, parse_key_value, split_sections,
};
pub use timing::{
    parse_background_filename, parse_break_periods, parse_timing_points, parse_video_event,
};

use crate::domain::errors::{PreviewError, Result};
use crate::domain::media::is_video_entry;
use crate::domain::models::*;
/// 从宿主提供的字节解析谱面；浏览器、移动端和桌面端都通过这个入口复用解析逻辑。
pub fn parse_beatmap_bytes(bytes: &[u8]) -> Result<Beatmap> {
    let content = String::from_utf8_lossy(bytes);
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    parse_beatmap_str(content).ok_or_else(|| PreviewError::parse("Failed to parse beatmap."))
}

fn parse_beatmap_str(content: &str) -> Option<Beatmap> {
    let sections = split_sections(content);

    let metadata = match sections.get("Metadata") {
        Some(lines) => parse_key_value(lines),
        None => default_metadata(),
    };
    let difficulty = parse_key_value(sections.get("Difficulty")?);
    let mut general = match sections.get("General") {
        Some(lines) => parse_key_value(lines),
        None => {
            let mut kv = KvSection::default();
            kv.insert("Mode", "0".to_string());
            kv
        }
    };
    general.insert("FormatVersion", parse_format_version(content).to_string());
    let timing_points = parse_timing_points(sections.get("TimingPoints")?)?;
    let break_periods = parse_break_periods(sections.get("Events"));
    let video_event = parse_video_event(sections.get("Events"));
    // osu! 兼容：老谱面会把背景图写成 `Video,` 行（扩展名不是视频扩展名），
    // 这种行按背景图处理；已有正经背景事件时不覆盖它。
    let background_filename = parse_background_filename(sections.get("Events")).or_else(|| {
        video_event
            .as_ref()
            .filter(|event| !is_video_entry(&event.filename))
            .map(|event| event.filename.clone())
    });
    // 扩展名不在白名单内的 `Video` 行不算背景视频（同 osu! 的 storyboard 解码规则）。
    let video = video_event.filter(|event| is_video_entry(&event.filename));
    let mode: i32 = general.get("Mode").unwrap_or("0").parse().ok()?;

    let combo_colors = parse_combo_colors(sections.get("Colours"));

    let editor = sections
        .get("Editor")
        .map(|lines| parse_key_value(lines))
        .unwrap_or_default();
    let beat_divisor: i32 = editor
        .get("BeatDivisor")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let hit_lines = sections.get("HitObjects")?;
    let hit_objects = match mode {
        0 => HitObjects::Standard(hit_objects::parse_standard(
            hit_lines,
            &difficulty,
            &timing_points,
        )?),
        1 => HitObjects::Taiko(hit_objects::parse_taiko(
            hit_lines,
            &difficulty,
            &timing_points,
        )?),
        2 => HitObjects::Catch(hit_objects::parse_catch(
            hit_lines,
            &difficulty,
            &timing_points,
        )?),
        3 => HitObjects::Mania(hit_objects::parse_mania(hit_lines, &difficulty)?),
        _ => return None,
    };

    Some(Beatmap {
        metadata,
        difficulty,
        general,
        timing_points,
        hit_objects,
        break_periods,
        background_filename,
        video,
        combo_colors,
        beat_divisor,
    })
}

#[cfg(test)]
pub(crate) fn parse_beatmap_str_for_tests(content: &str) -> Option<Beatmap> {
    parse_beatmap_str(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一张最小可解析的谱面，`events` 逐行填进 `[Events]`。
    fn beatmap_with_events(events: &str) -> Beatmap {
        let content = format!(
            "osu file format v14\n\n\
             [General]\nAudioFilename: audio.mp3\nMode: 0\n\n\
             [Metadata]\nTitle:Song\n\n\
             [Difficulty]\nCircleSize:4\nApproachRate:9\nOverallDifficulty:8\nHPDrainRate:5\n\n\
             [TimingPoints]\n0,500,4,2,0,100,1,0\n\n\
             [Events]\n{events}\n\
             [HitObjects]\n256,192,1000,1,0,0:0:0:0:\n"
        );
        parse_beatmap_str(&content).expect("测试谱面必须可解析")
    }

    /// 视频事件进入 `Beatmap.video`，背景事件保持独立。
    #[test]
    fn video_event_populates_video_and_keeps_background() {
        let beatmap = beatmap_with_events("0,0,\"bg.jpg\",0,0\nVideo,-250,\"intro.mp4\"");
        assert_eq!(beatmap.background_filename.as_deref(), Some("bg.jpg"));
        assert_eq!(
            beatmap.video,
            Some(VideoEvent {
                filename: "intro.mp4".to_string(),
                start_ms: -250,
            })
        );
    }

    /// 与 osu! 一致：`Video,` 行指向非视频文件时按背景图处理（老谱面兼容），
    /// 但已有正经背景事件时不覆盖它；白名单内的视频不受影响。
    #[test]
    fn video_line_with_non_video_extension_is_background_fallback() {
        let beatmap = beatmap_with_events("Video,0,\"legacy background.jpg\"");
        assert_eq!(
            beatmap.background_filename.as_deref(),
            Some("legacy background.jpg")
        );
        assert_eq!(beatmap.video, None);

        let beatmap = beatmap_with_events("0,0,\"bg.jpg\",0,0\nVideo,0,\"other.jpg\"");
        assert_eq!(beatmap.background_filename.as_deref(), Some("bg.jpg"));
        assert_eq!(beatmap.video, None);

        // 大小写扩展名同样按视频处理。
        let beatmap = beatmap_with_events("Video,0,\"clip.MP4\"");
        assert_eq!(
            beatmap.video.as_ref().map(|video| video.filename.as_str()),
            Some("clip.MP4")
        );
        assert_eq!(beatmap.background_filename, None);
    }
}
