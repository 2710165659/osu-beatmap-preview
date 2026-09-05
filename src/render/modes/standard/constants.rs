//! osu!standard 编译期常量，按缓存、规则、逻辑尺寸和样式分类。

pub(crate) mod cache {
    pub(crate) const ID_CIRCLE_PIECE: u64 = 100;
    pub(crate) const ID_SLIDER_BALL: u64 = 102;
    pub(crate) const ID_FOLLOW: u64 = 103;
    pub(crate) const ID_SLIDER_TICK: u64 = 104;
    pub(crate) const ID_ARROW_BASE: u64 = 4096;
    pub(crate) const ID_REVERSE_EDGE: u64 = 8192;
}

pub(crate) mod rules {
    pub(crate) const PLAYFIELD_WIDTH: f64 = 512.0;
    pub(crate) const PLAYFIELD_HEIGHT: f64 = 384.0;
    pub(crate) const BROKEN_GAMEFIELD_ROUNDING_ALLOWANCE: f64 = 1.00041;
    pub(crate) const POST_HIT_FADE_MS: i64 = 120;
    pub(crate) const SLIDER_FADE_OUT_MS: i64 = 240;
    pub(crate) const SPINNER_FADE_OUT_MS: i64 = 240;
    pub(crate) const BREAK_MIN_DURATION_MS: i64 = 650;
    pub(crate) const BREAK_FADE_DURATION_MS: i64 = 325;
    pub(crate) const SNAKING_IN_SLIDERS: bool = true;
    pub(crate) const SNAKING_OUT_SLIDERS: bool = true;
}

/// 这里的值是逻辑像素；使用处必须按当前输出格式的 `SCALE` 换算。
pub(crate) mod sizing {
    pub(crate) const OBJECT_RADIUS: f64 = 64.0;
    pub(crate) const BREAK_OVERLAY_BAR_HEIGHT: f64 = 8.0;
    pub(crate) const BREAK_OVERLAY_COUNTER_FONT_SIZE: u32 = 33;
    pub(crate) const BREAK_OVERLAY_INFO_FONT_SIZE: u32 = 18;
    pub(crate) const BREAK_OVERLAY_INFO_TOP_GAP: i64 = 14;
}

pub(crate) mod style {
    pub(crate) const PLAYFIELD_VIEWPORT_RATIO: f64 = 0.8;
    pub(crate) const BREAK_OVERLAY_BAR_WIDTH_RATIO: f64 = 0.3;
    pub(crate) const BREAK_OVERLAY_COLOR: [u8; 4] = [238, 238, 238, 255];
    pub(crate) const BREAK_OVERLAY_INFO_COLOR: [u8; 4] = [185, 185, 185, 255];
    pub(crate) const ARGON_BORDER_RATIO: f64 = 0.034482758620689655;
    pub(crate) const ARGON_SLIDER_WIDTH_RATIO: f64 = 0.8620703125;
    pub(crate) const ARGON_SLIDER_BORDER_PORTION: f64 = 0.2;
    pub(crate) const ARGON_SLIDER_BODY_ALPHA: f64 = 0.98;
    pub(crate) const ARGON_SLIDER_TICK_SIZE_RATIO: f64 = 12.0 / 128.0;
    pub(crate) const ARGON_SLIDER_TICK_BORDER_RATIO: f64 = 3.0 / 12.0;
    pub(crate) const ARGON_COMBO_COLORS: [[u8; 3]; 4] =
        [[255, 192, 0], [0, 202, 0], [18, 124, 255], [242, 24, 57]];
    pub(crate) const ARGON_SPINNER_PINK: [u8; 3] = [252, 97, 143];
}

pub(crate) use cache::*;
pub(crate) use rules::*;
pub(crate) use sizing::*;
pub(crate) use style::*;
