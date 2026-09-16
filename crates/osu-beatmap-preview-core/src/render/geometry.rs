//! 四模式共享的 playfield、缩放与视频画布几何。

use crate::domain::parser::round_half_even;

pub const STANDARD_CATCH_PLAYFIELD_WIDTH: f64 = 409.6;
pub const STANDARD_CATCH_PLAYFIELD_HEIGHT: f64 = 307.2;
pub const TAIKO_PLAYFIELD_WIDTH: f64 = 682.665;
pub const TAIKO_PLAYFIELD_HEIGHT: f64 = 100.0;
pub const MANIA_PLAYFIELD_HEIGHT: f64 = 384.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    Standard,
    Taiko,
    Catch,
    Mania,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Png,
    Gif,
    Mp4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayfieldGeometry {
    /// 可承载模式对象的内容框。Standard/Catch 的内容框大于 playfield。
    pub content: PixelRect,
    /// 游戏规则实际使用的 playfield，Mania/Taiko 以此作为硬裁剪边界。
    pub playfield: PixelRect,
}

pub fn output_scale(mode: GameMode, format: OutputFormat) -> f64 {
    let layout = &crate::config::current().render;
    match (mode, format) {
        (GameMode::Standard, OutputFormat::Png) => layout.standard.png.SCALE,
        (GameMode::Standard, OutputFormat::Gif) => layout.standard.gif.SCALE,
        (GameMode::Standard, OutputFormat::Mp4) => layout.standard.mp4.SCALE,
        (GameMode::Taiko, OutputFormat::Png) => layout.taiko.png.SCALE,
        (GameMode::Taiko, OutputFormat::Gif) => layout.taiko.gif.SCALE,
        (GameMode::Taiko, OutputFormat::Mp4) => layout.taiko.mp4.SCALE,
        (GameMode::Catch, OutputFormat::Png) => layout.catch.png.SCALE,
        (GameMode::Catch, OutputFormat::Gif) => layout.catch.gif.SCALE,
        (GameMode::Catch, OutputFormat::Mp4) => layout.catch.mp4.SCALE,
        (GameMode::Mania, OutputFormat::Png) => layout.mania.png.SCALE,
        (GameMode::Mania, OutputFormat::Gif) => layout.mania.gif.SCALE,
        (GameMode::Mania, OutputFormat::Mp4) => layout.mania.mp4.SCALE,
    }
}

pub fn scale_px(value: f64, scale: f64) -> i64 {
    round_half_even(value * scale)
}

/// 缩放可见线宽；低倍率下仍至少保留一个物理像素。
pub fn scale_stroke_px(value: f64, scale: f64) -> i64 {
    scale_px(value, scale).max(1)
}

pub fn standard_geometry(format: OutputFormat) -> PlayfieldGeometry {
    let scale = output_scale(GameMode::Standard, format);
    let content_width = scale_px(530.0, scale);
    let content_height = scale_px(384.0, scale);
    let playfield_width = scale_px(STANDARD_CATCH_PLAYFIELD_WIDTH, scale);
    let playfield_height = scale_px(STANDARD_CATCH_PLAYFIELD_HEIGHT, scale);
    let left = scale_px((530.0 - STANDARD_CATCH_PLAYFIELD_WIDTH) / 2.0, scale);
    let top = scale_px(
        (384.0 - STANDARD_CATCH_PLAYFIELD_HEIGHT) / 2.0 + 8.0 * 0.8,
        scale,
    );
    PlayfieldGeometry {
        content: PixelRect {
            x: 0,
            y: 0,
            width: content_width,
            height: content_height,
        },
        playfield: PixelRect {
            x: left,
            y: top,
            width: playfield_width,
            height: playfield_height,
        },
    }
}

pub fn catch_geometry(format: OutputFormat) -> PlayfieldGeometry {
    let scale = output_scale(GameMode::Catch, format);
    let content_width = scale_px(470.0, scale);
    let content_height = scale_px(384.0, scale);
    let playfield_width = scale_px(STANDARD_CATCH_PLAYFIELD_WIDTH, scale);
    let playfield_height = scale_px(STANDARD_CATCH_PLAYFIELD_HEIGHT, scale);
    PlayfieldGeometry {
        content: PixelRect {
            x: 0,
            y: 0,
            width: content_width,
            height: content_height,
        },
        playfield: PixelRect {
            x: scale_px((470.0 - STANDARD_CATCH_PLAYFIELD_WIDTH) / 2.0, scale),
            y: scale_px(57.6, scale),
            width: playfield_width,
            height: playfield_height,
        },
    }
}

pub fn taiko_geometry(format: OutputFormat) -> PlayfieldGeometry {
    let scale = output_scale(GameMode::Taiko, format);
    let rect = PixelRect {
        x: 0,
        y: 0,
        width: scale_px(TAIKO_PLAYFIELD_WIDTH, scale),
        height: scale_px(TAIKO_PLAYFIELD_HEIGHT, scale),
    };
    PlayfieldGeometry {
        content: rect,
        playfield: rect,
    }
}

