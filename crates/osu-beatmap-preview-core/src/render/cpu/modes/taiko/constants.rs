//! osu!taiko 编译期常量，按规则、逻辑尺寸和样式分类。

pub mod rules {
    pub const DEFAULT_BEAT_LENGTH: f64 = 500.0;
    pub const DEFAULT_METER: i32 = 4;
    pub const HIT_SOUNDS_RIM: i32 = 10;
    pub const HIT_SOUNDS_STRONG: i32 = 4;
    pub const DRUMROLL_FLAG: i32 = 2;
    pub const SWELL_FLAG: i32 = 8;
    pub const MULTIPLIER_BASE_BEAT_LENGTH: f64 = 1000.0;
    pub const BASE_PIXELS_PER_SCROLL_MS: f64 = 0.07;
    pub const SCROLL_LENGTH_RATIO: f64 = 1.6;
    pub const TAIKO_BASE_HEIGHT: f64 = 200.0;
    pub const REFERENCE_JUDGEMENT_X: f64 = 76.0;
    pub const STABLE_GAMEFIELD_HEIGHT: f64 = 480.0;
    pub const STABLE_HIT_LOCATION: f64 = 160.0;
    pub const VELOCITY_MULTIPLIER: f64 = 1.4;
}

/// 这里的值是逻辑像素；使用处必须按当前输出格式的 `SCALE` 换算。
pub mod sizing {
    pub const ROW_INNER_PADDING_X: i64 = 33;
    pub const MEASURE_LINE_WIDTH: i64 = 1;
    pub const MIN_BEAT_LINE_SPACING: f64 = 200.0;
}

pub mod style {
    pub const NORMAL_NOTE_SIZE_RATIO: f64 = 0.475;
    pub const BIG_NOTE_SCALE: f64 = 1.5384615384615383;
    pub const SPAN_BODY_HEIGHT_RATIO: f64 = 0.72;
    pub const SWELL_BODY_HEIGHT_RATIO: f64 = 0.8;
    pub const DRUM_PANEL_WIDTH_RATIO: f64 = 0.905;
    pub const CENTRE_NOTE_COLOR: [u8; 3] = [235, 69, 44];
    pub const RIM_NOTE_COLOR: [u8; 3] = [67, 142, 172];
    pub const ROLL_COLOR: [u8; 3] = [232, 198, 61];
    pub const SWELL_COLOR: [u8; 3] = [82, 204, 180];
    pub const NOTE_RING_COLOR: [u8; 4] = [245, 242, 235, 255];
    pub const NOTE_EDGE_COLOR: [u8; 4] = [0, 0, 0, 60];
    pub const NOTE_RING_THICKNESS_RATIO: f64 = 0.055;
    pub const DRUM_ROLL_TICK_DIAMETER_RATIO: f64 = 8.0 / 95.0;
    pub const DRUM_ROLL_TICK_COLOR: [u8; 4] = [255, 255, 255, 255];
    pub const MEASURE_LINE_HEIGHT_RATIO: f64 = 0.88;
    pub const ANIMATION_MEASURE_LINE_COLOR: [u8; 4] = [255, 255, 255, 170];
    pub const MP4_JUDGEMENT_LINE_COLOR: [u8; 4] = [255, 255, 255, 255];
    pub const ASPECT_RATIO: f64 = 1.7777777777777777;
}

pub use rules::*;
pub use sizing::*;
pub use style::*;

