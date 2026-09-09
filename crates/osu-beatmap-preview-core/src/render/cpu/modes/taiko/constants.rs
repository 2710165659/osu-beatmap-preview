//! osu!taiko 编译期常量，按规则、逻辑尺寸和样式分类。

pub mod rules {
    pub const DEFAULT_BEAT_LENGTH: f64 = 500.0;
    pub const DEFAULT_METER: i32 = 4;
    pub const HIT_SOUNDS_RIM: i32 = 10;
    pub const HIT_SOUNDS_STRONG: i32 = 4;
    pub const DRUMROLL_FLAG: i32 = 2;
    pub const SWELL_FLAG: i32 = 8;
    pub const MULTIPLIER_BASE_BEAT_LENGTH: f64 = 1000.0;
    pub const BASE_PIXELS_PER_SCROLL_MS: f64 = 0.07;
    pub const SCROLL_LENGTH_RATIO: f64 = 1.6;
    pub const TAIKO_BASE_HEIGHT: f64 = 200.0;
    pub const REFERENCE_JUDGEMENT_X: f64 = 76.0;
    pub const STABLE_GAMEFIELD_HEIGHT: f64 = 480.0;
    pub const STABLE_HIT_LOCATION: f64 = 160.0;
    pub const VELOCITY_MULTIPLIER: f64 = 1.4;
}

/// 这里的值是逻辑像素；使用处必须按当前输出格式的 `SCALE` 换算。
pub mod sizing {
    pub const ROW_INNER_PADDING_X: i64 = 33;
    pub const MEASURE_LINE_WIDTH: i64 = 1;
    pub const MIN_BEAT_LINE_SPACING: f64 = 200.0;
}

pub mod style {
    pub const NORMAL_NOTE_SIZE_RATIO: f64 = 0.475;
    pub const BIG_NOTE_SCALE: f64 = 1.5384615384615383;
    pub const SPAN_BODY_HEIGHT_RATIO: f64 = 0.72;
    pub const SWELL_BODY_HEIGHT_RATIO: f64 = 0.8;
    pub const DRUM_PANEL_WIDTH_RATIO: f64 = 0.905;
    pub const CENTRE_NOTE_COLOR: [u8; 3] = [235, 69, 44];
    pub const RIM_NOTE_COLOR: [u8; 3] = [67, 142, 172];
    pub const ROLL_COLOR: [u8; 3] = [232, 198, 61];
    pub const SWELL_COLOR: [u8; 3] = [82, 204, 180];
    pub const NOTE_RING_COLOR: [u8; 4] = [245, 242, 235, 255];
    pub const NOTE_EDGE_COLOR: [u8; 4] = [0, 0, 0, 60];
    pub const NOTE_RING_THICKNESS_RATIO: f64 = 0.055;
    pub const DRUM_ROLL_TICK_DIAMETER_RATIO: f64 = 8.0 / 95.0;
    pub const DRUM_ROLL_TICK_COLOR: [u8; 4] = [255, 255, 255, 255];
    pub const MEASURE_LINE_HEIGHT_RATIO: f64 = 0.88;
    pub const ANIMATION_MEASURE_LINE_COLOR: [u8; 4] = [255, 255, 255, 170];
    pub const MP4_JUDGEMENT_LINE_COLOR: [u8; 4] = [255, 255, 255, 255];
    pub const ASPECT_RATIO: f64 = 1.7777777777777777;
}

pub use rules::*;
pub use sizing::*;
pub use style::*;
