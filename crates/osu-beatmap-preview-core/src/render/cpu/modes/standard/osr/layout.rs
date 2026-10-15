//! 回放画布在原 gameplay 内容框之外预留 HUD 空间，物件像素大小不变。
use super::super::context::{build_frame_layout, FrameLayout};
use crate::render::geometry::{
    output_scale, scale_px, video_canvas_16_9, GameMode, OutputFormat, PixelRect,
};

pub(super) fn build(format: OutputFormat) -> FrameLayout {
    let gameplay = build_frame_layout(format);
    let scale = output_scale(GameMode::Standard, format);
    let top = scale_px(48.0, scale);
    let bottom = scale_px(32.0, scale);
    let (width, height) = video_canvas_16_9(
        gameplay.frame_width as u32,
        (gameplay.frame_height + top + bottom) as u32,
    );
    let left = (width as i64 - gameplay.frame_width) / 2;
    FrameLayout {
        playfield_left: gameplay.playfield_left + left as f64,
        playfield_top: gameplay.playfield_top + top as f64,
        frame_width: width as i64,
        frame_height: height as i64,
        content: PixelRect {
            x: left,
            y: top,
            width: gameplay.frame_width,
            height: gameplay.frame_height,
        },
        ..gameplay
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reserves_hud_space_without_reducing_gameplay_scale() {
        for format in [OutputFormat::Gif, OutputFormat::Mp4] {
            let gameplay = build_frame_layout(format);
            let replay = build(format);
            assert_eq!(replay.scale, gameplay.scale);
            assert_eq!(replay.content.height, gameplay.frame_height);
            assert!(replay.content.y > 0);
            assert!(replay.frame_height > replay.content.y + replay.content.height);
        }
    }
}
