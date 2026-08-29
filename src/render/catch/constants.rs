//! osu!catch 渲染使用的编译期常量。
/// 谱面未提供 BPM 时使用的默认拍长（毫秒）。
pub(crate) const DEFAULT_BEAT_LENGTH: f64 = 500.0;

/// osu!catch 游戏区域宽度。
pub(crate) const PLAYFIELD_WIDTH: f64 = 512.0;
/// PNG 中游戏区域的显示宽度。
pub(crate) const PLAYFIELD_DISPLAY_WIDTH: i64 = 260;
/// GIF 中游戏区域的顶部位置。
pub(crate) const PLAYFIELD_TOP: f64 = 57.6;
/// GIF 单帧画布宽度。
pub(crate) const IMAGE_WIDTH: i64 = 470;
/// GIF 单帧画布高度。
pub(crate) const IMAGE_HEIGHT: i64 = 384;
/// Catch GIF/MP4 画布水平边距。
pub(crate) const PAGE_MARGIN_X: i64 = 15;
/// Catch GIF/MP4 画布垂直边距。
pub(crate) const PAGE_MARGIN_Y: i64 = 15;
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
/// 香蕉雨随机数种子。
pub(crate) const RNG_SEED: i64 = 1337;
/// stable 模式香蕉颜色。
pub(crate) const BANANA_COLORS: [[u8; 3]; 3] = [[255, 240, 0], [255, 192, 0], [214, 221, 28]];
/// lazer 模式默认连击颜色。
pub(crate) const LAZER_COMBO_COLORS: [[u8; 3]; 4] =
    [[255, 192, 0], [0, 202, 0], [18, 124, 255], [242, 24, 57]];
/// GIF 播放区域缩放比例。
pub(crate) const PLAYFIELD_SCALE: f64 = 0.8;
