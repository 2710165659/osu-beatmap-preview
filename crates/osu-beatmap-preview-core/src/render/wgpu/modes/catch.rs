//! Catch WGPU 实时帧源准备。

use crate::domain::errors::{PreviewError, Result};
use crate::domain::models::Beatmap;
use crate::domain::mods::ModSettings;
use crate::render::cpu::modes::catch::animation::{
    build_video_animation_layout, object_alpha, AnimationLayout, CatchFlashlight,
};
use crate::render::cpu::modes::catch::caught::CatcherDisplay;
use crate::render::cpu::modes::catch::drawing::{draw_catch_object_scene, object_diameter};
use crate::render::cpu::modes::catch::objects::{
    build_catch_render_objects, effective_difficulty, RenderObject,
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
    let show_catcher = crate::config::current().render.catch.mp4.style.SHOW_CATCHER;
    let catcher = (show_catcher || mods.is_some_and(|m| m.flashlight)).then(|| {
        std::sync::Arc::new(
            crate::render::cpu::modes::catch::catcher::build_catcher_path(
                &render_objects,
                difficulty.cs,
            ),
        )
    });
    let flashlight = mods.is_some_and(|mods| mods.flashlight).then(|| {
        CatchFlashlight::new(
            &render_objects,
            &beatmap.break_periods,
            catcher.as_ref().unwrap().clone(),
        )
    });
    let catcher = catcher
        .filter(|_| show_catcher)
        .map(|path| CatcherDisplay::new(&render_objects, path, difficulty.cs));
    // 与 MP4 导出用同一套画布布局：物件层就是 16:9 画布本身，
    // 否则 FL 遮罩只盖住 playfield，合成阶段补出的背景不会被压暗。
    let layout = build_video_animation_layout(difficulty.cs, difficulty.ar, OutputFormat::Mp4)
        .with_hidden_kiai(beatmap, mods);
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
                flashlight.as_ref(),
                catcher.as_ref(),
            ))
        },
    ))
}

fn render_scene(
    render_objects: &[RenderObject],
    start_times: &[i64],
    absolute_time_ms: i64,
    layout: &AnimationLayout,
    flashlight: Option<&CatchFlashlight>,
    catcher: Option<&CatcherDisplay>,
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

    if let Some(catcher) = catcher {
        catcher.draw_scene(&mut scene, absolute_time_ms, layout);
    }
    let fall_window = (layout.frame_height as f64 / layout.pixels_per_ms).ceil() as i64 + 2000;
    let lo = start_times.partition_point(|&time| time > absolute_time_ms + fall_window);
    let hi = start_times.partition_point(|&time| time >= absolute_time_ms - 2000);
    let kiai_active = layout
        .hidden_kiai
        .as_ref()
        .is_some_and(|timeline| timeline.is_active_at(absolute_time_ms));
    for object in &render_objects[lo..hi] {
        if catcher.is_some_and(|catcher| catcher.has_caught(object, absolute_time_ms)) {
            continue;
        }
        draw_object_scene(
            &mut scene,
            object,
            absolute_time_ms,
            judgement_y,
            layout,
            kiai_active,
        );
    }
    if let Some(flashlight) = flashlight {
        flashlight.at(absolute_time_ms, layout).draw_scene(
            &mut scene,
            crate::render::geometry::PixelRect {
                x: 0,
                y: 0,
                width: layout.frame_width,
                height: layout.frame_height,
            },
        );
    }
    scene.finish()
}

fn draw_object_scene(
    scene: &mut FrameSceneBuilder,
    object: &RenderObject,
    absolute_time_ms: i64,
    judgement_y: f64,
    layout: &AnimationLayout,
    kiai_active: bool,
) {
    let local_time = object.start_time - absolute_time_ms;
    let center = [
        layout.playfield_left + object.x * layout.playfield_scale,
        judgement_y - local_time as f64 * layout.pixels_per_ms,
    ];
    let diameter = object_diameter(
        layout.object_scale,
        layout.playfield_scale,
        object.scale_factor,
    );
    if center[1] + diameter / 2.0 < 0.0 || center[1] - diameter / 2.0 > judgement_y {
        return;
    }
    let alpha = object_alpha(object, absolute_time_ms, layout.time_preempt, kiai_active);
    draw_catch_object_scene(scene, object, center, diameter, alpha);
}
