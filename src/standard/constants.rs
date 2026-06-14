//! Constants for osu!standard renderer.

use crate::canvas::Rgba;

// ——— layout config ———
pub(crate) const PNG_MS_PER_IMAGE: i64 = 400;
pub(crate) const PNG_ROW_COUNT: usize = 5;
pub(crate) const PNG_IMAGES_PER_ROW: usize = 8;

pub(crate) const GIF_ROW_COUNT: usize = 2;
pub(crate) const GIF_IMAGES_PER_ROW: usize = 2;
pub(crate) const GIF_DURATION_MS: i64 = 5000;
pub(crate) const GIF_FPS: i64 = 15;
pub(crate) const GIF_GRID_GAP: i64 = 20;

// 单帧画面为 16:9（匹配游戏内 1080p 比例），高度沿用 384，宽度 = 384 * 16 / 9 ≈ 683。
pub(crate) const IMAGE_WIDTH: i64 = 683;
pub(crate) const IMAGE_HEIGHT: i64 = 384;
pub(crate) const HORIZONTAL_PAGE_MARGIN: i64 = 20;
pub(crate) const VERTICAL_PAGE_MARGIN: i64 = 20;
pub(crate) const INTRA_ROW_IMAGE_GAP: i64 = 20;
pub(crate) const INTER_ROW_GAP: i64 = 100;
pub(crate) const CANVAS_BACKGROUND_COLOR: Rgba = [0, 0, 0, 255];
pub(crate) const IMAGE_BACKGROUND_COLOR: Rgba = [0, 0, 0, 255];

pub(crate) const TIME_LABEL_FONT_SIZE: u32 = 30;
pub(crate) const TIME_LABEL_NOTE_FONT_SIZE: u32 = 22;
pub(crate) const TIME_LABEL_HEIGHT: i64 = 76;
pub(crate) const TIME_LABEL_TOP_GAP: i64 = 8;
pub(crate) const TIME_LABEL_NOTE_TOP_GAP: i64 = 9;
pub(crate) const TIME_LABEL_COLOR: Rgba = [232, 232, 232, 255];
pub(crate) const TIME_LABEL_NOTE_COLOR: Rgba = [170, 170, 170, 255];
pub(crate) const PREVIEW_TIME_LABEL_COLOR: Rgba = [95, 221, 108, 255];

// ——— osu! source constants ———
pub(crate) const PLAYFIELD_WIDTH: f64 = 512.0;
pub(crate) const PLAYFIELD_HEIGHT: f64 = 384.0;
pub(crate) const PLAYFIELD_VIEWPORT_RATIO: f64 = 0.8;
pub(crate) const PLAYFIELD_STORYBOARD_SHIFT: f64 = 8.0;
pub(crate) const OBJECT_RADIUS: f64 = 64.0;
pub(crate) const BROKEN_GAMEFIELD_ROUNDING_ALLOWANCE: f64 = 1.00041;
pub(crate) const POST_HIT_FADE_MS: i64 = 120;
pub(crate) const SLIDER_FADE_OUT_MS: i64 = 240;
pub(crate) const SPINNER_FADE_OUT_MS: i64 = 240;
pub(crate) const BREAK_MIN_DURATION_MS: i64 = 650;
pub(crate) const BREAK_FADE_DURATION_MS: i64 = BREAK_MIN_DURATION_MS / 2;
pub(crate) const BREAK_OVERLAY_BAR_WIDTH_RATIO: f64 = 0.3;
pub(crate) const BREAK_OVERLAY_BAR_HEIGHT: f64 = 8.0;
pub(crate) const BREAK_OVERLAY_COUNTER_FONT_SIZE: u32 = 33;
pub(crate) const BREAK_OVERLAY_INFO_FONT_SIZE: u32 = 18;
pub(crate) const BREAK_OVERLAY_INFO_TOP_GAP: i64 = 14;
pub(crate) const BREAK_OVERLAY_COLOR: Rgba = [238, 238, 238, 255];
pub(crate) const BREAK_OVERLAY_INFO_COLOR: Rgba = [185, 185, 185, 255];
pub(crate) const SLIDER_BODY_SUPERSAMPLE: i64 = 2;
pub(crate) const SNAKING_IN_SLIDERS: bool = true;
pub(crate) const SNAKING_OUT_SLIDERS: bool = true;

// ——— Argon skin constants (relative to a 128px reference object) ———
pub(crate) const ARGON_BORDER_RATIO: f64 = 2.0 / 58.0;
pub(crate) const ARGON_SLIDER_WIDTH_RATIO: f64 = 110.345 / 128.0;
pub(crate) const ARGON_SLIDER_BORDER_PORTION: f64 = 0.2;
pub(crate) const ARGON_SLIDER_BODY_ALPHA: f64 = 0.98;
pub(crate) const ARGON_COMBO_COLORS: [[u8; 3]; 4] = [
    [255, 192, 0],
    [0, 202, 0],
    [18, 124, 255],
    [242, 24, 57],
];
pub(crate) const ARGON_SPINNER_PINK: [u8; 3] = [252, 97, 143];

// cache ids for procedural pieces
pub(crate) const ID_CIRCLE_PIECE: u64 = 100;
pub(crate) const ID_SLIDER_BALL: u64 = 102;
pub(crate) const ID_FOLLOW: u64 = 103;
pub(crate) const ID_ARROW_BASE: u64 = 4096;
