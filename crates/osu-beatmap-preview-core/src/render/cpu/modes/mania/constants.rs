//! osu!mania 编译期常量，按规则、逻辑尺寸和样式分类。

pub mod rules {
    pub const BASE_TIME_RANGE_MS: f64 = 11485.0;
    pub const DEFAULT_SCROLL_SPEED: f64 = 33.0;
}

/// 这里的值是逻辑像素；使用处必须按当前输出格式的 `SCALE` 换算。
pub mod sizing {
    pub const TOP_BUFFER: i64 = 15;
    pub const DEFAULT_HIT_TARGET_FROM_BOTTOM: f64 = 110.0;
    pub const DEFAULT_HIT_POSITION_FROM_BOTTOM: f64 = 124.8;
    pub const FRAME_HEIGHT: i64 = 768;
    pub const LANE_WIDTH: i64 = 38;
    pub const NOTE_HEAD_HEIGHT: i64 = 15;
    pub const LEFT_PANEL_WIDTH: i64 = 12;
    pub const STAGE_TOP_PADDING: i64 = 16;
    pub const NOTE_SIDE_PADDING: i64 = 2;
}

pub mod style {
    pub const MANIA_COLUMN_BACKGROUND: [u8; 4] = [0, 0, 0, 255];
    pub const MP4_LEFT_PANEL_BACKGROUND: [u8; 4] = [112, 112, 112, 255];
    pub const MP4_JUDGEMENT_LINE_COLOR: [u8; 4] = [238, 238, 238, 255];
    pub const MANIA_COLOR_W: [u8; 4] = [0xe9, 0xee, 0xf4, 255];
    pub const MANIA_COLOR_B: [u8; 4] = [0xbc, 0xdb, 0xf1, 255];
    pub const MANIA_COLOR_G: [u8; 4] = [0xcc, 0xfc, 0xb2, 255];
    pub const MANIA_COLOR_Y: [u8; 4] = [0xff, 0xe2, 0x74, 255];
    pub const MANIA_COLOR_R: [u8; 4] = [0xff, 0x7a, 0x5c, 255];
}

pub use rules::*;
pub use sizing::*;
pub use style::*;

/// PNG 固定排版与样式；尺寸按当前格式的倍率换算。
#[allow(non_snake_case)]
pub mod png {
    #[derive(Clone, Copy)]
    pub struct Sizing {
        pub PAGE_MARGIN_TOP: i64,
        pub PAGE_MARGIN_RIGHT: i64,
        pub PAGE_MARGIN_BOTTOM: i64,
        pub PAGE_MARGIN_LEFT: i64,
        pub INFO_MARGIN_TOP: i64,
        pub INFO_MARGIN_RIGHT: i64,
        pub INFO_MARGIN_BOTTOM: i64,
        pub INFO_MARGIN_LEFT: i64,
        pub LANE_WIDTH: i64,
        pub COLUMN_GAP: i64,
        pub NOTE_HEAD_HEIGHT: i64,
        pub LEFT_PANEL_WIDTH: i64,
        pub HIT_TARGET_FROM_BOTTOM: i64,
        pub NOTE_SIDE_PADDING: i64,
        pub SV_TEXT_FONT_SIZE: u32,
        pub LANE_GAP: i64,
        pub TIME_LABEL_FONT_SIZE: u32,
    }

    pub const SIZING: Sizing = Sizing {
        PAGE_MARGIN_TOP: 20,
        PAGE_MARGIN_RIGHT: 20,
        PAGE_MARGIN_BOTTOM: 20,
        PAGE_MARGIN_LEFT: 20,
        INFO_MARGIN_TOP: 0,
        INFO_MARGIN_RIGHT: 50,
        INFO_MARGIN_BOTTOM: 0,
        INFO_MARGIN_LEFT: 40,
        LANE_WIDTH: 38,
        COLUMN_GAP: 10,
        NOTE_HEAD_HEIGHT: 15,
        LEFT_PANEL_WIDTH: 12,
        HIT_TARGET_FROM_BOTTOM: 110,
        NOTE_SIDE_PADDING: 2,
        SV_TEXT_FONT_SIZE: 10,
        LANE_GAP: 0,
        TIME_LABEL_FONT_SIZE: 20,
    };

