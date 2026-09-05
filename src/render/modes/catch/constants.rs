//! osu!catch 编译期常量，按规则、逻辑尺寸和样式分类。

pub(crate) mod rules {
    pub(crate) const DEFAULT_BEAT_LENGTH: f64 = 500.0;
    pub(crate) const PLAYFIELD_WIDTH: f64 = 512.0;
    pub(crate) const STABLE_FRUIT_START_Y: f64 = -100.0;
    pub(crate) const STABLE_CATCHER_Y: f64 = 340.0;
    pub(crate) const OBJECT_RADIUS: f64 = 64.0;
    pub(crate) const ALLOWED_CATCH_RANGE: f64 = 0.8;
    pub(crate) const BASE_WALK_SPEED: f64 = 0.5;
    pub(crate) const BASE_DASH_SPEED: f64 = 1.0;
    pub(crate) const RNG_SEED: i64 = 1337;
}

/// 这里的值是逻辑像素；使用处必须按当前输出格式的 `SCALE` 换算。
pub(crate) mod sizing {
    pub(crate) const PLAYFIELD_DISPLAY_WIDTH: i64 = 260;
    pub(crate) const BANANA_ROUTE_LINE_WIDTH: f64 = 4.0;
}

pub(crate) mod style {
    pub(crate) const DROPLET_SCALE: f64 = 0.8;
    pub(crate) const TINY_DROPLET_SCALE: f64 = 0.4;
    pub(crate) const BANANA_SCALE: f64 = 0.6;
    pub(crate) const CATCHER_BASE_SIZE: f64 = 106.75;
    pub(crate) const RECOMMENDED_BANANA_COLOR: [u8; 3] = [255, 255, 255];
    pub(crate) const RECOMMENDED_DASH_BANANA_COLOR: [u8; 3] = [255, 128, 128];
    pub(crate) const BANANA_ROUTE_LINE_COLOR: [u8; 4] = [74, 198, 214, 255];
    pub(crate) const ANIMATION_JUDGEMENT_LINE_COLOR: [u8; 4] = [238, 238, 238, 200];
    pub(crate) const BANANA_COLORS: [[u8; 3]; 3] = [[255, 240, 0], [255, 192, 0], [214, 221, 28]];
    pub(crate) const LAZER_COMBO_COLORS: [[u8; 3]; 4] =
        [[255, 192, 0], [0, 202, 0], [18, 124, 255], [242, 24, 57]];
    pub(crate) const PLAYFIELD_SCALE: f64 = 0.8;
}

pub(crate) use rules::*;
pub(crate) use sizing::*;
pub(crate) use style::*;
