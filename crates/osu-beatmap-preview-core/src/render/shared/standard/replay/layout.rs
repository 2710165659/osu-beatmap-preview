//! 回放沿用同格式普通预览的布局，HUD 叠加在画布内，不额外预留空间。
use crate::render::cpu::modes::standard::context::{
    build_frame_layout, build_video_frame_layout, FrameLayout,
};
use crate::render::geometry::OutputFormat;

pub(super) fn build(format: OutputFormat) -> FrameLayout {
    match format {
        OutputFormat::Mp4 => build_video_frame_layout(format),
        OutputFormat::Png | OutputFormat::Gif => build_frame_layout(format),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_matches_preview_layout_for_each_format_and_scale() {
        for scale in [0.75, 1.0, 2.0] {
            let mut config = crate::config::CoreConfig::default();
            // 各格式使用不同倍率，避免回放意外借用其他格式的 SCALE。
            config.render.standard.png.SCALE = scale;
            config.render.standard.gif.SCALE = scale * 1.5;
            config.render.standard.mp4.SCALE = scale * 2.0;
            crate::config::with_config(std::sync::Arc::new(config), || {
                for format in [OutputFormat::Png, OutputFormat::Gif, OutputFormat::Mp4] {
                    let gameplay = if format == OutputFormat::Mp4 {
                        build_video_frame_layout(format)
                    } else {
                        build_frame_layout(format)
                    };
                    let replay = build(format);
                    assert_eq!(replay.scale, gameplay.scale);
                    assert_eq!(replay.playfield_left, gameplay.playfield_left);
                    assert_eq!(replay.playfield_top, gameplay.playfield_top);
                    assert_eq!(replay.frame_width, gameplay.frame_width);
                    assert_eq!(replay.frame_height, gameplay.frame_height);
                    assert_eq!(replay.content, gameplay.content);
                }
            });
        }
    }
}
