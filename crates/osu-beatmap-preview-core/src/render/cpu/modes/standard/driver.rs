//! 普通预览帧组装；回放在独立模块消费同一批物件绘制器。
use super::context::{py_round, RenderCache, RenderContext};
use super::frame::*;
use crate::model::BreakPeriod;
use crate::render::canvas::Img;
pub fn render_frame(
    context: &RenderContext,
    cache: &mut RenderCache,
    snapshot_time: i64,
    break_periods: &[BreakPeriod],
    visible_indexes: &[usize],
    background: Option<&Img>,
) -> Img {
    cache.prepare_for_context(context);
    let mut frame = background_frame(context, background);

    // 跟随点在游戏里位于物件层之下（`OsuPlayfield` 的 FollowPoints 早于
    // HitObjectContainer），因此最先绘制，会被随后画出的物件覆盖。
    draw_follow_points(&mut frame, context, cache, snapshot_time);

    for &index in visible_indexes {
        let hit_object = &context.hit_objects[index];
        if hit_object.hit_type & 8 != 0 {
            draw_spinner(&mut frame, context, cache, hit_object, snapshot_time);
        } else if hit_object.hit_type & 2 != 0 {
            draw_slider(&mut frame, context, cache, index, snapshot_time);
        } else {
            draw_hit_circle(&mut frame, context, cache, index, snapshot_time);
        }
    }

    for &index in visible_indexes {
        let hit_object = &context.hit_objects[index];
        if hit_object.hit_type & 8 == 0 {
            draw_approach_circle(
                &mut frame,
                context,
                cache,
                index,
                context.combo_info[index].color,
                snapshot_time,
            );
        }
    }

    if let Some(flashlight) = &context.flashlight {
        let state = flashlight.get(context);
        state.at(context, snapshot_time).apply(
            &mut frame,
            crate::render::geometry::PixelRect {
                x: 0,
                y: 0,
                width: context.frame_layout.frame_width,
                height: context.frame_layout.frame_height,
            },
        );
    }

    if let Some(current_break) = current_break_period(break_periods, snapshot_time) {
        draw_break_overlay(&mut frame, current_break, snapshot_time, context);
    }

    if context.show_cursor {
        if let Some(cache) = &context.autoplay {
            let cursor = cache.get(context);
            cursor.visit_trail(
                snapshot_time,
                context.frame_layout.scale,
                |from, to, width, alpha| {
                    frame.draw_line(
                        from[0],
                        from[1],
                        to[0],
                        to[1],
                        width,
                        [255, 255, 255, alpha],
                    );
                },
            );
            if let (Some(position), Some(sprite)) = (
                cursor.position_at(snapshot_time),
                cursor.sprite_at(snapshot_time),
            ) {
                frame.alpha_composite(
                    sprite,
                    py_round(position[0] - sprite.w as f64 / 2.0),
                    py_round(position[1] - sprite.h as f64 / 2.0),
                );
            }
        }
    }

    frame
}

pub(super) fn background_frame(context: &RenderContext, background: Option<&Img>) -> Img {
    match background {
        Some(background) => background.clone(),
        None => {
            let color = context.style.background_color;
            // 视频物件层的帧是最终画布，底色只填内容框：内容框之外的补边仍由
            // 合成阶段用画布底色处理，物件则可以溢出到画布边缘而不被切掉。
            let content = context.frame_layout.content;
            let mut frame = Img::new(
                context.frame_layout.frame_width as u32,
                context.frame_layout.frame_height as u32,
                [0, 0, 0, 0],
            );
            frame.fill_rect_size(content.x, content.y, content.width, content.height, color);
            frame
        }
    }
}
