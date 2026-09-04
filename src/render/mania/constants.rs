/// PNG 谱面顶部缓冲高度（像素）。
pub(crate) const TOP_BUFFER: i64 = 15;
/// GIF 时间轴默认显示范围（毫秒）。
pub(crate) const BASE_TIME_RANGE_MS: f64 = 11485.0;
/// MP4 未提供独立配置时使用的游戏滚动速度基准。
pub(crate) const DEFAULT_SCROLL_SPEED: f64 = 33.0;
/// 动画格式在皮肤字段缺失时使用的判定线距底部距离。
pub(crate) const DEFAULT_HIT_TARGET_FROM_BOTTOM: f64 = 110.0;
/// MP4 未提供独立样式字段时使用的轨道背景色。
pub(crate) const MP4_LANE_BACKGROUND: [u8; 4] = [0, 0, 0, 255];
/// MP4 未提供独立样式字段时使用的左右侧板颜色。
pub(crate) const MP4_LEFT_PANEL_BACKGROUND: [u8; 4] = [112, 112, 112, 255];
/// MP4 未提供独立样式字段时使用的判定线颜色。
pub(crate) const MP4_JUDGEMENT_LINE_COLOR: [u8; 4] = [238, 238, 238, 255];
/// 动画格式在皮肤颜色缺失时使用的轨道颜色。
pub(crate) const DEFAULT_LANE_BACKGROUND: [u8; 4] = [0, 0, 0, 255];
/// GIF 默认判定线距底部的距离（像素）。
pub(crate) const DEFAULT_HIT_POSITION_FROM_BOTTOM: f64 = 124.8;
/// Mania GIF/MP4 游戏区域高度（像素）。
pub(crate) const FRAME_HEIGHT: i64 = 768;
/// GIF/MP4 轨道宽度基准。
pub(crate) const LANE_WIDTH: i64 = 38;
/// GIF/MP4 音符头高度基准。
pub(crate) const NOTE_HEAD_HEIGHT: i64 = 15;
/// GIF/MP4 左侧面板宽度基准。
pub(crate) const LEFT_PANEL_WIDTH: i64 = 12;
/// GIF/MP4 游戏区域顶部留白。
pub(crate) const STAGE_TOP_PADDING: i64 = 16;
/// Mania GIF/MP4 音符左右侧留白。
pub(crate) const NOTE_SIDE_PADDING: i64 = 2;
