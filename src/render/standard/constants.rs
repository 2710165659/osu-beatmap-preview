//! osu!standard 渲染使用的编译期常量。
/// 圆圈图层缓存 ID 基值。
pub(crate) const ID_CIRCLE_PIECE: u64 = 100;
/// 滑条球图层缓存 ID 基值。
pub(crate) const ID_SLIDER_BALL: u64 = 102;
/// 跟随圈图层缓存 ID 基值。
pub(crate) const ID_FOLLOW: u64 = 103;
/// 反向箭头图层缓存 ID 基值。
pub(crate) const ID_ARROW_BASE: u64 = 4096;
/// 反向边缘图层缓存 ID 基值。
pub(crate) const ID_REVERSE_EDGE: u64 = 8192;

/// osu!standard 游戏区域宽度。
pub(crate) const PLAYFIELD_WIDTH: f64 = 512.0;
/// osu!standard 游戏区域高度。
pub(crate) const PLAYFIELD_HEIGHT: f64 = 384.0;
/// GIF/MP4 单帧画布宽度。
pub(crate) const IMAGE_WIDTH: i64 = 530;
/// GIF/MP4 单帧画布高度。
pub(crate) const IMAGE_HEIGHT: i64 = 384;
/// 游戏区域在视口中的缩放比例。
pub(crate) const PLAYFIELD_VIEWPORT_RATIO: f64 = 0.8;
/// 游戏区域相对 storyboard 的纵向偏移。
pub(crate) const PLAYFIELD_STORYBOARD_SHIFT: f64 = 8.0;
/// 圆形判定物半径。
pub(crate) const OBJECT_RADIUS: f64 = 64.0;
/// 修正旧版游戏区域圆角误差的容差。
pub(crate) const BROKEN_GAMEFIELD_ROUNDING_ALLOWANCE: f64 = 1.00041;
/// 击中后判定物淡出时长（毫秒）。
pub(crate) const POST_HIT_FADE_MS: i64 = 120;
/// 滑条淡出时长（毫秒）。
pub(crate) const SLIDER_FADE_OUT_MS: i64 = 240;
/// 转盘淡出时长（毫秒）。
pub(crate) const SPINNER_FADE_OUT_MS: i64 = 240;
/// Break 时段最短持续时间（毫秒）。
pub(crate) const BREAK_MIN_DURATION_MS: i64 = 650;
/// Break 覆盖层淡入淡出时长（毫秒）。
pub(crate) const BREAK_FADE_DURATION_MS: i64 = 325;
/// Break 进度条宽度占画布比例。
pub(crate) const BREAK_OVERLAY_BAR_WIDTH_RATIO: f64 = 0.3;
/// Break 进度条高度（像素）。
pub(crate) const BREAK_OVERLAY_BAR_HEIGHT: f64 = 8.0;
/// Break 计数文字字号。
pub(crate) const BREAK_OVERLAY_COUNTER_FONT_SIZE: u32 = 33;
/// Break 提示文字字号。
pub(crate) const BREAK_OVERLAY_INFO_FONT_SIZE: u32 = 18;
/// Break 提示文字顶部间距（毫秒）。
pub(crate) const BREAK_OVERLAY_INFO_TOP_GAP: i64 = 14;
/// Break 进度条颜色。
pub(crate) const BREAK_OVERLAY_COLOR: [u8; 4] = [238, 238, 238, 255];
/// Break 提示文字颜色。
pub(crate) const BREAK_OVERLAY_INFO_COLOR: [u8; 4] = [185, 185, 185, 255];
/// 滑条头部蛇形展开效果开关。
pub(crate) const SNAKING_IN_SLIDERS: bool = true;
/// 滑条尾部蛇形收缩效果开关。
pub(crate) const SNAKING_OUT_SLIDERS: bool = true;
/// Argon 滑条边框宽度比例。
pub(crate) const ARGON_BORDER_RATIO: f64 = 0.034482758620689655;
/// Argon 滑条主体宽度比例。
pub(crate) const ARGON_SLIDER_WIDTH_RATIO: f64 = 0.8620703125;
/// Argon 滑条边框占比。
pub(crate) const ARGON_SLIDER_BORDER_PORTION: f64 = 0.2;
/// Argon 滑条主体透明度。
pub(crate) const ARGON_SLIDER_BODY_ALPHA: f64 = 0.98;
/// Argon 默认连击颜色。
pub(crate) const ARGON_COMBO_COLORS: [[u8; 3]; 4] =
    [[255, 192, 0], [0, 202, 0], [18, 124, 255], [242, 24, 57]];
/// Argon 转盘粉色。
pub(crate) const ARGON_SPINNER_PINK: [u8; 3] = [252, 97, 143];