    pub fn sizing() -> Sizing {
        use crate::render::geometry::{output_scale, scale_px, GameMode, OutputFormat};
        let scale = output_scale(GameMode::Mania, OutputFormat::Png);
        Sizing {
            PAGE_MARGIN_TOP: scale_px(SIZING.PAGE_MARGIN_TOP as f64, scale),
            PAGE_MARGIN_RIGHT: scale_px(SIZING.PAGE_MARGIN_RIGHT as f64, scale),
            PAGE_MARGIN_BOTTOM: scale_px(SIZING.PAGE_MARGIN_BOTTOM as f64, scale),
            PAGE_MARGIN_LEFT: scale_px(SIZING.PAGE_MARGIN_LEFT as f64, scale),
            INFO_MARGIN_TOP: scale_px(SIZING.INFO_MARGIN_TOP as f64, scale),
            INFO_MARGIN_RIGHT: scale_px(SIZING.INFO_MARGIN_RIGHT as f64, scale),
            INFO_MARGIN_BOTTOM: scale_px(SIZING.INFO_MARGIN_BOTTOM as f64, scale),
            INFO_MARGIN_LEFT: scale_px(SIZING.INFO_MARGIN_LEFT as f64, scale),
            LANE_WIDTH: scale_px(SIZING.LANE_WIDTH as f64, scale),
            COLUMN_GAP: scale_px(SIZING.COLUMN_GAP as f64, scale),
            NOTE_HEAD_HEIGHT: scale_px(SIZING.NOTE_HEAD_HEIGHT as f64, scale),
            LEFT_PANEL_WIDTH: scale_px(SIZING.LEFT_PANEL_WIDTH as f64, scale),
            HIT_TARGET_FROM_BOTTOM: scale_px(SIZING.HIT_TARGET_FROM_BOTTOM as f64, scale),
            NOTE_SIDE_PADDING: scale_px(SIZING.NOTE_SIDE_PADDING as f64, scale),
            SV_TEXT_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.SV_TEXT_FONT_SIZE,
                scale,
            ),
            LANE_GAP: scale_px(SIZING.LANE_GAP as f64, scale),
            TIME_LABEL_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.TIME_LABEL_FONT_SIZE,
                scale,
            ),
        }
    }

    pub mod style {
        pub const LEFT_PANEL_BACKGROUND: [u8; 4] = [112, 112, 112, 255];
        pub const IMAGE_BACKGROUND: [u8; 4] = [0, 0, 0, 255];
        pub const LANE_BACKGROUND: [u8; 4] = [0, 0, 0, 255];
        pub const RULER_TEXT_COLOR: [u8; 4] = [232, 232, 232, 255];
        pub const SV_TEXT_COLOR: [u8; 4] = [95, 221, 108, 255];
        pub const LANE_SEPARATOR: [u8; 4] = [32, 32, 32, 255];
        pub const MEASURE_LINE_COLOR: [u8; 4] = [83, 83, 83, 255];
        pub const BEAT_LINE_COLOR: [u8; 4] = [56, 56, 56, 255];
        pub const SUBDIVISION_LINE: [u8; 4] = [34, 34, 34, 255];
    }
}

/// GIF 固定排版与样式；尺寸按当前格式的倍率换算。
#[allow(non_snake_case)]
pub mod gif {
    #[derive(Clone, Copy)]
    pub struct Sizing {
        pub GRID_GAP: i64,
        pub PAGE_MARGIN_TOP: i64,
        pub PAGE_MARGIN_RIGHT: i64,
        pub PAGE_MARGIN_BOTTOM: i64,
        pub PAGE_MARGIN_LEFT: i64,
        pub INFO_MARGIN_TOP: i64,
        pub INFO_MARGIN_RIGHT: i64,
        pub INFO_MARGIN_BOTTOM: i64,
        pub INFO_MARGIN_LEFT: i64,
        pub SEPARATOR_WIDTH: i64,
        pub TIME_LABEL_TOP_GAP: i64,
        pub TIME_LABEL_FONT_SIZE: u32,
        pub TIME_LABEL_NOTE_FONT_SIZE: u32,
        pub SV_TEXT_FONT_SIZE: u32,
    }

    pub const SIZING: Sizing = Sizing {
        GRID_GAP: 60,
        PAGE_MARGIN_TOP: 20,
        PAGE_MARGIN_RIGHT: 20,
        PAGE_MARGIN_BOTTOM: 20,
        PAGE_MARGIN_LEFT: 20,
        INFO_MARGIN_TOP: 0,
        INFO_MARGIN_RIGHT: 0,
        INFO_MARGIN_BOTTOM: 43,
        INFO_MARGIN_LEFT: 40,
        SEPARATOR_WIDTH: 10,
        TIME_LABEL_TOP_GAP: 5,
        TIME_LABEL_FONT_SIZE: 20,
        TIME_LABEL_NOTE_FONT_SIZE: 14,
        SV_TEXT_FONT_SIZE: 10,
    };

