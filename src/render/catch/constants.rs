//! osu!catch 渲染使用的编译期常量。
/// 谱面未提供 BPM 时使用的默认拍长（毫秒）。
pub(crate) const DEFAULT_BEAT_LENGTH: f64 = 500.0;

/// osu!catch 游戏区域宽度。
pub(crate) const PLAYFIELD_WIDTH: f64 = 512.0;
/// PNG 中游戏区域的显示宽度。
pub(crate) const PLAYFIELD_DISPLAY_WIDTH: i64 = 260;
/// 水果初始纵坐标。
pub(crate) const STABLE_FRUIT_START_Y: f64 = -100.0;
/// 接水果器纵坐标。
pub(crate) const STABLE_CATCHER_Y: f64 = 340.0;
/// 判定物基础半径。
pub(crate) const OBJECT_RADIUS: f64 = 64.0;
/// 水滴缩放比例。
pub(crate) const DROPLET_SCALE: f64 = 0.8;
/// 小水滴缩放比例。
pub(crate) const TINY_DROPLET_SCALE: f64 = 0.4;
/// 香蕉缩放比例。
pub(crate) const BANANA_SCALE: f64 = 0.6;
/// 接水果器基础尺寸。
pub(crate) const CATCHER_BASE_SIZE: f64 = 106.75;
/// stable 判定使用的接取范围倍率。
pub(crate) const ALLOWED_CATCH_RANGE: f64 = 0.8;
/// 接手普通移动的逻辑速度。
pub(crate) const BASE_WALK_SPEED: f64 = 0.5;
/// 接手冲刺移动的逻辑速度。
pub(crate) const BASE_DASH_SPEED: f64 = 1.0;
/// 香蕉雨推荐普通路线的颜色。
pub(crate) const RECOMMENDED_BANANA_COLOR: [u8; 3] = [255, 255, 255];
/// 香蕉雨推荐冲刺路线的颜色。
pub(crate) const RECOMMENDED_DASH_BANANA_COLOR: [u8; 3] = [255, 128, 128];
/// PNG 香蕉雨接盘中心路线的颜色。
pub(crate) const BANANA_ROUTE_LINE_COLOR: [u8; 4] = [74, 198, 214, 255];
/// 接盘中心路线宽度，单位为 stable 游戏区域坐标。
pub(crate) const BANANA_ROUTE_LINE_WIDTH: f64 = 4.0;
/// GIF/MP4 判定线颜色。
pub(crate) const ANIMATION_JUDGEMENT_LINE_COLOR: [u8; 4] = [238, 238, 238, 200];
/// 香蕉雨随机数种子。
pub(crate) const RNG_SEED: i64 = 1337;
/// stable 模式香蕉颜色。
pub(crate) const BANANA_COLORS: [[u8; 3]; 3] = [[255, 240, 0], [255, 192, 0], [214, 221, 28]];
/// lazer 模式默认连击颜色。
pub(crate) const LAZER_COMBO_COLORS: [[u8; 3]; 4] =
    [[255, 192, 0], [0, 202, 0], [18, 124, 255], [242, 24, 57]];
/// GIF 播放区域缩放比例。
pub(crate) const PLAYFIELD_SCALE: f64 = 0.8;
