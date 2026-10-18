//! 原生 Standard 回放场景；物件生命周期取判定时间，几何仍走共享 WebGPU 绘制器。
use super::*;
use crate::render::cpu::modes::standard::{
    context::build_replay_context,
    osr::{self, font::ArgonFont, ReplayVisuals},
};
use crate::{
    gameplay::timeline::GameplayTimeline, replay::preparation::standard::PreparedStandard,
};

pub(super) struct ReplayScene {
    prepared: Arc<PreparedStandard>,
    gameplay: Arc<GameplayTimeline>,
    visuals: ReplayVisuals,
    plan: crate::render::shared::standard::replay::ReplayRenderPlan,
    breaks: Vec<BreakPeriod>,
}
impl ReplayScene {
    pub(super) fn slider_data(&self, context: &RenderContext, id: usize) -> Arc<SliderRenderData> {
        Arc::clone(crate::render::shared::standard::replay::slider_data(
            context,
            &self.prepared,
            id,
        ))
    }
}

pub fn prepare(
    beatmap: &Beatmap,
    prepared: Arc<PreparedStandard>,
    gameplay: Arc<GameplayTimeline>,
    font: Option<ArgonFont>,
    dimensions: (u32, u32),
) -> Result<RealtimeFrameSource> {
    let mut context = build_replay_context(
        beatmap,
        &prepared,
        TimeAxis::new(prepared.objects[0].source.start_time),
        OutputFormat::Mp4,
    );
    let dx = (i64::from(dimensions.0) - context.frame_layout.frame_width) as f64 / 2.0;
    let dy = (i64::from(dimensions.1) - context.frame_layout.frame_height) as f64 / 2.0;
    context.frame_layout.playfield_left += dx;
    context.frame_layout.playfield_top += dy;
    context.frame_layout.frame_width = i64::from(dimensions.0);
    context.frame_layout.frame_height = i64::from(dimensions.1);
    let visuals = ReplayVisuals::new(
        &context,
        &gameplay,
        font,
        &beatmap.break_periods,
        prepared.od,
    );
    let plan = crate::render::shared::standard::replay::ReplayRenderPlan::new(
        &context, &prepared, &gameplay,
    );
    source_from_context(
        beatmap,
        context,
        Some(ReplayScene {
            prepared,
            gameplay,
            visuals,
            plan,
            breaks: beatmap.break_periods.clone(),
        }),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    context: &RenderContext,
    sliders: &[Option<PreparedSlider>],
    digits: &[Arc<Img>],
    breaks: &[PreparedBreak],
    follow: &PreparedFollowPoints,
    replay: &ReplayScene,
    time: i64,
) -> FrameScene {
    let gameplay = &replay.gameplay;
    let mut scene = FrameSceneBuilder::new(
        context.frame_layout.frame_width as u32,
        context.frame_layout.frame_height as u32,
        time,
    );
    replay
        .plan
        .frame(context, &replay.prepared, gameplay, time, &replay.breaks)
        .draw(&mut SceneReplayTarget {
            scene: &mut scene,
            context,
            sliders,
            digits,
            breaks,
            follow,
            time,
        });
    osr::draw_scene_overlay(
        &mut scene,
        context,
        &replay.prepared,
        gameplay,
        &replay.visuals,
        time,
    );
    scene.finish()
}

struct SceneReplayTarget<'a> {
    scene: &'a mut FrameSceneBuilder,
    context: &'a RenderContext,
    sliders: &'a [Option<PreparedSlider>],
    digits: &'a [Arc<Img>],
    breaks: &'a [PreparedBreak],
    follow: &'a PreparedFollowPoints,
    time: i64,
}