    pub fn sizing() -> Sizing {
        use crate::render::geometry::{output_scale, scale_px, GameMode, OutputFormat};
        let scale = output_scale(GameMode::Mania, OutputFormat::Gif);
        Sizing {
            GRID_GAP: scale_px(SIZING.GRID_GAP as f64, scale),
            PAGE_MARGIN_TOP: scale_px(SIZING.PAGE_MARGIN_TOP as f64, scale),
            PAGE_MARGIN_RIGHT: scale_px(SIZING.PAGE_MARGIN_RIGHT as f64, scale),
            PAGE_MARGIN_BOTTOM: scale_px(SIZING.PAGE_MARGIN_BOTTOM as f64, scale),
            PAGE_MARGIN_LEFT: scale_px(SIZING.PAGE_MARGIN_LEFT as f64, scale),
            INFO_MARGIN_TOP: scale_px(SIZING.INFO_MARGIN_TOP as f64, scale),
            INFO_MARGIN_RIGHT: scale_px(SIZING.INFO_MARGIN_RIGHT as f64, scale),
            INFO_MARGIN_BOTTOM: scale_px(SIZING.INFO_MARGIN_BOTTOM as f64, scale),
            INFO_MARGIN_LEFT: scale_px(SIZING.INFO_MARGIN_LEFT as f64, scale),
            SEPARATOR_WIDTH: scale_px(SIZING.SEPARATOR_WIDTH as f64, scale),
            TIME_LABEL_TOP_GAP: scale_px(SIZING.TIME_LABEL_TOP_GAP as f64, scale),
            TIME_LABEL_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.TIME_LABEL_FONT_SIZE,
                scale,
            ),
            TIME_LABEL_NOTE_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.TIME_LABEL_NOTE_FONT_SIZE,
                scale,
            ),
            SV_TEXT_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.SV_TEXT_FONT_SIZE,
                scale,
            ),
        }
    }

    pub mod style {
        pub const LEFT_PANEL_BACKGROUND: [u8; 4] = [112, 112, 112, 255];
        pub const SHOW_TIME_LABEL: bool = true;
        pub const TIME_LABEL_COLOR: [u8; 4] = [232, 232, 232, 255];
        pub const TIME_LABEL_NOTE_COLOR: [u8; 4] = [170, 170, 170, 255];
        pub const PREVIEW_TIME_LABEL_COLOR: [u8; 4] = [95, 221, 108, 255];
        pub const JUDGEMENT_LINE_COLOR: [u8; 4] = [238, 238, 238, 255];
        pub const SEPARATOR_BACKGROUND: [u8; 4] = [8, 8, 8, 255];
        pub const IMAGE_BACKGROUND: [u8; 4] = [0, 0, 0, 255];
        pub const LANE_BACKGROUND: [u8; 4] = [0, 0, 0, 255];
        pub const SV_TEXT_COLOR: [u8; 4] = [95, 221, 108, 255];
    }
}

/// MP4 固定排版与样式；尺寸按当前格式的倍率换算。
#[allow(non_snake_case)]
pub mod mp4 {
    #[derive(Clone, Copy)]
    pub struct Sizing {
        pub INFO_MARGIN_LEFT: i64,
        pub INFO_MARGIN_RIGHT: i64,
        pub LABEL_FONT_SIZE: u32,
        pub LABEL_PAD: i64,
        pub SV_TEXT_FONT_SIZE: u32,
    }

    pub const SIZING: Sizing = Sizing {
        INFO_MARGIN_LEFT: 40,
        INFO_MARGIN_RIGHT: 50,
        LABEL_FONT_SIZE: 24,
        LABEL_PAD: 12,
        SV_TEXT_FONT_SIZE: 10,
    };

    pub fn sizing() -> Sizing {
        use crate::render::geometry::{output_scale, scale_px, GameMode, OutputFormat};
        let scale = output_scale(GameMode::Mania, OutputFormat::Mp4);
        Sizing {
            INFO_MARGIN_LEFT: scale_px(SIZING.INFO_MARGIN_LEFT as f64, scale),
            INFO_MARGIN_RIGHT: scale_px(SIZING.INFO_MARGIN_RIGHT as f64, scale),
            LABEL_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.LABEL_FONT_SIZE,
                scale,
            ),
            LABEL_PAD: scale_px(SIZING.LABEL_PAD as f64, scale),
            SV_TEXT_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.SV_TEXT_FONT_SIZE,
                scale,
            ),
        }
    }

    pub mod style {
        pub const LABEL_COLOR: [u8; 4] = [232, 232, 232, 255];
        pub const IMAGE_BACKGROUND: [u8; 4] = [0, 0, 0, 255];
        pub const SV_TEXT_COLOR: [u8; 4] = [95, 221, 108, 255];
    }
}
