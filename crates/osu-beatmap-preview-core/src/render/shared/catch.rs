//! Catch 回放共享画面：真实接盘、堆叠与拖尾，复用主模式 HUD 和按键布局。
use super::replay::{
    feedback::{self, FeedbackVisuals},
    font::ArgonFont,
    hud::{self, HudLayout, HudVisuals},
    overlay::OverlayCanvas,
};
use crate::render::{
    canvas::Img,
    cpu::modes::catch::{
        animation::{self, AnimationLayout},
        catcher::CatcherPath,
        caught::CatcherDisplay,
        constants::*,
        objects::RenderObject,
    },
    geometry::{OutputFormat, PixelRect},
    scene::FrameSceneBuilder,
    visibility::{Flashlight, FlashlightKind, VisibilityTimeline},
};
use crate::{
    gameplay::timeline::GameplayTimeline, replay::preparation::catch::PreparedCatch, Result,
};
use std::sync::Arc;

pub struct ReplayRenderPlan {
    pub layout: AnimationLayout,
    prepared: Arc<PreparedCatch>,
    gameplay: Arc<GameplayTimeline>,
    objects: Vec<RenderObject>,
    times: Vec<i64>,
    path: Arc<CatcherPath>,
    catcher: CatcherDisplay,
    flashlight: Option<VisibilityTimeline>,
    hud: HudVisuals,
    feedback: FeedbackVisuals,
    misses: Vec<(f64, f64, Arc<Img>)>,
}
impl ReplayRenderPlan {
    pub fn new(
        prepared: Arc<PreparedCatch>,
        gameplay: Arc<GameplayTimeline>,
        font: Option<ArgonFont>,
        dimensions: Option<(u32, u32)>,
    ) -> Result<Self> {
        Self::with_format(prepared, gameplay, font, dimensions, OutputFormat::Mp4)
    }
    pub fn with_format(
        prepared: Arc<PreparedCatch>,
        gameplay: Arc<GameplayTimeline>,
        font: Option<ArgonFont>,
        dimensions: Option<(u32, u32)>,
        format: OutputFormat,
    ) -> Result<Self> {
        let mut layout = if format == OutputFormat::Gif {
            animation::build_animation_layout(prepared.cs, prepared.ar, format)
        } else {
            animation::build_video_animation_layout(prepared.cs, prepared.ar, format)
        }
        .with_hidden_kiai(&prepared.beatmap, Some(&prepared.mods.settings));
        if let Some((width, height)) = dimensions {
            let dx = (i64::from(width) - layout.frame_width) / 2;
            let dy = (i64::from(height) - layout.frame_height) / 2;
            layout.playfield_left += dx as f64;
            layout.playfield_top += dy as f64;
            layout.content.x += dx;
            layout.content.y += dy;
            layout.frame_width = width.into();
            layout.frame_height = height.into();
        }
        let path = Arc::new(CatcherPath::from_replay(&prepared, &gameplay));
        let catcher = CatcherDisplay::new(&prepared.objects, Arc::clone(&path), prepared.cs);
        let mut objects = prepared.objects.clone();
        objects.sort_by_key(|o| std::cmp::Reverse(o.start_time));
        let times = objects.iter().map(|o| o.start_time).collect();
        let flashlight =
            prepared.mods.settings.flashlight.then(|| {
                VisibilityTimeline::from_replay(&gameplay, &prepared.beatmap.break_periods)
            });
        let hud = HudVisuals::new(&gameplay, font.clone(), layout.render_scale, [1.0; 3]);
        let feedback = FeedbackVisuals::new(layout.render_scale, font.as_ref());
        let mut sprites = std::collections::HashMap::new();
        let misses = gameplay
            .events
            .iter()
            .filter(|event| {
                event.kind == crate::gameplay::events::EventKind::Judgement
                    && !event.result.is_hit()
            })
            .map(|event| {
                let object = &prepared.objects[event.object_id];
                let key = (object.object_type as u8, object.color, object.hyper_dash);
                let sprite = Arc::clone(sprites.entry(key).or_insert_with(|| {
                    let diameter = crate::render::cpu::modes::catch::drawing::object_diameter(
                        layout.object_scale,
                        layout.playfield_scale,
                        object.scale_factor,
                    );
                    let side =
                        (diameter * if object.hyper_dash { 1.6 } else { 1.0 }).ceil() as u32 + 4;
                    let mut sprite = Img::new(side, side, [0; 4]);
                    crate::render::cpu::modes::catch::drawing::draw_catch_object(
                        &mut sprite,
                        object,
                        f64::from(side) / 2.0,
                        f64::from(side) / 2.0,
                        diameter,
                    );
                    Arc::new(sprite)
                }));
                (event.time, object.x, sprite)
            })
            .collect();
        Ok(Self {
            layout,
            prepared,
            gameplay,
            objects,
            times,
            path,
            catcher,
            flashlight,
            hud,
            feedback,
            misses,
        })
    }
    pub fn draw_frame(&self, image: &mut Img, time: i64) {
        *image = animation::render_animation_frame(
            &self.objects,
            &self.times,
            time,
            &self.layout,
            Some(image),
            Some(&self.catcher),
        );
    }
    pub fn draw_scene(&self, scene: &mut FrameSceneBuilder, time: i64) {
        crate::render::wgpu::modes::catch::draw_scene(
            scene,
            &self.objects,
            &self.times,
            time,
            &self.layout,
            None,
            Some(&self.catcher),
        );
    }
    pub fn draw_overlay(&self, image: &mut Img, time: i64) {
        self.overlay_to(image, time);
    }
    pub fn draw_scene_overlay(&self, scene: &mut FrameSceneBuilder, time: i64) {
        self.overlay_to(scene, time);
    }
    fn overlay_to(&self, target: &mut impl OverlayCanvas, time: i64) {
        let layout = &self.layout;
        let scale = layout.render_scale;
        // 漏接物件继续下落并在 250ms 内淡出；按事件时刻计算，不依赖上一帧。
        let lo = self
            .misses
            .partition_point(|(at, _, _)| *at <= time as f64 - 250.0);
        let hi = self.misses.partition_point(|(at, _, _)| *at <= time as f64);
        for (at, x, sprite) in &self.misses[lo..hi] {
            let age = time as f64 - at;
            let center = [
                layout.playfield_left + x * layout.playfield_scale,
                layout.playfield_top
                    + STABLE_CATCHER_Y * layout.playfield_scale
                    + age * layout.pixels_per_ms,
            ];
            target.sprite_opacity(
                sprite,
                (center[0] - f64::from(sprite.w) / 2.0).round() as i64,
                (center[1] - f64::from(sprite.h) / 2.0).round() as i64,
                1.0 - age / 250.0,
            );
        }
        for event in self
            .gameplay
            .events_between(time as f64 - 800.0, time as f64)
        {
            if event.kind != crate::gameplay::events::EventKind::Judgement
                || event.result.is_hit()
                || event.maximum == crate::gameplay::events::HitResult::LargeBonus
            {
                continue;
            }
            let object = &self.prepared.objects[event.object_id];
            let age = time as f64 - event.time;
            let position = [
                layout.playfield_left + object.x * layout.playfield_scale,
                layout.playfield_top + STABLE_CATCHER_Y * layout.playfield_scale - 20.0 * scale,
            ];
            if event.result == crate::gameplay::events::HitResult::Miss {
                feedback::draw_judgement(
                    target,
                    &self.feedback,
                    event.result,
                    event.id,
                    position,
                    scale,
                    age,
                );
            } else if age < 600.0 {
                target.additive_transformed(
                    &self.feedback.tick_misses[1],
                    position,
                    1.0,
                    0.0,
                    1.0 - age / 600.0,
                );
            }
        }
        if let Some(timeline) = &self.flashlight {
            let mask = Flashlight {
                center: [
                    layout.playfield_left + self.path.position_at(time) * layout.playfield_scale,
                    layout.playfield_top + STABLE_CATCHER_Y * layout.playfield_scale,
                ],
                radius: 203.125
                    * layout.playfield_scale
                    * timeline.flashlight_scale(FlashlightKind::Catch, time),
                band: false,
                dim: 0.0,
            };
            target.flashlight(
                &mask,
                PixelRect {
                    x: 0,
                    y: 0,
                    width: layout.frame_width,
                    height: layout.frame_height,
                },
            );
        }
        let hud_layout = HudLayout {
            width: layout.frame_width,
            height: layout.frame_height,
            scale,
            time_axis: crate::processing::timeline::TimeAxis::new(self.prepared.first),
        };
        hud::draw(
            target,
            hud_layout,
            &self.gameplay,
            &self.hud,
            time as f64,
            self.prepared
                .last_end
                .max(self.gameplay.settlement_time)
                .ceil() as i64,
        );
        let input = self.gameplay.input_at(time as f64);
        let frames = &self.gameplay.document.frames;
        let index = frames.partition_point(|f| f.time_ms as f64 <= time as f64);
        let direction = if index > 0
            && index < frames.len()
            && input.is_some_and(|_| {
                self.gameplay.state_at(time as f64).status
                    == crate::gameplay::state::ReplayStatus::Playing
            }) {
            frames[index].cursor[0] - frames[index - 1].cursor[0]
        } else {
            0.0
        };
        hud::draw_keys(
            target,
            &self.hud,
            hud_layout,
            &[
                (direction < 0.0, "L"),
                (direction > 0.0, "R"),
                (input.is_some_and(|s| s.keys == 1), "D"),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        render::scene::DrawCommand,
        replay::{parse_osr, preparation::catch::prepare_replay},
    };

    #[test]
    fn replay_cpu_and_gpu_share_keys_effects_and_fractional_format_scales() {
        let mut config = crate::config::CoreConfig::default();
        config.render.catch.gif.SCALE = 0.75;
        config.render.catch.mp4.SCALE = 1.25;
        config.render.standard.mp4.SCALE = 2.0;
        crate::config::with_config(Arc::new(config), || {
            let map = crate::parse_beatmap_bytes(include_bytes!(
                "../../../../osu-beatmap-preview-web/test/fixtures/replay-catch.osu"
            ))
            .unwrap();
            let document = Arc::new(
                parse_osr(include_bytes!(
                    "../../../../osu-beatmap-preview-web/test/fixtures/replay-catch.osr"
                ))
                .unwrap(),
            );
            let prepared = Arc::new(prepare_replay(&map, &document).unwrap());
            let gameplay =
                Arc::new(crate::gameplay::engine::compile_catch(&prepared, document).unwrap());
            for (format, scale) in [(OutputFormat::Gif, 0.75), (OutputFormat::Mp4, 1.25)] {
                let plan = ReplayRenderPlan::with_format(
                    Arc::clone(&prepared),
                    Arc::clone(&gameplay),
                    None,
                    None,
                    format,
                )
                .unwrap();
                assert_eq!(plan.layout.render_scale, scale);
                let mut image = Img::new(
                    plan.layout.frame_width as u32,
                    plan.layout.frame_height as u32,
                    [10, 20, 30, 255],
                );
                plan.draw_frame(&mut image, 1400);
                assert_eq!(image.get(0, 0), [10, 20, 30, 255]);
                plan.draw_overlay(&mut image, 1400);
                let x = plan.layout.frame_width - (43.0 * scale).round() as i64;
                let y = plan.layout.frame_height / 2 + (15.5 * scale).round() as i64;
                assert_eq!(
                    image.get((x + 1) as u32, (y + 1) as u32),
                    [252, 97, 143, 255]
                );
                let frame = |time| {
                    let mut scene = FrameSceneBuilder::new(image.w, image.h, time);
                    plan.draw_scene(&mut scene, time);
                    plan.draw_scene_overlay(&mut scene, time);
                    scene.finish()
                };
                let before = frame(1400);
                frame(3800);
                assert_eq!(
                    format!("{:?}", before.commands),
                    format!("{:?}", frame(1400).commands)
                );
                assert!(before.commands.iter().any(|c| matches!(c,DrawCommand::Rectangle { color, .. } if *color == [252,97,143,255])));
                assert!(before
                    .commands
                    .iter()
                    .any(|c| matches!(c, DrawCommand::Circle { .. } | DrawCommand::Ring { .. })));
                assert!(before
                    .resources
                    .values()
                    .all(|image| image.w < before.size.width || image.h < before.size.height));
            }
        });
    }
}
