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
