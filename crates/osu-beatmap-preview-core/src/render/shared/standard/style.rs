//! 准备阶段解析当前格式的绘制样式，逐帧不再依赖调用线程的配置作用域。

use crate::render::geometry::OutputFormat;

#[derive(Clone, Copy)]
pub struct StandardRenderStyle {
    pub background_color: [u8; 4],
    pub slider_supersample: i64,
}

impl StandardRenderStyle {
    pub(crate) fn current(format: OutputFormat) -> Self {
        let config = crate::config::current();
        let standard = &config.render.standard;
        let (background_color, slider_supersample) = match format {
            OutputFormat::Png => (
                standard.png.style.IMAGE_BACKGROUND_COLOR,
                standard.png.style.SLIDER_BODY_SUPERSAMPLE,
            ),
            OutputFormat::Gif => (
                standard.gif.style.IMAGE_BACKGROUND_COLOR,
                standard.gif.style.SLIDER_BODY_SUPERSAMPLE,
            ),
            // 视频没有独立超采样配置，保留默认 2 倍，不借用 PNG 的独立设置。
            OutputFormat::Mp4 => (standard.mp4.style.IMAGE_BACKGROUND_COLOR, 2),
        };
        Self {
            background_color,
            slider_supersample,
        }
    }
}
