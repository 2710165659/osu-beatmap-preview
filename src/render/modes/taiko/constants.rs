//! osu!taiko 编译期常量，按规则、逻辑尺寸和样式分类。

pub(crate) mod rules {
    pub(crate) const DEFAULT_BEAT_LENGTH: f64 = 500.0;
    pub(crate) const DEFAULT_METER: i32 = 4;
    pub(crate) const HIT_SOUNDS_RIM: i32 = 10;
    pub(crate) const HIT_SOUNDS_STRONG: i32 = 4;
    pub(crate) const DRUMROLL_FLAG: i32 = 2;
    pub(crate) const SWELL_FLAG: i32 = 8;
    pub(crate) const MULTIPLIER_BASE_BEAT_LENGTH: f64 = 1000.0;
    pub(crate) const BASE_PIXELS_PER_SCROLL_MS: f64 = 0.07;
    pub(crate) const SCROLL_LENGTH_RATIO: f64 = 1.6;
    pub(crate) const TAIKO_BASE_HEIGHT: f64 = 200.0;
    pub(crate) const REFERENCE_JUDGEMENT_X: f64 = 76.0;
    pub(crate) const STABLE_GAMEFIELD_HEIGHT: f64 = 480.0;
    pub(crate) const STABLE_HIT_LOCATION: f64 = 160.0;
    pub(crate) const VELOCITY_MULTIPLIER: f64 = 1.4;
}

/// 这里的值是逻辑像素；使用处必须按当前输出格式的 `SCALE` 换算。
pub(crate) mod sizing {
    pub(crate) const ROW_INNER_PADDING_X: i64 = 33;
    pub(crate) const MEASURE_LINE_WIDTH: i64 = 1;
    pub(crate) const MIN_BEAT_LINE_SPACING: f64 = 200.0;
}

pub(crate) mod style {
    pub(crate) const NORMAL_NOTE_SIZE_RATIO: f64 = 0.475;
    pub(crate) const BIG_NOTE_SCALE: f64 = 1.5384615384615383;
    pub(crate) const SPAN_BODY_HEIGHT_RATIO: f64 = 0.72;
    pub(crate) const SWELL_BODY_HEIGHT_RATIO: f64 = 0.8;
    pub(crate) const DRUM_PANEL_WIDTH_RATIO: f64 = 0.905;
    pub(crate) const CENTRE_NOTE_COLOR: [u8; 3] = [235, 69, 44];
    pub(crate) const RIM_NOTE_COLOR: [u8; 3] = [67, 142, 172];
    pub(crate) const ROLL_COLOR: [u8; 3] = [232, 198, 61];
    pub(crate) const SWELL_COLOR: [u8; 3] = [82, 204, 180];
    pub(crate) const NOTE_RING_COLOR: [u8; 4] = [245, 242, 235, 255];
    pub(crate) const NOTE_EDGE_COLOR: [u8; 4] = [0, 0, 0, 60];
    pub(crate) const NOTE_RING_THICKNESS_RATIO: f64 = 0.055;
    pub(crate) const DRUM_ROLL_TICK_DIAMETER_RATIO: f64 = 8.0 / 95.0;
    pub(crate) const DRUM_ROLL_TICK_COLOR: [u8; 4] = [255, 255, 255, 255];
    pub(crate) const MEASURE_LINE_HEIGHT_RATIO: f64 = 0.88;
    pub(crate) const ANIMATION_MEASURE_LINE_COLOR: [u8; 4] = [255, 255, 255, 170];
    pub(crate) const MP4_JUDGEMENT_LINE_COLOR: [u8; 4] = [255, 255, 255, 255];
    pub(crate) const ASPECT_RATIO: f64 = 1.7777777777777777;
}

pub(crate) use rules::*;
pub(crate) use sizing::*;
pub(crate) use style::*;