/// PNG 固定排版与样式；尺寸按当前格式的倍率换算。
#[allow(non_snake_case)]
pub mod png {
    #[derive(Clone, Copy)]
    pub struct Sizing {
        pub ROW_GAP: i64,
        pub ROW_HEIGHT: i64,
        pub ROW_INNER_PADDING_X: i64,
        pub PAGE_MARGIN_TOP: i64,
        pub PAGE_MARGIN_RIGHT: i64,
        pub PAGE_MARGIN_BOTTOM: i64,
        pub PAGE_MARGIN_LEFT: i64,
        pub INFO_MARGIN_TOP: i64,
        pub INFO_MARGIN_RIGHT: i64,
        pub INFO_MARGIN_BOTTOM: i64,
        pub INFO_MARGIN_LEFT: i64,
        pub LABEL_RIGHT_PADDING: i64,
        pub TIME_LABEL_FONT_SIZE: u32,
        pub SWELL_HIT_COUNT_FONT_SIZE: u32,
        pub SWELL_DURATION_FONT_SIZE: u32,
        pub SWELL_DURATION_TOP_GAP: i64,
        pub TIME_LABEL_NOTE_FONT_SIZE: u32,
        pub BPM_FONT_SIZE: u32,
        pub TIME_LABEL_TOP_GAP: i64,
        pub TIME_LABEL_NOTE_TOP_GAP: i64,
        pub BPM_TOP_GAP: i64,
        pub SV_TEXT_FONT_SIZE: u32,
        pub SV_TOP_GAP: i64,
    }

    pub const SIZING: Sizing = Sizing {
        ROW_GAP: 6,
        ROW_HEIGHT: 100,
        ROW_INNER_PADDING_X: 33,
        PAGE_MARGIN_TOP: 8,
        PAGE_MARGIN_RIGHT: 8,
        PAGE_MARGIN_BOTTOM: 8,
        PAGE_MARGIN_LEFT: 8,
        INFO_MARGIN_TOP: 8,
        INFO_MARGIN_RIGHT: 0,
        INFO_MARGIN_BOTTOM: 66,
        INFO_MARGIN_LEFT: 0,
        LABEL_RIGHT_PADDING: 1,
        TIME_LABEL_FONT_SIZE: 24,
        SWELL_HIT_COUNT_FONT_SIZE: 32,
        SWELL_DURATION_FONT_SIZE: 16,
        SWELL_DURATION_TOP_GAP: 3,
        TIME_LABEL_NOTE_FONT_SIZE: 17,
        BPM_FONT_SIZE: 22,
        TIME_LABEL_TOP_GAP: 0,
        TIME_LABEL_NOTE_TOP_GAP: 5,
        BPM_TOP_GAP: 5,
        SV_TEXT_FONT_SIZE: 15,
        SV_TOP_GAP: 0,
    };

