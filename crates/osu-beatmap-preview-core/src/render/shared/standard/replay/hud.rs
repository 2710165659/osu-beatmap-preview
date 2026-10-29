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
        image, context, gameplay, visuals, time, width, height, scale,
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
    scale: f64,
) {
    let Some(input) = gameplay.input_at(time) else {
        return;
    };
    hud::draw_keys(
        image,
        &visuals.hud,
        HudLayout {
            width,
            height,
            scale,
            time_axis: context.time_axis,
        },
        &[
            (input.keys & 1 != 0, "M1"),
            (input.keys & 2 != 0, "M2"),
            (input.keys & 4 != 0, "K1"),
            (input.keys & 8 != 0, "K2"),
        ],
    );
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
