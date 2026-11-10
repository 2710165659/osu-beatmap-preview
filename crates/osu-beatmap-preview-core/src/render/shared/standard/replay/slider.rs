//! 将回放准备结果映射到当前布局；逻辑路径与 tick 不在绘制层重新生成。

use crate::domain::shared::slider_path::{simplify_timed_path, SliderPath};
use crate::render::cpu::modes::standard::{
    context::{to_frame_point, RenderContext},
    slider::{SliderRenderData, SliderTickRenderData},
};
use crate::replay::preparation::standard::{NodeKind, PreparedObject, PreparedStandard};

pub(crate) fn data<'a>(
    context: &'a RenderContext,
    prepared: &PreparedStandard,
    id: usize,
) -> &'a std::sync::Arc<SliderRenderData> {
    context
        .body_layers
        .slider_data(id, || prepare(context, &prepared.objects[id]))
}

pub(super) fn prepare(context: &RenderContext, object: &PreparedObject) -> SliderRenderData {
    let path = object
        .path
        .as_ref()
        .expect("回放滑条必须具有准备好的逻辑路径");
    let layout = &context.frame_layout;
    let offset = f64::from(object.position[0] - object.source.x as f32);
    let transform = |path: &SliderPath| SliderPath {
        points: path
            .points
            .iter()
            .map(|&(x, y)| to_frame_point(x + offset, y + offset, layout))
            .collect(),
        cumulative_lengths: path
            .cumulative_lengths
            .iter()
            .map(|length| length * layout.scale)
            .collect(),
        total_length: path.total_length * layout.scale,
    };
    let ticks = object
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Tick)
        .map(|node| {
            let span = ((node.time - object.source.start_time as f64) / object.span).floor();
            let span_start = object.source.start_time as f64 + span * object.span;
            let position = object.position_at(node.time);
            SliderTickRenderData {
                center: to_frame_point(f64::from(position[0]), f64::from(position[1]), layout),
                time: node.time,
                time_preempt: ((node.time - span_start) / 2.0
                    + if span > 0.0 {
                        200.0
                    } else {
                        context.settings.preempt_ms as f64 * 0.66
                    })
                .max(0.0),
            }
        })
        .collect();
    SliderRenderData::from_paths(
        transform(&simplify_timed_path(path, 1.0)),
        transform(path),
        ticks,
        object.source.slider_repeats,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn replay_uses_preview_simplification_and_keeps_full_motion_at_each_scale() {
        use crate::domain::shared::slider_path::{build_standard_slider_paths, path_position_at};
        use crate::render::geometry::OutputFormat;

        let dense = (0..2048)
            .map(|index| format!("|{}:100", 200 + index % 2))
            .collect::<String>();
        for curve in [
            "P|200:200|300:100".to_owned(),
            format!("L|200:100{dense}|300:100"),
        ] {
            let (beatmap, prepared, _, _) = super::super::tests::object_mod_fixture(
                serde_json::json!([]),
                &format!("100,100,1000,2,0,{curve},1,150000"),
            );
            let object = &prepared.objects[0];
            let (display, timing) = build_standard_slider_paths(
                object.source.x,
                object.source.y,
                &object.source.slider_points,
                object.source.slider_type.as_deref().unwrap(),
                object.source.slider_pixel_length,
            );
            assert_eq!(object.path.as_ref().unwrap().points, timing.points);
            assert_eq!(
                object.path.as_ref().unwrap().cumulative_lengths,
                timing.cumulative_lengths
            );
            if curve.starts_with('L') {
                assert!(display.points.len() < timing.points.len() / 100);
            }
            for scale in [0.75, 1.0, 1.5] {
                let mut config = crate::config::CoreConfig::default();
                config.render.standard.gif.SCALE = scale;
                crate::config::with_config(Arc::new(config), || {
                    let context =
                        crate::render::cpu::modes::standard::context::build_replay_context(
                            &beatmap,
                            &prepared,
                            crate::processing::timeline::TimeAxis::new(1000),
                            OutputFormat::Gif,
                        );
                    let actual = data(&context, &prepared, 0);
                    let expected = display
                        .points
                        .iter()
                        .map(|&(x, y)| to_frame_point(x, y, &context.frame_layout))
                        .collect::<Vec<_>>();
                    assert_eq!(actual.frame_path.points, expected);
                    assert_eq!(actual.timing_path.points.len(), timing.points.len());
                    for progress in [0.0, 0.25, 0.5, 0.75, 1.0] {
                        let position = path_position_at(&timing, progress);
                        let expected =
                            to_frame_point(position.0, position.1, &context.frame_layout);
                        let actual = path_position_at(&actual.timing_path, progress);
                        assert!((actual.0 - expected.0).abs() < 1e-6);
                        assert!((actual.1 - expected.1).abs() < 1e-6);
                    }
                });
            }
        }
    }

    #[test]
    fn legacy_ticks_and_full_path_come_from_preparation_and_share_cache() {
        let (mut beatmap, _, gameplay, _) = super::super::tests::object_mod_fixture(
            serde_json::json!([]),
            "100,100,1000,2,0,B|180:160|300:100,2,400",
        );
        beatmap.general.insert("FormatVersion", "7".into());
        let mut inherited = beatmap.timing_points[0];
        inherited.time = 100.0;
        inherited.beat_length = -50.0;
        inherited.uninherited = false;
        beatmap.timing_points.push(inherited);
        let prepared =
            crate::replay::preparation::standard::prepare_replay(&beatmap, &gameplay.document)
                .unwrap();
        let context = crate::render::cpu::modes::standard::context::build_replay_context(
            &beatmap,
            &prepared,
            crate::processing::timeline::TimeAxis::new(1000),
            crate::render::geometry::OutputFormat::Gif,
        );
        let object = &prepared.objects[0];
        let first = data(&context, &prepared, 0);
        assert!(Arc::ptr_eq(first, data(&context, &prepared, 0)));
        let nodes: Vec<_> = object
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Tick)
            .collect();
        assert!(!nodes.is_empty());
        assert_eq!(first.ticks.len(), nodes.len());
        for (tick, node) in first.ticks.iter().zip(nodes) {
            assert_eq!(tick.time, node.time);
            let position = object.position_at(node.time);
            assert_eq!(
                tick.center,
                to_frame_point(
                    f64::from(position[0]),
                    f64::from(position[1]),
                    &context.frame_layout
                )
            );
        }
        assert_eq!(
            first.timing_path.points.len(),
            object.path.as_ref().unwrap().points.len()
        );
    }
}
