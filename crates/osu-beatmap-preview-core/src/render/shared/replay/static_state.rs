//! 静态回放共用的 ACC 文字与评级徽标，保持各模式的颜色及字形一致。

use crate::{
    gameplay::state::GameplayState,
    processing::parse::round_half_even,
    render::{
        canvas::Img,
        text::{draw_text, text_size},
    },
};

const WHITE: [u8; 4] = [255, 255, 255, 255];
const MARKER_BACKGROUND: [u8; 4] = [20, 8, 14, 255];

// 与 osu.Game/Graphics/OsuColour.ForRank 一致，评级字母继续复用共享 HUD 的 SS／失败 F 映射。
pub(crate) fn rank_color(rank: &str) -> [u8; 4] {
    match rank {
        "SS" => [222, 49, 174, 255],
        "S" => [2, 181, 195, 255],
        "A" => [136, 218, 32, 255],
        "B" => [227, 177, 48, 255],
        "C" => [255, 142, 93, 255],
        "D" => [255, 90, 90, 255],
        _ => [63, 63, 63, 255],
    }
}

fn draw_state_text(image: &mut Img, x: i64, y: i64, label: &str, font: u32, color: [u8; 4]) {
    // 完整字形只缩放一次，保留小数倍率下的笔画空隙；快照数量有上限，不为整图分配文字图层。
    let (width, height) = text_size(label, 8);
    let mut glyphs = Img::new(width, height, [0; 4]);
    draw_text(&mut glyphs, 0, 0, label, 8, color);
    let target_width = round_half_even(f64::from(width) * f64::from(font) / 8.0).max(1) as u32;
    for dy in 0..font {
        for dx in 0..target_width {
            image.blend_px(
                x + i64::from(dx),
                y + i64::from(dy),
                glyphs.get(dx * width / target_width, dy * height / font),
            );
        }
    }
}

pub(crate) fn badge_width(state: &GameplayState, font: u32, padding: i64) -> i64 {
    let label = format!("{:.2}%", state.accuracy.unwrap_or(1.0) * 100.0);
    let rank = super::hud::display_rank(state.rank.as_deref(), state.status);
    i64::from(text_size(&label, font).0 + text_size(rank, font).0) + padding * 6
}

pub(crate) fn draw_badge(
    image: &mut Img,
    x: i64,
    label_y: i64,
    state: &GameplayState,
    font: u32,
    padding: i64,
    scale: f64,
) {
    let label = format!("{:.2}%", state.accuracy.unwrap_or(1.0) * 100.0);
    let rank = super::hud::display_rank(state.rank.as_deref(), state.status);
    let rank_color = rank_color(rank);
    let height = i64::from(font) + padding * 2;
    image.fill_rect_size(
        x,
        label_y - padding,
        (3.0 * scale).round().max(1.0) as i64,
        height,
        rank_color,
    );
    draw_state_text(image, x + padding * 2, label_y, &label, font, rank_color);
    let badge_x = x + padding * 3 + i64::from(text_size(&label, font).0);
    image.fill_rect_size(
        badge_x,
        label_y - padding,
        i64::from(text_size(rank, font).0) + padding * 2,
        height,
        rank_color,
    );
    draw_state_text(
        image,
        badge_x + padding,
        label_y,
        rank,
        font,
        if matches!(rank, "F" | "-") {
            WHITE
        } else {
            MARKER_BACKGROUND
        },
    );
}