    pub fn sizing() -> Sizing {
        use crate::render::geometry::{output_scale, scale_px, GameMode, OutputFormat};
        let scale = output_scale(GameMode::Taiko, OutputFormat::Png);
        Sizing {
            ROW_GAP: scale_px(SIZING.ROW_GAP as f64, scale),
            ROW_HEIGHT: scale_px(SIZING.ROW_HEIGHT as f64, scale),
            ROW_INNER_PADDING_X: scale_px(SIZING.ROW_INNER_PADDING_X as f64, scale),
            PAGE_MARGIN_TOP: scale_px(SIZING.PAGE_MARGIN_TOP as f64, scale),
            PAGE_MARGIN_RIGHT: scale_px(SIZING.PAGE_MARGIN_RIGHT as f64, scale),
            PAGE_MARGIN_BOTTOM: scale_px(SIZING.PAGE_MARGIN_BOTTOM as f64, scale),
            PAGE_MARGIN_LEFT: scale_px(SIZING.PAGE_MARGIN_LEFT as f64, scale),
            INFO_MARGIN_TOP: scale_px(SIZING.INFO_MARGIN_TOP as f64, scale),
            INFO_MARGIN_RIGHT: scale_px(SIZING.INFO_MARGIN_RIGHT as f64, scale),
            INFO_MARGIN_BOTTOM: scale_px(SIZING.INFO_MARGIN_BOTTOM as f64, scale),
            INFO_MARGIN_LEFT: scale_px(SIZING.INFO_MARGIN_LEFT as f64, scale),
            LABEL_RIGHT_PADDING: scale_px(SIZING.LABEL_RIGHT_PADDING as f64, scale),
            TIME_LABEL_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.TIME_LABEL_FONT_SIZE,
                scale,
            ),
            SWELL_HIT_COUNT_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.SWELL_HIT_COUNT_FONT_SIZE,
                scale,
            ),
            SWELL_DURATION_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.SWELL_DURATION_FONT_SIZE,
                scale,
            ),
            SWELL_DURATION_TOP_GAP: scale_px(SIZING.SWELL_DURATION_TOP_GAP as f64, scale),
            TIME_LABEL_NOTE_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.TIME_LABEL_NOTE_FONT_SIZE,
                scale,
            ),
            BPM_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.BPM_FONT_SIZE,
                scale,
            ),
            TIME_LABEL_TOP_GAP: scale_px(SIZING.TIME_LABEL_TOP_GAP as f64, scale),
            TIME_LABEL_NOTE_TOP_GAP: scale_px(SIZING.TIME_LABEL_NOTE_TOP_GAP as f64, scale),
            BPM_TOP_GAP: scale_px(SIZING.BPM_TOP_GAP as f64, scale),
            SV_TEXT_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.SV_TEXT_FONT_SIZE,
                scale,
            ),
            SV_TOP_GAP: scale_px(SIZING.SV_TOP_GAP as f64, scale),
        }
    }

    pub mod style {
        pub const SWELL_COLOR: [u8; 3] = [82, 204, 180];
        pub const SWELL_TEXT_COLOR: [u8; 4] = [245, 242, 235, 255];
        pub const SV_TEXT_COLOR: [u8; 4] = [255, 217, 102, 255];
        pub const IMAGE_BACKGROUND: [u8; 4] = [0, 0, 0, 255];
        pub const BEAT_LINE_COLOR: [u8; 4] = [83, 83, 83, 255];
        pub const RULER_TEXT_COLOR: [u8; 4] = [232, 232, 232, 255];
        pub const ACCENT_LABEL_COLOR: [u8; 4] = [95, 221, 108, 255];
        pub const MEASURE_LINE_COLOR: [u8; 4] = [255, 255, 255, 170];
        pub const TRACK_BACKGROUND_COLOR: [u8; 4] = [55, 55, 55, 255];
        pub const TRACK_EDGE_COLOR: [u8; 4] = [80, 80, 80, 255];
        pub const TRACK_ACCENT_COLOR: [u8; 4] = [254, 59, 1, 255];
    }
}

/// GIF 固定排版与样式；尺寸按当前格式的倍率换算。
#[allow(non_snake_case)]
pub mod gif {
    #[derive(Clone, Copy)]
    pub struct Sizing {
        pub SWELL_HIT_COUNT_FONT_SIZE: u32,
        pub SWELL_CLEAR_FONT_SIZE: u32,
        pub ROW_GAP: i64,
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
    }

    pub const SIZING: Sizing = Sizing {
        SWELL_HIT_COUNT_FONT_SIZE: 24,
        SWELL_CLEAR_FONT_SIZE: 16,
        ROW_GAP: 7,
        PAGE_MARGIN_TOP: 8,
        PAGE_MARGIN_RIGHT: 8,
        PAGE_MARGIN_BOTTOM: 8,
        PAGE_MARGIN_LEFT: 8,
        INFO_MARGIN_TOP: 0,
        INFO_MARGIN_RIGHT: 0,
        INFO_MARGIN_BOTTOM: 33,
        INFO_MARGIN_LEFT: 0,
        TIME_LABEL_FONT_SIZE: 20,
        TIME_LABEL_NOTE_FONT_SIZE: 14,
    };

