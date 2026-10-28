//! osu!catch 编译期常量，按规则、逻辑尺寸和样式分类。

pub mod rules {
    pub const DEFAULT_BEAT_LENGTH: f64 = 500.0;
    pub const PLAYFIELD_WIDTH: f64 = 512.0;
    pub const STABLE_FRUIT_START_Y: f64 = -100.0;
    pub const STABLE_CATCHER_Y: f64 = 340.0;
    pub const OBJECT_RADIUS: f64 = 64.0;
    pub const ALLOWED_CATCH_RANGE: f64 = 0.8;
    pub const BASE_WALK_SPEED: f64 = 0.5;
    pub const BASE_DASH_SPEED: f64 = 1.0;
    pub const RNG_SEED: i64 = 1337;
}

/// 这里的值是逻辑像素；使用处必须按当前输出格式的 `SCALE` 换算。
pub mod sizing {
    pub const PLAYFIELD_DISPLAY_WIDTH: i64 = 260;
    pub const BANANA_ROUTE_LINE_WIDTH: f64 = 4.0;
    pub const CATCHER_PLATE_HEIGHT: f64 = 18.0;
}

pub mod style {
    pub const DROPLET_SCALE: f64 = 0.8;
    pub const TINY_DROPLET_SCALE: f64 = 0.4;
    pub const BANANA_SCALE: f64 = 0.6;
    pub const CATCHER_BASE_SIZE: f64 = 106.75;
    pub const RECOMMENDED_BANANA_COLOR: [u8; 3] = [255, 255, 255];
    pub const RECOMMENDED_DASH_BANANA_COLOR: [u8; 3] = [255, 128, 128];
    pub const BANANA_ROUTE_LINE_COLOR: [u8; 4] = [74, 198, 214, 255];
    pub const ANIMATION_JUDGEMENT_LINE_COLOR: [u8; 4] = [238, 238, 238, 200];
    pub const BANANA_COLORS: [[u8; 3]; 3] = [[255, 240, 0], [255, 192, 0], [214, 221, 28]];
    pub const LAZER_COMBO_COLORS: [[u8; 3]; 4] =
        [[255, 192, 0], [0, 202, 0], [18, 124, 255], [242, 24, 57]];
    pub const PLAYFIELD_SCALE: f64 = 0.8;
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
        pub LEFT_PANEL_WIDTH: i64,
        pub COLUMN_WIDTH: i64,
        pub COLUMN_GAP: i64,
        pub TIME_LABEL_FONT_SIZE: u32,
        pub BPM_LABEL_GAP: i64,
        pub EDGE_GUIDE_WIDTH: f64,
        pub EDGE_COMBO_LABEL_FONT_SIZE: u32,
        pub EDGE_COMBO_LABEL_GAP: f64,
        pub EDGE_COMBO_LABEL_PADDING: i64,
        pub EDGE_COMBO_LABEL_SHADOW_GAP: i64,
    }

    pub const SIZING: Sizing = Sizing {
        PAGE_MARGIN_TOP: 15,
        PAGE_MARGIN_RIGHT: 15,
        PAGE_MARGIN_BOTTOM: 15,
        PAGE_MARGIN_LEFT: 15,
        INFO_MARGIN_TOP: 0,
        INFO_MARGIN_RIGHT: 60,
        INFO_MARGIN_BOTTOM: 0,
        INFO_MARGIN_LEFT: 0,
        LEFT_PANEL_WIDTH: 9,
        COLUMN_WIDTH: 315,
        COLUMN_GAP: 15,
        TIME_LABEL_FONT_SIZE: 14,
        BPM_LABEL_GAP: 2,
        EDGE_GUIDE_WIDTH: 3.0,
        EDGE_COMBO_LABEL_FONT_SIZE: 16,
        EDGE_COMBO_LABEL_GAP: 5.0,
        EDGE_COMBO_LABEL_PADDING: 2,
        EDGE_COMBO_LABEL_SHADOW_GAP: 1,
    };

    pub fn sizing() -> Sizing {
        use crate::render::geometry::{output_scale, scale_px, GameMode, OutputFormat};
        let scale = output_scale(GameMode::Catch, OutputFormat::Png);
        Sizing {
            PAGE_MARGIN_TOP: scale_px(SIZING.PAGE_MARGIN_TOP as f64, scale),
            PAGE_MARGIN_RIGHT: scale_px(SIZING.PAGE_MARGIN_RIGHT as f64, scale),
            PAGE_MARGIN_BOTTOM: scale_px(SIZING.PAGE_MARGIN_BOTTOM as f64, scale),
            PAGE_MARGIN_LEFT: scale_px(SIZING.PAGE_MARGIN_LEFT as f64, scale),
            INFO_MARGIN_TOP: scale_px(SIZING.INFO_MARGIN_TOP as f64, scale),
            INFO_MARGIN_RIGHT: scale_px(SIZING.INFO_MARGIN_RIGHT as f64, scale),
            INFO_MARGIN_BOTTOM: scale_px(SIZING.INFO_MARGIN_BOTTOM as f64, scale),
            INFO_MARGIN_LEFT: scale_px(SIZING.INFO_MARGIN_LEFT as f64, scale),
            LEFT_PANEL_WIDTH: scale_px(SIZING.LEFT_PANEL_WIDTH as f64, scale),
            COLUMN_WIDTH: scale_px(SIZING.COLUMN_WIDTH as f64, scale),
            COLUMN_GAP: scale_px(SIZING.COLUMN_GAP as f64, scale),
            TIME_LABEL_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.TIME_LABEL_FONT_SIZE,
                scale,
            ),
            BPM_LABEL_GAP: scale_px(SIZING.BPM_LABEL_GAP as f64, scale),
            EDGE_GUIDE_WIDTH: SIZING.EDGE_GUIDE_WIDTH * scale,
            EDGE_COMBO_LABEL_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.EDGE_COMBO_LABEL_FONT_SIZE,
                scale,
            ),
            EDGE_COMBO_LABEL_GAP: SIZING.EDGE_COMBO_LABEL_GAP * scale,
            EDGE_COMBO_LABEL_PADDING: scale_px(SIZING.EDGE_COMBO_LABEL_PADDING as f64, scale),
            EDGE_COMBO_LABEL_SHADOW_GAP: scale_px(SIZING.EDGE_COMBO_LABEL_SHADOW_GAP as f64, scale),
        }
    }

    pub mod style {
        pub const LEFT_PANEL_BACKGROUND: [u8; 4] = [112, 112, 112, 255];
        pub const IMAGE_BACKGROUND: [u8; 4] = [7, 7, 7, 255];
        pub const PLAYFIELD_BACKGROUND: [u8; 4] = [7, 7, 7, 255];
        pub const PLAYFIELD_BORDER: [u8; 4] = [34, 34, 34, 255];
        pub const MEASURE_LINE_COLOR: [u8; 4] = [87, 87, 87, 255];
        pub const BEAT_LINE_COLOR: [u8; 4] = [62, 62, 62, 255];
        pub const TIME_LABEL_COLOR: [u8; 4] = [200, 200, 200, 255];
        pub const BPM_LABEL_COLOR: [u8; 4] = [255, 82, 82, 255];
        pub const EDGE_GUIDE_COLOR: [u8; 4] = [255, 255, 255, 255];
        pub const EDGE_COMBO_LABEL_COLOR: [u8; 4] = [255, 255, 255, 255];
        pub const EDGE_COMBO_LABEL_SHADOW: [u8; 4] = [0, 0, 0, 220];
        pub const EDGE_COMBO_LABEL_BACKGROUND: [u8; 4] = [7, 7, 7, 230];
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
        pub TIME_LABEL_FONT_SIZE: u32,
        pub TIME_LABEL_NOTE_FONT_SIZE: u32,
        pub TIME_LABEL_TOP_GAP: i64,
        pub TIME_LABEL_NOTE_TOP_GAP: i64,
    }

    pub const SIZING: Sizing = Sizing {
        GRID_GAP: 20,
        PAGE_MARGIN_TOP: 15,
        PAGE_MARGIN_RIGHT: 15,
        PAGE_MARGIN_BOTTOM: 15,
        PAGE_MARGIN_LEFT: 15,
        INFO_MARGIN_TOP: 0,
        INFO_MARGIN_RIGHT: 0,
        INFO_MARGIN_BOTTOM: 84,
        INFO_MARGIN_LEFT: 0,
        TIME_LABEL_FONT_SIZE: 30,
        TIME_LABEL_NOTE_FONT_SIZE: 22,
        TIME_LABEL_TOP_GAP: 8,
        TIME_LABEL_NOTE_TOP_GAP: 9,
    };

    pub fn sizing() -> Sizing {
        use crate::render::geometry::{output_scale, scale_px, GameMode, OutputFormat};
        let scale = output_scale(GameMode::Catch, OutputFormat::Gif);
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
            TIME_LABEL_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.TIME_LABEL_FONT_SIZE,
                scale,
            ),
            TIME_LABEL_NOTE_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.TIME_LABEL_NOTE_FONT_SIZE,
                scale,
            ),
            TIME_LABEL_TOP_GAP: scale_px(SIZING.TIME_LABEL_TOP_GAP as f64, scale),
            TIME_LABEL_NOTE_TOP_GAP: scale_px(SIZING.TIME_LABEL_NOTE_TOP_GAP as f64, scale),
        }
    }

    pub mod style {
        pub const PLAYFIELD_BACKGROUND: [u8; 4] = [7, 7, 7, 255];
        pub const SHOW_TIME_LABEL: bool = true;
        pub const TIME_LABEL_COLOR: [u8; 4] = [232, 232, 232, 255];
        pub const TIME_LABEL_NOTE_COLOR: [u8; 4] = [170, 170, 170, 255];
        pub const PREVIEW_TIME_LABEL_COLOR: [u8; 4] = [95, 221, 108, 255];
    }
}

/// MP4 固定排版与样式；尺寸按当前格式的倍率换算。
#[allow(non_snake_case)]
pub mod mp4 {
    #[derive(Clone, Copy)]
    pub struct Sizing {
        pub LABEL_FONT_SIZE: u32,
        pub LABEL_PAD: i64,
    }

    pub const SIZING: Sizing = Sizing {
        LABEL_FONT_SIZE: 24,
        LABEL_PAD: 12,
    };

    pub fn sizing() -> Sizing {
        use crate::render::geometry::{output_scale, scale_px, GameMode, OutputFormat};
        let scale = output_scale(GameMode::Catch, OutputFormat::Mp4);
        Sizing {
            LABEL_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.LABEL_FONT_SIZE,
                scale,
            ),
            LABEL_PAD: scale_px(SIZING.LABEL_PAD as f64, scale),
        }
    }

    pub mod style {
        pub const LABEL_COLOR: [u8; 4] = [232, 232, 232, 255];
        pub const PLAYFIELD_BACKGROUND: [u8; 4] = [7, 7, 7, 255];
    }
}