impl crate::render::shared::standard::replay::ReplayDrawTarget for SceneReplayTarget<'_> {
    fn follow_points(&mut self) {
        draw_follow_points(self.scene, self.context, self.follow, self.time);
    }
    fn circle(&mut self, id: usize, position: (f64, f64), opacity: f64) {
        draw_circle_piece(
            self.scene,
            self.context,
            self.digits,
            to_frame_point(position.0, position.1, &self.context.frame_layout),
            self.context.combo_info[id].color,
            opacity,
            self.context.combo_info[id].number,
        );
    }
    fn slider(&mut self, id: usize, head_opacity: Option<f64>, tracking: bool) {
        draw_slider_state(
            self.scene,
            self.context,
            self.sliders[id].as_ref().unwrap(),
            self.digits,
            id,
            self.time,
            head_opacity,
            tracking,
        );
    }
    fn spinner(&mut self, id: usize, rotation: Option<(f64, f64, f64)>) {
        crate::render::cpu::modes::standard::spinner::draw_gpu_state(
            self.scene,
            self.context,
            &self.context.hit_objects[id],
            self.time,
            rotation,
        );
    }
    fn approach_circle(&mut self, id: usize) {
        draw_approach_circle(self.scene, self.context, id, self.time);
    }
    fn break_overlay(&mut self, index: usize, _period: &BreakPeriod) {
        draw_break(self.scene, self.context, &self.breaks[index], self.time);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::events::HitResult;
    use crate::render::scene::DrawCommand;

    #[test]
    fn hidden_scene_does_not_reintroduce_judged_circle_or_slider_head() {
        for object in ["100,100,1000,1,0", "100,100,1000,2,0,L|300:100,1,200"] {
            for (mods, visible) in [
                (serde_json::json!([]), true),
                (serde_json::json!([{"acronym":"HD"}]), false),
                (
                    serde_json::json!([{"acronym":"HD","settings":{"only_fade_approach_circles":true}}]),
                    true,
                ),
            ] {
                let raw = format!("osu file format v14\n[General]\nMode:0\n[Difficulty]\nCircleSize:5\nOverallDifficulty:5\nSliderMultiplier:1.4\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n{object}\n");
                let beatmap = crate::parse_beatmap_bytes(raw.as_bytes()).unwrap();
                let mut document = crate::replay::parse_osr(include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../osu-beatmap-preview-web/test/fixtures/replay-standard.osr"
                )))
                .unwrap();
                document.header.version = 30_000_016;
                document.metadata =
                    Some(serde_json::from_value(serde_json::json!({"mods":mods})).unwrap());
                let prepared = Arc::new(
                    crate::replay::preparation::standard::prepare_replay(&beatmap, &document)
                        .unwrap(),
                );
                let mut gameplay =
                    crate::gameplay::engine::compile_standard(&prepared, Arc::new(document))
                        .unwrap();
                gameplay.failure_time = None;
                gameplay.objects[0].head = Some((1000.0, HitResult::Great));
                let context = build_replay_context(
                    &beatmap,
                    &prepared,
                    TimeAxis::new(1000),
                    OutputFormat::Mp4,
                );
                let center = to_frame_point(100.0, 100.0, &context.frame_layout);
                let radius = context.frame_circle_diameter as f32 / 2.0;
                let source = prepare(
                    &beatmap,
                    prepared,
                    Arc::new(gameplay),
                    None,
                    (
                        context.frame_layout.frame_width as u32,
                        context.frame_layout.frame_height as u32,
                    ),
                )
                .unwrap();
                // 检查物件的白色外环，HUD、光标和判定粒子不属于 note 主体。
                for time in [1000, 1001, 1040, 1080, 1119] {
                    let scene = source.render(time).unwrap();
                    let has_head = scene.commands.iter().any(|command| matches!(command,
                        DrawCommand::Ring { center: point, radius: r, color, .. }
                        if *point == [center.0 as f32, center.1 as f32] && *r == radius && color[3] > 0
                    ));
                    assert_eq!(
                        has_head, visible,
                        "WebGPU 判定后可见性错误：{object}/{time}"
                    );
                }
            }
        }
    }
}
