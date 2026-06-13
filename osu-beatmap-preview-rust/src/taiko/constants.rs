//! Constants for osu!taiko renderers.

use crate::canvas::Rgba;

// ─── config.py constants ───

pub(crate) const MAX_SUPPORTED_DURATION_MS: i64 = 10 * 60 * 1000;

pub(crate) const BASE_ROW_WIDTH_0_TO_1_MIN: i64 = 2600;
pub(crate) const BASE_ROW_WIDTH_1_TO_2_MIN: i64 = 3200;
pub(crate) const BASE_ROW_WIDTH_2_TO_3_MIN: i64 = 3800;
pub(crate) const BASE_ROW_WIDTH_3_TO_4_MIN: i64 = 4400;
pub(crate) const BASE_ROW_WIDTH_4_TO_5_MIN: i64 = 5000;
pub(crate) const BASE_ROW_WIDTH_5_TO_6_MIN: i64 = 5600;
pub(crate) const BASE_ROW_WIDTH_6_TO_10_MIN: i64 = 6400;

pub(crate) const ROW_WIDTH_BPM_0_TO_180: f64 = 1.0;
pub(crate) const ROW_WIDTH_BPM_180_TO_240: f64 = 1.15;
pub(crate) const ROW_WIDTH_BPM_240_TO_300: f64 = 1.3;
pub(crate) const ROW_WIDTH_BPM_300_PLUS: f64 = 1.45;

pub(crate) const DRAW_DRUM_EACH_ROW: bool = false;
pub(crate) const ROW_GAP: i64 = 80;
pub(crate) const ROW_HEIGHT: i64 = 80;
pub(crate) const SPACING_BPM: f64 = 0.0;

pub(crate) const PIXELS_PER_SCROLL_MULTIPLIER_MS: f64 = 0.07;
pub(crate) const SCROLL_LENGTH_RATIO: f64 = 1.6;
pub(crate) const DEFAULT_BEAT_LENGTH: f64 = 500.0;
pub(crate) const DEFAULT_METER: i32 = 4;

pub(crate) const PAGE_MARGIN_X: i64 = 8;
pub(crate) const PAGE_MARGIN_Y: i64 = 8;
pub(crate) const ROW_INNER_PADDING_X: i64 = 33;
pub(crate) const LABEL_RIGHT_PADDING: i64 = 1;
pub(crate) const MIN_BEAT_LINE_SPACING: f64 = 200.0;
pub(crate) const TIME_LABEL_FONT_SIZE: u32 = 24;
pub(crate) const TIME_LABEL_NOTE_FONT_SIZE: u32 = 17;
pub(crate) const BPM_FONT_SIZE: u32 = 22;
pub(crate) const TIME_LABEL_TOP_GAP: i64 = 0;
pub(crate) const TIME_LABEL_NOTE_TOP_GAP: i64 = 5;
pub(crate) const BPM_TOP_GAP: i64 = 5;
pub(crate) const SV_TEXT_COLOR: Rgba = [255, 217, 102, 255];
pub(crate) const SV_TEXT_FONT_SIZE: u32 = 15;
pub(crate) const SV_TOP_GAP: i64 = 0;

pub(crate) const IMAGE_BACKGROUND: Rgba = [0, 0, 0, 255];
pub(crate) const CENTRE_NOTE_COLOR: [u8; 3] = [235, 69, 44];
pub(crate) const RIM_NOTE_COLOR: [u8; 3] = [67, 142, 172];
pub(crate) const ROLL_COLOR: [u8; 3] = [232, 198, 61];
pub(crate) const SWELL_COLOR: [u8; 3] = [82, 204, 180];
pub(crate) const BEAT_LINE_COLOR: Rgba = [83, 83, 83, 255];
pub(crate) const RULER_TEXT_COLOR: Rgba = [232, 232, 232, 255];
pub(crate) const ACCENT_LABEL_COLOR: Rgba = [95, 221, 108, 255];

pub(crate) const NORMAL_NOTE_SIZE_RATIO: f64 = 0.475;
pub(crate) const BIG_NOTE_SCALE: f64 = 1.0 / 0.65;
pub(crate) const SPAN_BODY_HEIGHT_RATIO: f64 = 0.72;
pub(crate) const SWELL_BODY_HEIGHT_RATIO: f64 = 0.8;

// 程序化 classic-2013 note 风格（无中心符号）
pub(crate) const NOTE_RING_COLOR: Rgba = [245, 242, 235, 255];
pub(crate) const NOTE_EDGE_COLOR: Rgba = [0, 0, 0, 60];
pub(crate) const NOTE_RING_THICKNESS_RATIO: f64 = 0.055;
pub(crate) const MEASURE_LINE_COLOR: Rgba = [255, 255, 255, 170];

// 程序化行背景颜色（替代原 taiko-bar-left/right 图片）
/// note 滚动轨道底色（原图采样为 60,60,60 半透明，叠在黑底上）
pub(crate) const TRACK_BACKGROUND_COLOR: Rgba = [55, 55, 55, 255];
/// 轨道上下边缘高光
pub(crate) const TRACK_EDGE_COLOR: Rgba = [80, 80, 80, 255];
/// 鼓面板红色饰条（取自 classic 皮肤主色）
pub(crate) const TRACK_ACCENT_COLOR: Rgba = [254, 59, 1, 255];

// GIF config
pub(crate) const GIF_SEGMENT_COUNT: usize = 4;
pub(crate) const GIF_DURATION_MS: f64 = 5000.0;
pub(crate) const GIF_FPS: f64 = 15.0;
pub(crate) const GIF_ROW_HEIGHT: i64 = 80;
pub(crate) const GIF_ROW_GAP: i64 = 60;
pub(crate) const GIF_TIME_LABEL_FONT_SIZE: u32 = 20;
pub(crate) const GIF_TIME_LABEL_NOTE_FONT_SIZE: u32 = 14;
pub(crate) const GIF_TIME_LABEL_COLOR: Rgba = [232, 232, 232, 255];
pub(crate) const GIF_TIME_LABEL_NOTE_COLOR: Rgba = [170, 170, 170, 255];
pub(crate) const GIF_PREVIEW_TIME_LABEL_COLOR: Rgba = [95, 221, 108, 255];
pub(crate) const GIF_JUDGEMENT_LINE_COLOR: Rgba = [255, 255, 255, 255];
pub(crate) const GIF_TAIKO_BASE_HEIGHT: f64 = 200.0;
pub(crate) const GIF_REFERENCE_SCROLL_LENGTH: f64 = 1109.3333333333333;
pub(crate) const GIF_REFERENCE_JUDGEMENT_X: f64 = 76.0;
pub(crate) const GIF_STABLE_GAMEFIELD_HEIGHT: f64 = 480.0;
pub(crate) const GIF_STABLE_HIT_LOCATION: f64 = 160.0;
pub(crate) const GIF_VELOCITY_MULTIPLIER: f64 = 1.4;
pub(crate) const GIF_ASPECT: f64 = 16.0 / 9.0;

pub(crate) const HIT_SOUNDS_RIM: i32 = 2 | 8;
pub(crate) const HIT_SOUNDS_STRONG: i32 = 4;
pub(crate) const DRUMROLL_FLAG: i32 = 2;
pub(crate) const SWELL_FLAG: i32 = 8;

pub(crate) const MULTIPLIER_BASE_BEAT_LENGTH: f64 = 1000.0;