/// 视频画布尺寸与内容框在画布中的居中位置。
///
/// `origin` 是内容框左上角在画布中的整数坐标：物件层的帧尺寸就是画布，
/// 因此物件只会在视频边界被裁，不会再被中间缓冲切掉一半。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoCanvas {
    pub width: u32,
    pub height: u32,
    pub origin_x: i64,
    pub origin_y: i64,
}

/// 由内容框推导 16:9 视频画布。
///
/// 尺寸公式与 [`video_canvas_16_9`] 完全一致，因此分辨率不变；
/// 原点使用向下取整的整除，与合成阶段原来的居中偏移保持逐像素一致。
pub fn video_canvas(content: PixelRect) -> VideoCanvas {
    let (width, height) =
        video_canvas_16_9(content.width.max(1) as u32, content.height.max(1) as u32);
    VideoCanvas {
        width,
        height,
        origin_x: (width as i64 - content.width) / 2,
        origin_y: (height as i64 - content.height) / 2,
    }
}

/// 以内容框为下限补齐最接近的 16:9，并始终向上取为偶数，避免裁掉边缘像素。
pub fn video_canvas_16_9(content_width: u32, content_height: u32) -> (u32, u32) {
    let width = content_width.max(1) as u64;
    let height = content_height.max(1) as u64;
    let (width, height) = if width * 9 >= height * 16 {
        (width, (width * 9).div_ceil(16))
    } else {
        ((height * 16).div_ceil(9), height)
    };
    let even = |value: u64| value.div_ceil(2) * 2;
    (even(width) as u32, even(height) as u32)
}

#[cfg(test)]
mod tests {
    use super::{scale_px, scale_stroke_px, video_canvas, video_canvas_16_9, PixelRect};

    #[test]
    fn base_playfield_lengths_use_half_even_rounding() {
        assert_eq!(scale_px(409.6, 1.0), 410);
        assert_eq!(scale_px(307.2, 1.0), 307);
        assert_eq!(scale_px(682.665, 1.0), 683);
        assert_eq!(scale_px(100.0, 1.0), 100);
    }

    #[test]
    fn video_canvas_never_shrinks_content_and_is_even() {
        for (width, height) in [(410, 307), (683, 100), (272, 384), (1920, 1080)] {
            let (out_width, out_height) = video_canvas_16_9(width, height);
            assert!(out_width >= width);
            assert!(out_height >= height);
            assert_eq!(out_width % 2, 0);
            assert_eq!(out_height % 2, 0);
            assert!((out_width as i64 * 9 - out_height as i64 * 16).abs() <= 24);
        }
    }

    #[test]
    fn visible_strokes_keep_at_least_one_physical_pixel() {
        assert_eq!(scale_stroke_px(1.0, 0.01), 1);
        assert_eq!(scale_stroke_px(1.0, 0.5), 1);
        assert_eq!(scale_stroke_px(1.0, 1.5), 2);
        assert_eq!(scale_stroke_px(2.0, 2.0), 4);
    }

    #[test]
    fn video_canvas_keeps_the_legacy_resolution_and_centers_content() {
        // 四模式 1x/2x 的内容框：画布尺寸必须与旧公式一致，否则等于改了视频分辨率。
        for (width, height) in [(530, 384), (470, 384), (683, 100), (384, 384)] {
            let content = PixelRect {
                x: 0,
                y: 0,
                width,
                height,
            };
            let canvas = video_canvas(content);
            let (legacy_width, legacy_height) = video_canvas_16_9(width as u32, height as u32);
            assert_eq!((canvas.width, canvas.height), (legacy_width, legacy_height));
            // 内容框必须完整落在画布内，且左右/上下留白最多相差 1 像素。
            assert!(canvas.origin_x >= 0 && canvas.origin_y >= 0);
            assert!(canvas.origin_x + width <= canvas.width as i64);
            assert!(canvas.origin_y + height <= canvas.height as i64);
            let right = canvas.width as i64 - canvas.origin_x - width;
            let bottom = canvas.height as i64 - canvas.origin_y - height;
            assert!((canvas.origin_x - right).abs() <= 1);
            assert!((canvas.origin_y - bottom).abs() <= 1);
        }
    }

    #[test]
    fn standard_video_canvas_matches_known_resolutions() {
        let standard = PixelRect {
            x: 0,
            y: 0,
            width: 530,
            height: 384,
        };
        assert_eq!(
            video_canvas(standard),
            super::VideoCanvas {
                width: 684,
                height: 384,
                origin_x: 77,
                origin_y: 0,
            }
        );
        let taiko = PixelRect {
            x: 0,
            y: 0,
            width: 683,
            height: 100,
        };
        assert_eq!(
            video_canvas(taiko),
            super::VideoCanvas {
                width: 684,
                height: 386,
                origin_x: 0,
                origin_y: 143,
            }
        );
    }
}
