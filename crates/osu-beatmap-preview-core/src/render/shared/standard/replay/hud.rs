//! Standard 回放的光标与按键面板；基础 HUD 使用模式无关的绘制契约。
use super::overlay::OverlayCanvas;
use super::*;
use crate::render::shared::replay::hud::{self, HudLayout};

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    image: &mut impl OverlayCanvas,
    context: &RenderContext,
    gameplay: &GameplayTimeline,
    visuals: &ReplayVisuals,
    time: f64,
    total: i64,
    _od: f64,
) {
    let scale = context.frame_layout.output_scale();
    let width = context.frame_layout.frame_width;
    let height = context.frame_layout.frame_height;
    hud::draw(
        image,
        HudLayout {
            width,
            height,
            scale,
            time_axis: context.time_axis,
        },
        gameplay,
        &visuals.hud,
        time,
        total,
    );
    draw_input(
        image,
        context,
        gameplay,
        visuals,
        time,
        width,
        height,
        |v| (v * scale).round() as i64,
        scale,
        [252, 97, 143, 255],
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_input(
    image: &mut impl OverlayCanvas,
    context: &RenderContext,
    gameplay: &GameplayTimeline,
    visuals: &ReplayVisuals,
    time: f64,
    width: i64,
    height: i64,
    px: impl Fn(f64) -> i64,
    scale: f64,
    pink: [u8; 4],
) {
    let Some(input) = gameplay.input_at(time) else {
        return;
    };
    let text = |value: &str| hud::hud_text(&visuals.hud, value, 18.0, [255; 4], scale);
    for (i, (bit, name)) in [(1, "M1"), (2, "M2"), (4, "K1"), (8, "K2")]
        .iter()
        .enumerate()
    {
        let x = width - px(43.0);
        let y = height / 2 + px(i as f64 * 31.0 - 62.0);
        image.fill_rect_size(
            x,
            y,
            px(34.0),
            px(26.0),
            if input.keys & bit != 0 {
                pink
            } else {
                [27, 23, 39, 205]
            },
        );
        let label = text(name);
        image.shared_sprite(
            &label,
            x + (px(34.0) - label.w as i64) / 2,
            y + (px(26.0) - label.h as i64) / 2,
        );
    }
    draw_cursor(image, context, visuals, input, time);
}

fn draw_cursor(
    image: &mut impl OverlayCanvas,
    context: &RenderContext,
    visuals: &ReplayVisuals,
    input: crate::gameplay::InputSnapshot,
    time: f64,
) {
    visuals.cursor.visit_trail(
        time as i64,
        context.frame_layout.scale,
        |from, to, width, alpha| {
            image.blend_line(
                from[0],
                from[1],
                to[0],
                to[1],
                width,
                [255, 255, 255, alpha],
            );
        },
    );
    if let Some(cursor) = input.cursor {
        let (x, y) = to_frame_point(cursor[0] as f64, cursor[1] as f64, &context.frame_layout);
        if let Some(sprite) = visuals.cursor.sprite_at(time as i64) {
            image.shared_sprite(
                sprite,
                (x - sprite.w as f64 / 2.0).round() as i64,
                (y - sprite.h as f64 / 2.0).round() as i64,
            );
        }
    }
}
