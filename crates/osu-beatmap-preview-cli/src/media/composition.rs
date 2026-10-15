//! 视频和回放 GIF 共用背景、故事板合成；回放 HUD 自带时间标签。
use super::{storyboard::MediaStoryboard, video::VideoStyle};
use crate::export::{
    canvas::Img,
    text::{draw_text, text_size},
};

/// 将谱面背景图按 osu! 的等比适应方式缩放并居中，超出部分保留黑边，
/// 再应用暗化配置。背景只在最终视频画布上处理一次，避免 playfield 与画布
/// 分别裁剪同一张图片造成画面不连续。
pub(crate) fn prepare_video_background(
    source: &Img,
    width: u32,
    height: u32,
    style: VideoStyle,
) -> Img {
    prepare_video_background_inner(source, width, height, style, false)
}

fn prepare_video_background_inner(
    source: &Img,
    width: u32,
    height: u32,
    style: VideoStyle,
    cover: bool,
) -> Img {
    let mut result = Img::new(width, height, style.black_opaque);
    if source.w == 0 || source.h == 0 || width == 0 || height == 0 {
        return result;
    }

    // CPU 路径使用 contain；WGPU 的 Standard/Catch 使用 cover，消除固定画布黑边。
    let scale = if cover {
        (width as f64 / source.w as f64).max(height as f64 / source.h as f64)
    } else {
        (width as f64 / source.w as f64).min(height as f64 / source.h as f64)
    };
    let resized_width = ((source.w as f64 * scale).round() as u32).max(1);
    let resized_height = ((source.h as f64 * scale).round() as u32).max(1);
    // 双线性而非 Lanczos3：这条路径逐帧执行（视频背景每秒换几十次），
    // Lanczos 每帧要 32ms，是背景视频合成的七成耗时；暗化装饰层看不出差别。
    let resized = source.resize_bilinear(resized_width, resized_height);
    let left = (width as i64 - resized_width as i64) / 2;
    let top = (height as i64 - resized_height as i64) / 2;
    result.alpha_composite(&resized, left, top);
    // 暗化用整数查找表：逐像素 f64 循环在逐帧路径上同样不划算。
    let brightness = 1.0 - style.background_dim.clamp(0.0, 1.0);
    let table: [u8; 256] = std::array::from_fn(|value| (value as f64 * brightness).round() as u8);
    for pixel in result.data.chunks_exact_mut(4) {
        pixel[0] = table[pixel[0] as usize];
        pixel[1] = table[pixel[1] as usize];
        pixel[2] = table[pixel[2] as usize];
    }
    result
}

/// 将游戏区域帧居中放置到 16:9 背景画布，并在右上角绘制
///“当前 / 总时长”游戏时间标签；没有谱面背景时画布为黑色。
///
/// 故事板（启用时）层序与 osu! 一致：underlay（Background/Pass/Foreground）压在
/// 背景之上、物件层之下，只有 Overlay 层压在物件层之上、时间标签之下；全部层都
/// 与背景同吃 `BACKGROUND_DIM` 暗度（亮度预乘进精灵颜色）。`chart_ms` 是谱面
/// 绝对毫秒，`current_ms` 是进度标签用的显示时间。
#[allow(clippy::too_many_arguments)]
pub(crate) fn compose_frame(
    pf: Img,
    current_ms: i64,
    chart_ms: i64,
    total_ms: i64,
    out_w: u32,
    out_h: u32,
    background: Option<&Img>,
    style: VideoStyle,
    storyboard: Option<&MediaStoryboard>,
) -> Img {
    compose_frame_with_label(
        pf, current_ms, chart_ms, total_ms, out_w, out_h, background, style, storyboard, false,
    )
}

/// 回放 HUD 已含右下时间标签，这里只共享背景与故事板合成。
#[allow(clippy::too_many_arguments)]
pub(crate) fn compose_replay_frame(
    pf: Img,
    current_ms: i64,
    chart_ms: i64,
    total_ms: i64,
    out_w: u32,
    out_h: u32,
    background: Option<&Img>,
    style: VideoStyle,
    storyboard: Option<&MediaStoryboard>,
) -> Img {
    compose_frame_with_label(
        pf, current_ms, chart_ms, total_ms, out_w, out_h, background, style, storyboard, true,
    )
}

#[allow(clippy::too_many_arguments)]
fn compose_frame_with_label(
    pf: Img,
    current_ms: i64,
    chart_ms: i64,
    total_ms: i64,
    out_w: u32,
    out_h: u32,
    background: Option<&Img>,
    style: VideoStyle,
    storyboard: Option<&MediaStoryboard>,
    replay_hud: bool,
) -> Img {
    let mut canvas = background
        .cloned()
        .unwrap_or_else(|| Img::new(out_w, out_h, style.black_opaque));
    // 与背景图同一亮度：背景已按 (1 - BACKGROUND_DIM) 预暗化，故事板同倍率
    // 乘进精灵颜色后，与「整层压暗」的合成结果逐像素一致。
    let brightness = (1.0 - style.background_dim.clamp(0.0, 1.0)) as f32;
    // 故事板每帧只求值一次，两半分别画在物件层两侧：分两次求值会让另一半的
    // 状态结果直接丢弃（元素多的谱面上每帧白算一半的精灵状态）。
    let storyboard_draws = storyboard.map(|storyboard| storyboard.sprites_at(chart_ms));
    if let Some((storyboard, draws)) = storyboard.zip(storyboard_draws.as_ref()) {
        storyboard.draw_sprites(&mut canvas, &draws.0, brightness);
    }
    let ox = ((out_w - pf.w) / 2) as i64;
    let oy = ((out_h - pf.h) / 2) as i64;
    canvas.alpha_composite(&pf, ox, oy);
    if let Some((storyboard, draws)) = storyboard.zip(storyboard_draws.as_ref()) {
        storyboard.draw_sprites(&mut canvas, &draws.1, brightness);
    }

    // OSR 的右下时间标签与其他 HUD 一起使用原字体，避免视频层再叠一份位图文字。
    if !replay_hud {
        let label = format_progress_label(current_ms, total_ms);
        let (lw, _) = text_size(&label, style.label_font_size);
        let lx = out_w as i64 - lw as i64 - style.label_pad;
        draw_text(
            &mut canvas,
            lx,
            style.label_pad,
            &label,
            style.label_font_size,
            style.label_color,
        );
    }
    canvas
}

pub(crate) fn format_progress_label(current_ms: i64, total_ms: i64) -> String {
    format!("{}/{}", format_mmss(current_ms), format_mmss(total_ms))
}

fn format_mmss(ms: i64) -> String {
    crate::export::text::format_mmss_floor(ms)
}