    pub fn sizing() -> Sizing {
        use crate::render::geometry::{output_scale, scale_px, GameMode, OutputFormat};
        let scale = output_scale(GameMode::Taiko, OutputFormat::Gif);
        Sizing {
            SWELL_HIT_COUNT_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.SWELL_HIT_COUNT_FONT_SIZE,
                scale,
            ),
            SWELL_CLEAR_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.SWELL_CLEAR_FONT_SIZE,
                scale,
            ),
            ROW_GAP: scale_px(SIZING.ROW_GAP as f64, scale),
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
        }
    }

    pub mod style {
        pub const SWELL_COLOR: [u8; 4] = [82, 204, 180, 255];
        pub const SWELL_TEXT_COLOR: [u8; 4] = [245, 242, 235, 255];
        pub const SWELL_RING_THICKNESS_RATIO: f64 = 0.08;
        pub const SHOW_SWELL_MARKER: bool = true;
        pub const SHOW_SWELL_HIT_COUNT: bool = true;
        pub const SHOW_SWELL_APPROACH_CIRCLE: bool = true;
        pub const SHOW_SWELL_CLEAR: bool = true;
        pub const IMAGE_BACKGROUND: [u8; 4] = [0, 0, 0, 255];
        pub const TRACK_BACKGROUND_COLOR: [u8; 4] = [55, 55, 55, 255];
        pub const TRACK_EDGE_COLOR: [u8; 4] = [80, 80, 80, 255];
        pub const TRACK_ACCENT_COLOR: [u8; 4] = [254, 59, 1, 255];
        pub const SHOW_TIME_LABEL: bool = true;
        pub const TIME_LABEL_COLOR: [u8; 4] = [232, 232, 232, 255];
        pub const TIME_LABEL_NOTE_COLOR: [u8; 4] = [170, 170, 170, 255];
        pub const PREVIEW_TIME_LABEL_COLOR: [u8; 4] = [95, 221, 108, 255];
        pub const JUDGEMENT_LINE_COLOR: [u8; 4] = [255, 255, 255, 255];
    }
}

/// MP4 固定排版与样式；尺寸按当前格式的倍率换算。
#[allow(non_snake_case)]
pub mod mp4 {
    #[derive(Clone, Copy)]
    pub struct Sizing {
        pub SWELL_HIT_COUNT_FONT_SIZE: u32,
        pub SWELL_CLEAR_FONT_SIZE: u32,
        pub LABEL_FONT_SIZE: u32,
        pub LABEL_PAD: i64,
    }

    pub const SIZING: Sizing = Sizing {
        SWELL_HIT_COUNT_FONT_SIZE: 24,
        SWELL_CLEAR_FONT_SIZE: 16,
        LABEL_FONT_SIZE: 24,
        LABEL_PAD: 12,
    };

    pub fn sizing() -> Sizing {
        use crate::render::geometry::{output_scale, scale_px, GameMode, OutputFormat};
        let scale = output_scale(GameMode::Taiko, OutputFormat::Mp4);
        Sizing {
            SWELL_HIT_COUNT_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.SWELL_HIT_COUNT_FONT_SIZE,
                scale,
            ),
            SWELL_CLEAR_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.SWELL_CLEAR_FONT_SIZE,
                scale,
            ),
            LABEL_FONT_SIZE: crate::render::text::scaled_bitmap_font_height(
                SIZING.LABEL_FONT_SIZE,
                scale,
            ),
            LABEL_PAD: scale_px(SIZING.LABEL_PAD as f64, scale),
        }
    }

    pub mod style {
        pub const SWELL_COLOR: [u8; 4] = [82, 204, 180, 255];
        pub const SWELL_TEXT_COLOR: [u8; 4] = [245, 242, 235, 255];
        pub const SWELL_RING_THICKNESS_RATIO: f64 = 0.08;
        pub const SHOW_SWELL_MARKER: bool = true;
        pub const SHOW_SWELL_HIT_COUNT: bool = true;
        pub const SHOW_SWELL_APPROACH_CIRCLE: bool = true;
        pub const SHOW_SWELL_CLEAR: bool = true;
        pub const LABEL_COLOR: [u8; 4] = [232, 232, 232, 255];
        pub const IMAGE_BACKGROUND: [u8; 4] = [0, 0, 0, 255];
        pub const TRACK_BACKGROUND_COLOR: [u8; 4] = [55, 55, 55, 255];
        pub const TRACK_EDGE_COLOR: [u8; 4] = [80, 80, 80, 255];
        pub const TRACK_ACCENT_COLOR: [u8; 4] = [254, 59, 1, 255];
        pub const JUDGEMENT_LINE_COLOR: [u8; 4] = [255, 255, 255, 255];
    }
}
