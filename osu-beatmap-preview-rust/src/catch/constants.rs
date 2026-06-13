//! Constants for osu!catch renderer.

use crate::canvas::Rgba;

pub(crate) const MAX_SUPPORTED_DURATION_MS: i64 = 10 * 60 * 1000;

pub(crate) const MAX_AREA_HEIGHT_0_TO_1_MIN: i64 = 4000;
pub(crate) const MAX_AREA_HEIGHT_1_TO_2_MIN: i64 = 5500;
pub(crate) const MAX_AREA_HEIGHT_2_TO_3_MIN: i64 = 7000;
pub(crate) const MAX_AREA_HEIGHT_3_TO_4_MIN: i64 = 8500;
pub(crate) const MAX_AREA_HEIGHT_4_TO_5_MIN: i64 = 10000;
pub(crate) const MAX_AREA_HEIGHT_5_TO_6_MIN: i64 = 11500;
/// 谱面总像素高度上限（所有列合计）。超出时压缩纵向密度，
/// 限制最终图像内存占用（约 30 列 × 11500 px 的量级）。
pub(crate) const MAX_TOTAL_CHART_HEIGHT: i64 = 240_000;

pub(crate) const PLAYFIELD_WIDTH: f64 = 512.0;
pub(crate) const STABLE_FRUIT_START_Y: f64 = -100.0;
pub(crate) const STABLE_CATCHER_Y: f64 = 340.0;
pub(crate) const OBJECT_RADIUS: f64 = 64.0;

pub(crate) const PAGE_MARGIN_X: i64 = 20;
pub(crate) const PAGE_MARGIN_Y: i64 = 20;
pub(crate) const LEFT_PANEL_WIDTH: i64 = 12;
pub(crate) const COLUMN_WIDTH: i64 = 360;
pub(crate) const COLUMN_GAP: i64 = 100;

pub(crate) const LEFT_PANEL_BACKGROUND: Rgba = [112, 112, 112, 255];
pub(crate) const IMAGE_BACKGROUND: Rgba = [0, 0, 0, 255];
pub(crate) const PLAYFIELD_BACKGROUND: Rgba = [7, 7, 7, 255];
pub(crate) const PLAYFIELD_BORDER: Rgba = [34, 34, 34, 255];
pub(crate) const MEASURE_LINE: Rgba = [87, 87, 87, 255];
pub(crate) const BEAT_LINE: Rgba = [62, 62, 62, 255];

pub(crate) const DROPLET_SCALE: f64 = 0.8;
pub(crate) const TINY_DROPLET_SCALE: f64 = 0.4;
pub(crate) const BANANA_SCALE: f64 = 0.6;

pub(crate) const CATCHER_BASE_SIZE: f64 = 106.75;
pub(crate) const LEGACY_CATCHER_VISUAL_SCALE: f64 = 0.35;
pub(crate) const CATCHER_SPRITE_LOGICAL_WIDTH: f64 = 307.0;

pub(crate) const DEFAULT_BEAT_LENGTH: f64 = 500.0;
pub(crate) const RNG_SEED: u32 = 1337;

pub(crate) const BANANA_COLORS: [[u8; 3]; 3] =
    [[255, 240, 0], [255, 192, 0], [214, 221, 28]];

pub(crate) const LAZER_COMBO_COLORS: [[u8; 3]; 4] = [
    [255, 192, 0],
    [0, 202, 0],
    [18, 124, 255],
    [242, 24, 57],
];

pub(crate) const GIF_ROW_COUNT: i64 = 2;
pub(crate) const GIF_IMAGES_PER_ROW: i64 = 2;
pub(crate) const GIF_SEGMENT_COUNT: usize = (GIF_ROW_COUNT * GIF_IMAGES_PER_ROW) as usize;
pub(crate) const GIF_DURATION_MS: f64 = 5000.0;
pub(crate) const GIF_FPS: f64 = 15.0;
/// 单帧为 16:9（匹配游戏内 1080p 比例）。
pub(crate) const GIF_IMAGE_WIDTH: i64 = 683;
pub(crate) const GIF_IMAGE_HEIGHT: i64 = 384;
pub(crate) const GIF_GRID_GAP: i64 = 20;

// ─── 游戏内 1080p 布局换算（CatchPlayfieldAdjustmentContainer） ───
// 游戏逻辑空间高 768；playfield 取 80%（1024×768 → 819.2×614.4），
// 顶部偏移 = 768 × (1-0.8)/4×3 = 115.2；本帧高 384 = 768 的一半，
// 故 screen_scale = 0.5，playfield 在帧内的横向缩放 = 1.6 × 0.5 = 0.8。
/// 帧高相对游戏逻辑空间（768 高）的缩放。
pub(crate) const GIF_SCREEN_SCALE: f64 = 384.0 / 768.0;
/// playfield 相对 512 宽坐标系在帧内的缩放（1.6 × screen_scale）。
pub(crate) const GIF_PLAYFIELD_SCALE: f64 = 1.6 * GIF_SCREEN_SCALE;
/// playfield 顶部在帧内的 y 偏移（游戏内 115.2 × screen_scale）。
pub(crate) const GIF_PLAYFIELD_TOP: f64 = 115.2 * GIF_SCREEN_SCALE;
pub(crate) const GIF_TIME_LABEL_FONT_SIZE: u32 = 30;
pub(crate) const GIF_TIME_LABEL_NOTE_FONT_SIZE: u32 = 22;
pub(crate) const GIF_TIME_LABEL_HEIGHT: i64 = 76;
pub(crate) const GIF_TIME_LABEL_TOP_GAP: i64 = 8;
pub(crate) const GIF_TIME_LABEL_NOTE_TOP_GAP: i64 = 9;
pub(crate) const GIF_TIME_LABEL_COLOR: Rgba = [232, 232, 232, 255];
pub(crate) const GIF_TIME_LABEL_NOTE_COLOR: Rgba = [170, 170, 170, 255];
pub(crate) const GIF_PREVIEW_TIME_LABEL_COLOR: Rgba = [95, 221, 108, 255];
