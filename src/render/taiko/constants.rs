//! osu!taiko 游戏规则与渲染使用的编译期常量。

/// 谱面未提供 BPM 时使用的默认拍长（毫秒）。
pub(crate) const DEFAULT_BEAT_LENGTH: f64 = 500.0;
/// 谱面未提供拍号时使用的默认分母。
pub(crate) const DEFAULT_METER: i32 = 4;
/// 鼓边音符 hitsound 标志。
pub(crate) const HIT_SOUNDS_RIM: i32 = 10;
/// 强打音符 hitsound 标志。
pub(crate) const HIT_SOUNDS_STRONG: i32 = 4;
/// 连打音符类型标志。
pub(crate) const DRUMROLL_FLAG: i32 = 2;
/// 大连打音符类型标志。
pub(crate) const SWELL_FLAG: i32 = 8;
/// 速度倍率计算使用的基准拍长（毫秒）。
pub(crate) const MULTIPLIER_BASE_BEAT_LENGTH: f64 = 1000.0;

/// 普通音符直径占行高的比例。
pub(crate) const NORMAL_NOTE_SIZE_RATIO: f64 = 0.475;
/// 大连音符相对普通音符的缩放比例。
pub(crate) const BIG_NOTE_SCALE: f64 = 1.5384615384615383;
/// 连打主体高度占行高的比例。
pub(crate) const SPAN_BODY_HEIGHT_RATIO: f64 = 0.72;
/// 大连打主体高度占行高的比例。
pub(crate) const SWELL_BODY_HEIGHT_RATIO: f64 = 0.8;
/// Taiko 行内容左右内边距基准（GIF/MP4）。
pub(crate) const ROW_INNER_PADDING_X: i64 = 33;

/// 鼓面面板宽度占比。
pub(crate) const DRUM_PANEL_WIDTH_RATIO: f64 = 0.905;
/// 中心音符颜色。
pub(crate) const CENTRE_NOTE_COLOR: [u8; 3] = [235, 69, 44];
/// 鼓边音符颜色。
pub(crate) const RIM_NOTE_COLOR: [u8; 3] = [67, 142, 172];
/// 连打音符颜色。
pub(crate) const ROLL_COLOR: [u8; 3] = [232, 198, 61];
/// 大连打音符颜色。
pub(crate) const SWELL_COLOR: [u8; 3] = [82, 204, 180];
/// 音符环颜色。
pub(crate) const NOTE_RING_COLOR: [u8; 4] = [245, 242, 235, 255];
/// 音符边缘颜色。
pub(crate) const NOTE_EDGE_COLOR: [u8; 4] = [0, 0, 0, 60];
/// 音符环厚度占音符直径比例。
pub(crate) const NOTE_RING_THICKNESS_RATIO: f64 = 0.055;
/// 连打点白色菱形的外接尺寸相对普通 Taiko 音符直径。
pub(crate) const DRUM_ROLL_TICK_DIAMETER_RATIO: f64 = 8.0 / 95.0;
/// 连打点主体颜色。
pub(crate) const DRUM_ROLL_TICK_COLOR: [u8; 4] = [255, 255, 255, 255];
/// classic-2013 小节线占轨道高度的比例。
pub(crate) const MEASURE_LINE_HEIGHT_RATIO: f64 = 0.88;
/// osu! 的小节线跟踪器宽度。
pub(crate) const MEASURE_LINE_WIDTH: i64 = 1;
/// 动画小节线颜色，独立于 PNG 图表的配色配置。
pub(crate) const ANIMATION_MEASURE_LINE_COLOR: [u8; 4] = [255, 255, 255, 170];
/// MP4 未提供独立字段时使用的判定线颜色。
pub(crate) const MP4_JUDGEMENT_LINE_COLOR: [u8; 4] = [255, 255, 255, 255];
/// PNG 时间轴每毫秒滚动像素数。
pub(crate) const BASE_PIXELS_PER_SCROLL_MS: f64 = 0.07;
/// 时间轴滚动长度比例。
pub(crate) const SCROLL_LENGTH_RATIO: f64 = 1.6;
/// 相邻节拍线的最小间距。
pub(crate) const MIN_BEAT_LINE_SPACING: f64 = 200.0;
/// GIF 基准游戏区域高度。
pub(crate) const TAIKO_BASE_HEIGHT: f64 = 200.0;
/// 参考判定位置横坐标。
pub(crate) const REFERENCE_JUDGEMENT_X: f64 = 76.0;
/// stable 游戏区域高度。
pub(crate) const STABLE_GAMEFIELD_HEIGHT: f64 = 480.0;
/// stable 判定位置。
pub(crate) const STABLE_HIT_LOCATION: f64 = 160.0;
/// 速度换算倍数。
pub(crate) const VELOCITY_MULTIPLIER: f64 = 1.4;
/// Taiko GIF/MP4 游戏视口纵横比。
pub(crate) const ASPECT_RATIO: f64 = 1.7777777777777777;
