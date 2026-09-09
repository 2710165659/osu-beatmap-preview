//! osu!catch 编译期常量，按规则、逻辑尺寸和样式分类。

pub mod rules {
    pub const DEFAULT_BEAT_LENGTH: f64 = 500.0;
    pub const PLAYFIELD_WIDTH: f64 = 512.0;
    pub const STABLE_FRUIT_START_Y: f64 = -100.0;
    pub const STABLE_CATCHER_Y: f64 = 340.0;
    pub const OBJECT_RADIUS: f64 = 64.0;
    pub const ALLOWED_CATCH_RANGE: f64 = 0.8;
    pub const BASE_WALK_SPEED: f64 = 0.5;
    pub const BASE_DASH_SPEED: f64 = 1.0;
    pub const RNG_SEED: i64 = 1337;
}

/// 这里的值是逻辑像素；使用处必须按当前输出格式的 `SCALE` 换算。
pub mod sizing {
    pub const PLAYFIELD_DISPLAY_WIDTH: i64 = 260;
    pub const BANANA_ROUTE_LINE_WIDTH: f64 = 4.0;
}

pub mod style {
    pub const DROPLET_SCALE: f64 = 0.8;
    pub const TINY_DROPLET_SCALE: f64 = 0.4;
    pub const BANANA_SCALE: f64 = 0.6;
    pub const CATCHER_BASE_SIZE: f64 = 106.75;
    pub const RECOMMENDED_BANANA_COLOR: [u8; 3] = [255, 255, 255];
    pub const RECOMMENDED_DASH_BANANA_COLOR: [u8; 3] = [255, 128, 128];
    pub const BANANA_ROUTE_LINE_COLOR: [u8; 4] = [74, 198, 214, 255];
    pub const ANIMATION_JUDGEMENT_LINE_COLOR: [u8; 4] = [238, 238, 238, 200];
    pub const BANANA_COLORS: [[u8; 3]; 3] = [[255, 240, 0], [255, 192, 0], [214, 221, 28]];
    pub const LAZER_COMBO_COLORS: [[u8; 3]; 4] =
        [[255, 192, 0], [0, 202, 0], [18, 124, 255], [242, 24, 57]];
    pub const PLAYFIELD_SCALE: f64 = 0.8;
}

pub use rules::*;
pub use sizing::*;
pub use style::*;
