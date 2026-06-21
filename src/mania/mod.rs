//! osu!mania renderers: vertical multi-column PNG chart and 4-segment GIF.
//! Port of beatmap_preview/mania/{renderer,gif_renderer,skin,config}.py.

pub(crate) mod conv;
mod gif;
mod png;
mod skin;
mod utils;

pub(crate) use gif::render_mania_gif;
pub(crate) use png::render_mania_grid;
pub(crate) use utils::*;

use crate::canvas::Rgba;

// ── PNG constants ──
pub(crate) const PIXELS_PER_MS: f64 = 0.4;
pub(crate) const PAGE_MARGIN_X: i64 = 20;
pub(crate) const PAGE_MARGIN_Y: i64 = 20;
pub(crate) const LANE_WIDTH: i64 = 38;
pub(crate) const COLUMN_GAP: i64 = 100;
pub(crate) const NOTE_HEAD_HEIGHT: i64 = 15;
pub(crate) const TOP_BUFFER: i64 = NOTE_HEAD_HEIGHT;
pub(crate) const LEFT_PANEL_WIDTH: i64 = 12;
pub(crate) const NOTE_SIDE_PADDING: i64 = 2;

// ── GIF constants ──
pub(crate) const GIF_SEGMENT_COUNT: i64 = 4;
pub(crate) const GIF_DURATION_MS: i64 = 10000;
pub(crate) const GIF_FPS: i64 = 15;
pub(crate) const GIF_SCROLL_SPEED: f64 = 33.0;
pub(crate) const GIF_MAX_TIME_RANGE: f64 = 11485.0;
pub(crate) const GIF_FRAME_HEIGHT: i64 = 768;
pub(crate) const GIF_GRID_GAP: i64 = 128;
pub(crate) const GIF_SEPARATOR_WIDTH: i64 = 10;
pub(crate) const GIF_HIT_TARGET_FROM_BOTTOM: i64 = 110;
pub(crate) const GIF_DEFAULT_HIT_POSITION: f64 = 124.8;
pub(crate) const GIF_STAGE_TOP_PADDING: i64 = 16;
pub(crate) const GIF_TIME_LABEL_HEIGHT: i64 = 38;
pub(crate) const GIF_TIME_LABEL_TOP_GAP: i64 = 5;
pub(crate) const GIF_TIME_LABEL_FONT_SIZE: u32 = 20;
pub(crate) const GIF_TIME_LABEL_NOTE_FONT_SIZE: u32 = 14;
pub(crate) const GIF_TIME_LABEL_COLOR: Rgba = [232, 232, 232, 255];
pub(crate) const GIF_TIME_LABEL_NOTE_COLOR: Rgba = [170, 170, 170, 255];
pub(crate) const GIF_PREVIEW_TIME_LABEL_COLOR: Rgba = [95, 221, 108, 255];
pub(crate) const GIF_JUDGEMENT_LINE: Rgba = [238, 238, 238, 255];
pub(crate) const GIF_SEPARATOR_BACKGROUND: Rgba = [8, 8, 8, 255];

// ── shared colors ──
pub(crate) const LEFT_PANEL_BACKGROUND: Rgba = [112, 112, 112, 255];
pub(crate) const IMAGE_BACKGROUND: Rgba = [0, 0, 0, 255];
pub(crate) const LANE_BACKGROUND: Rgba = [0, 0, 0, 255];
pub(crate) const RULER_TEXT: Rgba = [232, 232, 232, 255];
pub(crate) const SV_TEXT_COLOR: Rgba = [95, 221, 108, 255];
pub(crate) const SV_TEXT_FONT_SIZE: u32 = 10;
