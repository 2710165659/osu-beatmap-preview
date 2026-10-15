//! Lazer 无 Mod 几何和嵌套节点。原始字节 MD5 由宿主在准备前校验。
use crate::domain::shared::slider_path::{build_lazer_slider_path, path_position_at, SliderPath};
use crate::model::{Beatmap, StandardHitObject};
use crate::support::error::{PreviewError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Tick,
    Repeat,
    Tail,
}
#[derive(Debug, Clone)]
pub struct SliderNode {
    pub time: f64,
    pub sample_time: f64,
    pub kind: NodeKind,
    pub edge: usize,
}
#[derive(Clone)]
pub struct PreparedObject {
    pub source: StandardHitObject,
    pub position: [f32; 2],
    pub end: f64,
    pub path: Option<SliderPath>,
    pub span: f64,
    pub nodes: Vec<SliderNode>,
}
impl PreparedObject {
    pub fn position_at(&self, time: f64) -> [f32; 2] {
        let Some(path) = &self.path else {
            return self.position;
        };
        let spans = self.source.slider_repeats.max(1) as f64;
        let elapsed = ((time - self.source.start_time as f64) / self.span).clamp(0.0, spans);
        let span = (elapsed.floor() as usize).min(spans as usize - 1);
        let progress = elapsed - span as f64;
        let progress = if span.is_multiple_of(2) {
            progress
        } else {
            1.0 - progress
        };
        let point = path_position_at(path, progress);
        let offset = self.position[0] - self.source.x as f32;
        [point.0 as f32 + offset, point.1 as f32 + offset]
    }
}
pub struct PreparedStandard {
    pub objects: Vec<PreparedObject>,
    pub radius: f32,
    pub od: f64,
    pub last_end: f64,
}
pub fn prepare(beatmap: &Beatmap) -> Result<PreparedStandard> {
    crate::replay::compatibility::validate_beatmap(beatmap)?;
    let objects = beatmap.hit_objects.as_standard().unwrap().to_vec();
    if beatmap
        .timing_points
        .iter()
        .any(|p| !p.time.is_finite() || !p.beat_length.is_finite())
    {
        return Err(PreviewError::parse(
            "InvalidReplay: non-finite timing point",
        ));
    }
    if objects.is_empty() || objects.len() > 200_000 {
        return Err(PreviewError::parse(
            "InvalidReplay: unsupported object count",
        ));
    }
    let od = beatmap.difficulty.get_f64_or("OverallDifficulty", 5.0);
    let ar = beatmap.difficulty.get_f64_or("ApproachRate", od) as f32;
    let cs = beatmap.difficulty.get_f64_or("CircleSize", 5.0);
    // 与原渲染器的 f32 取整次序相同；堆叠偏移和圆半径不能因准备层迁移变化。
    let scale = (1.0 - 0.7_f32 as f64 * ((cs - 5.0) / 5.0)) as f32 / 2.0 * 1.00041_f32;
    let radius = 64.0 * scale;
    let mut prepared = Vec::with_capacity(objects.len());
    let mut node_count = 0_usize;
    let multiplier = beatmap.difficulty.get_f64_or("SliderMultiplier", 1.4);
    let tick_rate = beatmap.difficulty.get_f64_or("SliderTickRate", 1.0);
    if !multiplier.is_finite() || multiplier <= 0.0 || !tick_rate.is_finite() || tick_rate <= 0.0 {
        return Err(PreviewError::parse(
            "InvalidReplay: invalid slider difficulty",
        ));
    }
    for source in objects {
        if source.start_time.unsigned_abs() > 86_400_000
            || source.end_time.unsigned_abs() > 86_400_000
        {
            return Err(PreviewError::parse(
                "InvalidReplay: object time limit exceeded",
            ));
        }
        let offset = 0.0;
        let position = [source.x as f32 + offset, source.y as f32 + offset];
        let mut end = source.end_time as f64;
        let mut span = 0.0;
        let mut path = None;
        let mut nodes = Vec::new();
        if source.hit_type & 2 != 0 {
            if source.slider_points.len() > 1024
                || source
                    .slider_points
                    .iter()
                    .any(|&(x, y)| x.unsigned_abs() > 100_000 || y.unsigned_abs() > 100_000)
                || !source.slider_pixel_length.is_finite()
                || !(0.0..=100_000.0).contains(&source.slider_pixel_length)
            {
                return Err(PreviewError::parse(
                    "InvalidReplay: slider geometry limit exceeded",
                ));
            }
            let (beat_length, sv) = crate::domain::parser::resolve_slider_timing(
                source.start_time,
                &beatmap.timing_points,
            );
            // LegacyRulesetExtensions 在乘回 beat length 前把绿线倍率转为 f32，保留边界精度。
            let bpm_multiplier = ((100.0 / sv) as f32).clamp(10.0, 1000.0) as f64 / 100.0;
            let velocity = multiplier * 100.0 / (beat_length * bpm_multiplier);
            span = source.slider_pixel_length / velocity;
            if !span.is_finite() || span <= 0.0 || source.slider_repeats > 10_000 {
                return Err(PreviewError::parse(
                    "InvalidReplay: invalid slider duration",
                ));
            }
            end = source.start_time as f64 + span * source.slider_repeats.max(1) as f64;
            if end.abs() > 86_400_000.0 {
                return Err(PreviewError::parse(
                    "InvalidReplay: slider time limit exceeded",
                ));
            }
            // 回放采用 lazer 逻辑路径；Preview 的曲线拟合只用于原有纯预览绘制。
            path = Some(build_lazer_slider_path(
                source.x,
                source.y,
                &source.slider_points,
                source.slider_type.as_deref().unwrap_or("B"),
                source.slider_pixel_length,
            ));
            let tick_distance = velocity * beat_length
                / tick_rate
                / if beatmap.format_version() < 8 {
                    sv
                } else {
                    1.0
                };
            for index in 0..source.slider_repeats.max(1) as usize {
                let start = source.start_time as f64 + index as f64 * span;
                let mut distance = tick_distance;
                while distance < source.slider_pixel_length - velocity * 10.0 {
                    // 反向 span 的 tick 固定在同一几何位置，时间需要从终点反算并恢复升序。
                    let progress = distance / source.slider_pixel_length;
                    let time = start
                        + if index.is_multiple_of(2) {
                            progress
                        } else {
                            1.0 - progress
                        } * span;
                    nodes.push(SliderNode {
                        time,
                        sample_time: time,
                        kind: NodeKind::Tick,
                        edge: 0,
                    });
                    distance += tick_distance;
                    if nodes.len() > 100_000 {
                        return Err(PreviewError::parse(
                            "InvalidReplay: slider node limit exceeded",
                        ));
                    }
                }

                if index + 1 < source.slider_repeats.max(1) as usize {
                    nodes.push(SliderNode {
                        time: start + span,
                        sample_time: start + span,
                        kind: NodeKind::Repeat,
                        edge: index + 1,
                    });
                }
            }
            nodes.sort_by(|a, b| a.time.total_cmp(&b.time));
            let last_node = nodes.last().map_or(source.start_time as f64, |n| n.time);
            nodes.push(SliderNode {
                time: (end - 36.0).max(last_node),
                sample_time: end,
                kind: NodeKind::Tail,
                edge: source.slider_repeats.max(1) as usize,
            });
        }
        node_count = node_count
            .checked_add(nodes.len())
            .filter(|n| *n <= 1_000_000)
            .ok_or_else(|| {
                PreviewError::parse("InvalidReplay: total slider node limit exceeded")
            })?;
        prepared.push(PreparedObject {
            source,
            position,
            end,
            path,
            span,
            nodes,
        });
    }
    let end_positions: Vec<_> = prepared
        .iter()
        .map(|o| {
            let p = o.position_at(o.end);
            (p[0] as f64, p[1] as f64)
        })
        .collect();
    let end_times: Vec<_> = prepared.iter().map(|o| o.end).collect();
    let mut sources: Vec<_> = prepared.iter().map(|o| o.source.clone()).collect();
    super::stacking::apply_stacking_with_geometry(
        &mut sources,
        beatmap.format_version(),
        super::stacking::calculate_preempt(ar) as f32 * beatmap.stack_leniency() as f32,
        &end_positions,
        &end_times,
    );
    for (object, source) in prepared.iter_mut().zip(sources) {
        let offset = source.stack_height as f32 * scale * -6.4;
        object.position = [source.x as f32 + offset, source.y as f32 + offset];
        object.source = source;
    }
    let last_end = prepared
        .iter()
        .map(|o| o.end)
        .fold(f64::NEG_INFINITY, f64::max);
    Ok(PreparedStandard {
        objects: prepared,
        radius,
        od,
        last_end,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_span_ticks_keep_the_same_geometric_positions() {
        let beatmap = crate::parse_beatmap_bytes(b"osu file format v14\n[General]\nMode:0\n[Difficulty]\nSliderMultiplier:1\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n100,100,1000,2,0,L|350:100,2,250\n").unwrap();
        let prepared = prepare(&beatmap).unwrap();
        let object = &prepared.objects[0];
        let ticks: Vec<_> = object
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Tick)
            .collect();
        assert_eq!(
            ticks.iter().map(|n| n.time).collect::<Vec<_>>(),
            [1500.0, 2000.0, 2500.0, 3000.0]
        );
        assert_eq!(
            object.position_at(ticks[0].time),
            object.position_at(ticks[3].time)
        );
        assert_eq!(
            object.position_at(ticks[1].time),
            object.position_at(ticks[2].time)
        );
    }
}
