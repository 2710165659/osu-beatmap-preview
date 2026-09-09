//! Catch WGPU 实时帧源准备。

use crate::domain::errors::{PreviewError, Result};
use crate::domain::models::Beatmap;
use crate::domain::mods::ModSettings;
use crate::render::cpu::modes::catch::animation::{build_animation_layout, AnimationLayout};
use crate::render::cpu::modes::catch::drawing::object_diameter;
use crate::render::cpu::modes::catch::objects::{
    build_catch_render_objects, effective_difficulty, ObjType, RenderObject,
};
use crate::render::geometry::{GameMode, OutputFormat};
use crate::render::scene::{FrameSceneBuilder, SceneRect};
use crate::render::wgpu::RealtimeFrameSource;

/// 将 Catch 物件排序和难度换算固定到会话加载阶段。
pub fn prepare_realtime(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
) -> Result<RealtimeFrameSource> {
    let hit_objects = match beatmap.hit_objects.as_catch() {
        Some(objects) if !objects.is_empty() => objects,
        _ => return Err(PreviewError::render("catch beatmap has no hit objects")),
    };
    let difficulty = effective_difficulty(beatmap, mods);
    let mut render_objects =
        build_catch_render_objects(beatmap, hit_objects, mods, &difficulty, false)?;
    let layout = build_animation_layout(difficulty.cs, difficulty.ar, OutputFormat::Mp4);
    render_objects.sort_by_key(|object| std::cmp::Reverse(object.start_time));
    let start_times = render_objects
        .iter()
        .map(|object| object.start_time)
        .collect::<Vec<_>>();
    Ok(RealtimeFrameSource::new(
        GameMode::Catch,
        move |absolute_time_ms| {
            Ok(render_scene(
                &render_objects,
                &start_times,
                absolute_time_ms,
                &layout,
            ))
        },
    ))
}

fn render_scene(
    render_objects: &[RenderObject],
    start_times: &[i64],
    absolute_time_ms: i64,
    layout: &AnimationLayout,
) -> crate::render::scene::FrameScene {
    let mut scene = FrameSceneBuilder::new(
        layout.frame_width as u32,
        layout.frame_height as u32,
        absolute_time_ms,
    );
    // 背景由固定画布合成阶段提供，透明占位保持空帧也有稳定命令序列。
    scene.rectangle(
        SceneRect {
            x: 0.0,
            y: 0.0,
            width: layout.frame_width as f32,
            height: layout.frame_height as f32,
        },
        [0, 0, 0, 0],
    );
    let playfield_right = layout.playfield_left
        + crate::render::cpu::modes::catch::constants::PLAYFIELD_WIDTH * layout.playfield_scale;
    let judgement_y = layout.playfield_top
        + crate::render::cpu::modes::catch::constants::STABLE_CATCHER_Y * layout.playfield_scale;
    let line_height = crate::render::geometry::scale_stroke_px(2.0, layout.render_scale);
    scene.rectangle(
        SceneRect {
            x: layout.playfield_left.round() as f32,
            y: judgement_y.round() as f32,
            width: (playfield_right.round() - layout.playfield_left.round()).max(0.0) as f32,
            height: line_height as f32,
        },
        layout.judgement_line_color,
    );

    let fall_window = (layout.frame_height as f64 / layout.pixels_per_ms).ceil() as i64 + 2000;
    let lo = start_times.partition_point(|&time| time > absolute_time_ms + fall_window);
    let hi = start_times.partition_point(|&time| time >= absolute_time_ms - 2000);
    for object in &render_objects[lo..hi] {
        draw_object_scene(&mut scene, object, absolute_time_ms, judgement_y, layout);
    }
    scene.finish()
}

fn draw_object_scene(
    scene: &mut FrameSceneBuilder,
    object: &RenderObject,
    absolute_time_ms: i64,
    judgement_y: f64,
    layout: &AnimationLayout,
) {
    let local_time = object.start_time - absolute_time_ms;
    let center = [
        (layout.playfield_left + object.x * layout.playfield_scale) as f32,
        (judgement_y - local_time as f64 * layout.pixels_per_ms) as f32,
    ];
    let diameter = object_diameter(
        layout.object_scale,
        layout.playfield_scale,
        object.scale_factor,
    ) as f32;
    if center[1] + diameter / 2.0 < 0.0 || center[1] - diameter / 2.0 > judgement_y as f32 {
        return;
    }
    let radius = diameter / 2.0;
    let color = [object.color[0], object.color[1], object.color[2], 255];
    match object.object_type {
        ObjType::Fruit | ObjType::Droplet | ObjType::TinyDroplet => {
            if object.hyper_dash {
                let hyper = crate::config::current().skin.HYPER_DASH;
                scene.ring(
                    center,
                    radius * 1.6,
                    radius * 0.6,
                    [hyper[0], hyper[1], hyper[2], 255],
                );
            }
            scene.circle(center, radius, color);
            scene.ring(center, radius, radius * 0.2, [255, 255, 255, 255]);
        }
        ObjType::Banana => scene.ring(center, radius, radius * 0.2, color),
    }
}
