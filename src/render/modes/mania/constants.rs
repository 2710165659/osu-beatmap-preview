//! osu!mania 编译期常量，按规则、逻辑尺寸和样式分类。

pub(crate) mod rules {
    pub(crate) const BASE_TIME_RANGE_MS: f64 = 11485.0;
    pub(crate) const DEFAULT_SCROLL_SPEED: f64 = 33.0;
}

/// 这里的值是逻辑像素；使用处必须按当前输出格式的 `SCALE` 换算。
pub(crate) mod sizing {
    pub(crate) const TOP_BUFFER: i64 = 15;
    pub(crate) const DEFAULT_HIT_TARGET_FROM_BOTTOM: f64 = 110.0;
    pub(crate) const DEFAULT_HIT_POSITION_FROM_BOTTOM: f64 = 124.8;
    pub(crate) const FRAME_HEIGHT: i64 = 768;
    pub(crate) const LANE_WIDTH: i64 = 38;
    pub(crate) const NOTE_HEAD_HEIGHT: i64 = 15;
    pub(crate) const LEFT_PANEL_WIDTH: i64 = 12;
    pub(crate) const STAGE_TOP_PADDING: i64 = 16;
    pub(crate) const NOTE_SIDE_PADDING: i64 = 2;
}

pub(crate) mod style {
    pub(crate) const MP4_LANE_BACKGROUND: [u8; 4] = [0, 0, 0, 255];
    pub(crate) const MP4_LEFT_PANEL_BACKGROUND: [u8; 4] = [112, 112, 112, 255];
    pub(crate) const MP4_JUDGEMENT_LINE_COLOR: [u8; 4] = [238, 238, 238, 255];
    pub(crate) const DEFAULT_LANE_BACKGROUND: [u8; 4] = [0, 0, 0, 255];
    pub(crate) const MANIA_COLOR_W: [u8; 4] = [0xe9, 0xee, 0xf4, 255];
    pub(crate) const MANIA_COLOR_B: [u8; 4] = [0xbc, 0xdb, 0xf1, 255];
    pub(crate) const MANIA_COLOR_G: [u8; 4] = [0xcc, 0xfc, 0xb2, 255];
    pub(crate) const MANIA_COLOR_Y: [u8; 4] = [0xff, 0xe2, 0x74, 255];
    pub(crate) const MANIA_COLOR_R: [u8; 4] = [0xff, 0x7a, 0x5c, 255];
}

pub(crate) use rules::*;
pub(crate) use sizing::*;
pub(crate) use style::*;
