//! osu!standard 编译期常量，按缓存、规则、逻辑尺寸和样式分类。

pub mod cache {
    pub const ID_CIRCLE_PIECE: u64 = 100;
    pub const ID_SLIDER_BALL: u64 = 102;
    pub const ID_FOLLOW: u64 = 103;
    pub const ID_SLIDER_TICK: u64 = 104;
    /// 滑条球方向箭头按旋转角度缓存，占用 `ID_BALL_ARROW + 角度` 的编号。
    /// 取值与滑条 tick（`ID_SLIDER_TICK + 尺寸`）和折返箭头（`ID_ARROW_BASE + 角度`）的编号段都不重叠。
    pub const ID_BALL_ARROW: u64 = 8192;
    pub const ID_ARROW_BASE: u64 = 4096;
}

pub mod rules {
    pub const PLAYFIELD_WIDTH: f64 = 512.0;
    pub const PLAYFIELD_HEIGHT: f64 = 384.0;
    pub const BROKEN_GAMEFIELD_ROUNDING_ALLOWANCE: f64 = 1.00041;
    pub const POST_HIT_FADE_MS: i64 = 120;
    pub const SLIDER_FADE_OUT_MS: i64 = 240;
    pub const SPINNER_FADE_OUT_MS: i64 = 240;
    pub const BREAK_MIN_DURATION_MS: i64 = 650;
    pub const BREAK_FADE_DURATION_MS: i64 = 325;
    pub const SNAKING_IN_SLIDERS: bool = true;
    pub const SNAKING_OUT_SLIDERS: bool = true;
    /// 相邻跟随点的间距（playfield 单位，对应 lazer `FollowPointConnection.SPACING`）。
    pub const FOLLOW_POINT_SPACING: i64 = 32;
    /// 跟随点的淡出提前量（毫秒，对应 lazer `FollowPointConnection.PREEMPT`）。
    pub const FOLLOW_POINT_PREEMPT_MS: f64 = 800.0;
    /// AR=10 时的最小 preempt（毫秒，对应 lazer `OsuHitObject.PREEMPT_MIN`）：
    /// 更短的 preempt 会让跟随点按同一比例整体加快。
    pub const FOLLOW_POINT_PREEMPT_MIN_MS: f64 = 450.0;
    /// 跟随点淡入与淡出时长的上限（毫秒，对应 lazer `OsuHitObject.TimeFadeIn`）。
    pub const FOLLOW_POINT_FADE_IN_MS: f64 = 400.0;
}

/// 这里的值是逻辑像素；使用处必须按当前输出格式的 `SCALE` 换算。
pub mod sizing {
    pub const OBJECT_RADIUS: f64 = 64.0;
    pub const BREAK_OVERLAY_BAR_HEIGHT: f64 = 8.0;
    pub const BREAK_OVERLAY_COUNTER_FONT_SIZE: u32 = 33;
    pub const BREAK_OVERLAY_INFO_FONT_SIZE: u32 = 18;
    pub const BREAK_OVERLAY_INFO_TOP_GAP: i64 = 14;
    /// 跟随点图标外框边长（playfield 单位）：lazer `FollowPoint` 里 `SpriteIcon` 的尺寸 8。
    /// chevron 字形高大于宽，等比缩放到该方框后墨迹高度就等于方框边长。
    pub const FOLLOW_POINT_ICON_SIZE: f64 = 8.0;
    /// FontAwesome Solid `ChevronRight` 的墨迹宽高比（字形路径包围盒约 262×429）。
    pub const FOLLOW_POINT_CHEVRON_ASPECT: f64 = 262.0 / 429.0;
    /// chevron 笔画厚度相对墨迹高度的比例（字形路径中斜边带的垂直厚度）。
    pub const FOLLOW_POINT_CHEVRON_THICKNESS_RATIO: f64 = 0.16;
    /// 两个 chevron 的墨迹中心间距相对墨迹高度的比例：
    /// lazer 里第二个图标相对第一个偏移 4 个 playfield 单位，正好是图标外框的一半。
    pub const FOLLOW_POINT_CHEVRON_GAP_RATIO: f64 = 0.5;
    /// 跟随点淡入起始时的整体缩放倍率（相对 `end.Scale`）：
    /// lazer 为 `fp.Scale = 1.5 * end.Scale`，再 `ScaleTo(end.Scale, ...)` 收敛到 1 倍。
    pub const FOLLOW_POINT_SCALE_START: f64 = 1.5;
}

pub mod style {
    pub const PLAYFIELD_VIEWPORT_RATIO: f64 = 0.8;
    pub const BREAK_OVERLAY_BAR_WIDTH_RATIO: f64 = 0.3;
    pub const BREAK_OVERLAY_COLOR: [u8; 4] = [238, 238, 238, 255];
    pub const BREAK_OVERLAY_INFO_COLOR: [u8; 4] = [185, 185, 185, 255];
    pub const ARGON_BORDER_RATIO: f64 = 0.034482758620689655;
    pub const ARGON_SLIDER_WIDTH_RATIO: f64 = 0.8620703125;
    pub const ARGON_SLIDER_BORDER_PORTION: f64 = 0.2;
    pub const ARGON_SLIDER_BODY_ALPHA: f64 = 0.98;
    pub const ARGON_SLIDER_TICK_SIZE_RATIO: f64 = 12.0 / 128.0;
    pub const ARGON_SLIDER_TICK_BORDER_RATIO: f64 = 3.0 / 12.0;
    /// 滑条球方向箭头：lazer `ArgonSliderBall` 内的 FontAwesome Solid `AngleRight` 图标
    /// （SpriteIcon 尺寸 48、缩放 (0.6, 0.8)，即墨迹高度 38.4 = 物件直径 128 的 0.3 倍）。
    pub const ARGON_SLIDER_BALL_ARROW_HEIGHT_RATIO: f64 = 0.3;
    /// 以下三个比例来自该图标在 font size 100 下的字形位图（38×60），
    /// 单位都是字形高度（60）的倍数：45° 笔画的垂直厚度 21/√2≈14.85、
    /// 折角中心线半高 22.575、尖端中心线相对字形中心的横向偏移 9.75。
    pub const ARGON_SLIDER_BALL_ARROW_THICKNESS_RATIO: f64 = 14.85 / 60.0;
    pub const ARGON_SLIDER_BALL_ARROW_HALF_HEIGHT_RATIO: f64 = 22.575 / 60.0;
    pub const ARGON_SLIDER_BALL_ARROW_TIP_OFFSET_RATIO: f64 = 9.75 / 60.0;
    pub const ARGON_COMBO_COLORS: [[u8; 3]; 4] =
        [[255, 192, 0], [0, 202, 0], [18, 124, 255], [242, 24, 57]];
    pub const ARGON_SPINNER_PINK: [u8; 3] = [252, 97, 143];
    /// 跟随点图标的竖向渐变：lazer `ArgonFollowPoint` 的
    /// `ColourInfo.GradientVertical(FC618F, BB1A41)`，上亮下暗。
    pub const ARGON_FOLLOW_POINT_TOP: [u8; 3] = [0xFC, 0x61, 0x8F];
    pub const ARGON_FOLLOW_POINT_BOTTOM: [u8; 3] = [0xBB, 0x1A, 0x41];
    /// 前一个（拖后）chevron 叠加 `OsuColour.Gray(0.2)` 后的亮度比例。
    pub const ARGON_FOLLOW_POINT_DIM: f64 = 0.2;
}

pub use cache::*;
pub use rules::*;
pub use sizing::*;
pub use style::*;
