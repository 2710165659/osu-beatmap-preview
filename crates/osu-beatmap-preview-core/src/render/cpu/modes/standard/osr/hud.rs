//! 回放 HUD 布局；尺寸使用当前输出格式 SCALE，光标复用 Autoplay 实现。
use super::overlay::OverlayCanvas;
use super::*;
use crate::gameplay::state::ReplayStatus;
use crate::render::text::{format_mmss_floor, render_text_sprite, scaled_bitmap_font_height};

pub(super) fn display_rank(rank: Option<&str>, status: ReplayStatus) -> &str {
    if status == ReplayStatus::Failed {
        "F"
    } else {
        match rank {
            Some("X" | "XH") => "SS",
            Some("SH") => "S",
            Some(rank) => rank,
            None => "-",
        }
    }
}

pub(super) fn draw(
    image: &mut impl OverlayCanvas,
    context: &RenderContext,
    gameplay: &GameplayTimeline,
    visuals: &ReplayVisuals,
    time: f64,
    total: i64,
) {
    let scale = crate::render::geometry::output_scale(
        crate::render::geometry::GameMode::Standard,
        context.output_format,
    );
    let px = |value: f64| (value * scale).round() as i64;
    let text = |value: &str, size: f64, color: [u8; 4]| {
        visuals.font.as_ref().map_or_else(
            || {
                std::sync::Arc::new(render_text_sprite(
                    value,
                    scaled_bitmap_font_height(size as u32, scale),
                    color,
                ))
            },
            |font| font.sprite_arc(value, size * scale, 0.0, [color[0], color[1], color[2]]),
        )
    };
    let width = context.frame_layout.frame_width;
    let height = context.frame_layout.frame_height;
    let state = gameplay.state_at(time);
    let pink = [252, 97, 143, 255];
    let score = format!("{:07}", state.score);
    let score = text(&score, 36.0, [255; 4]);
    image.shared_sprite(&score, width - score.w as i64 - px(12.0), px(4.0));
    let accuracy = state
        .accuracy
        .map_or("--.--%".into(), |a| format!("{:.2}%", a * 100.0));
    let summary = format!(
        "{}  {}",
        display_rank(state.rank.as_deref(), state.status),
        accuracy
    );
    let summary = text(&summary, 21.0, [255; 4]);
    image.shared_sprite(&summary, width - summary.w as i64 - px(12.0), px(30.0));
    let combo = text(&format!("{}x", state.combo), 36.0, [255; 4]);
    image.shared_sprite(&combo, px(12.0), height - combo.h as i64 - px(8.0));
    let progress = text(
        &format!(
            "{}/{}",
            format_mmss_floor(context.time_axis.to_display(time as i64)),
            format_mmss_floor(context.time_axis.to_display(total))
        ),
        28.0,
        [232, 232, 232, 255],
    );
    image.shared_sprite(
        &progress,
        width - progress.w as i64 - px(12.0),
        height - progress.h as i64 - px(8.0),
    );
    if state.status == ReplayStatus::Failed {
        let label = text("REPLAY FAILED", 45.0, pink);
        let w = label.w;
        image.fill_rect_size(
            (width - w as i64) / 2 - px(16.0),
            px(66.0),
            w as i64 + px(32.0),
            px(42.0),
            [71, 18, 40, 235],
        );
        image.shared_sprite(&label, (width - w as i64) / 2, px(76.0));
    }
    if let Some(input) = gameplay.input_at(time) {
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
            let label = text(name, 18.0, [255; 4]);
            image.shared_sprite(
                &label,
                x + (px(34.0) - label.w as i64) / 2,
                y + (px(26.0) - label.h as i64) / 2,
            );
        }
        visuals.cursor.visit_trail(
            time as i64,
            context.frame_layout.scale,
            |from, to, width, alpha| {
                image.draw_line(
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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uses_ss_and_f_in_the_hud() {
        assert_eq!(display_rank(Some("X"), ReplayStatus::Completed), "SS");
        assert_eq!(display_rank(Some("S"), ReplayStatus::Failed), "F");
        for rank in ["S", "A", "B", "C", "D"] {
            assert_eq!(display_rank(Some(rank), ReplayStatus::Playing), rank);
        }
    }
}
