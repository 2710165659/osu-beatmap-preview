//! Standard FL：用堆叠、HR 和滑条路径处理后的自动光标驱动光圈。

use crate::domain::models::BreakPeriod;
use crate::render::visibility::{AutoplayPath, Flashlight, FlashlightKind, VisibilityTimeline};
use std::sync::OnceLock;

use super::context::{stacked_position, to_frame_point, RenderCache, RenderContext};
use super::slider::get_slider_render_data;

pub struct StandardFlashlightCache {
    state: OnceLock<StandardFlashlight>,
    breaks: Vec<BreakPeriod>,
}

impl StandardFlashlightCache {
    pub fn new(breaks: &[BreakPeriod]) -> Self {
        Self {
            state: OnceLock::new(),
            breaks: breaks.to_vec(),
        }
    }

    pub fn get(&self, context: &RenderContext) -> &StandardFlashlight {
        self.state
            .get_or_init(|| StandardFlashlight::new(context, &self.breaks))
    }
}

pub struct StandardFlashlight {
    path: AutoplayPath,
    timeline: VisibilityTimeline,
    slider_spans: Vec<(i64, i64)>,
}

impl StandardFlashlight {
    pub fn new(context: &RenderContext, breaks: &[BreakPeriod]) -> Self {
        let mut points = Vec::new();
        let mut hit_times = Vec::new();
        let mut slider_spans = Vec::new();
        let mut cache = RenderCache::default();
        if let Some(first) = context.hit_objects.first() {
            let center = to_frame_point(256.0, 192.0, &context.frame_layout);
            points.push((
                (first.start_time - context.settings.preempt_ms) as f64,
                [center.0, center.1],
            ));
        }
        for (index, object) in context.hit_objects.iter().enumerate() {
            let start = object.start_time as f64;
            let end = object.end_time as f64;
            if object.hit_type & 8 != 0 {
                let center = to_frame_point(256.0, 192.0, &context.frame_layout);
                points.extend([(start, [center.0, center.1]), (end, [center.0, center.1])]);
                hit_times.push(end);
            } else if object.hit_type & 2 != 0 {
                let data = get_slider_render_data(&mut cache, context, index);
                let path = &data.timing_path;
                let spans = object.slider_repeats.max(1) as usize;
                let span_duration = (end - start) / spans as f64;
                hit_times.push(start);
                hit_times.extend(data.ticks.iter().map(|tick| tick.time));
                for span in 0..spans {
                    hit_times.push(start + (span + 1) as f64 * span_duration);
                    for node in 0..path.points.len() {
                        let node = if span % 2 == 0 {
                            node
                        } else {
                            path.points.len() - 1 - node
                        };
                        let distance = if span % 2 == 0 {
                            path.cumulative_lengths[node]
                        } else {
                            path.total_length - path.cumulative_lengths[node]
                        };
                        let fraction = if path.total_length > 0.0 {
                            distance / path.total_length
                        } else {
                            0.0
                        };
                        let (x, y) = path.points[node];
                        points.push((start + (span as f64 + fraction) * span_duration, [x, y]));
                    }
                }
                slider_spans.push((object.start_time, object.end_time));
            } else {
                let (x, y) = stacked_position(object, &context.settings);
                let point = to_frame_point(x, y, &context.frame_layout);
                points.push((start, [point.0, point.1]));
                hit_times.push(start);
            }
        }
        Self {
            // 默认跟随延迟 120ms 的 Out 插值用 60ms 连续阻尼近似，保持不同 fps 一致。
            path: AutoplayPath::new(points, 60.0),
            timeline: VisibilityTimeline::new(hit_times, breaks),
            slider_spans,
        }
    }

    pub fn at(&self, context: &RenderContext, time: i64) -> Flashlight {
        Flashlight {
            center: self.path.position_at(time),
            radius: 125.0
                * context.frame_layout.scale
                * self
                    .timeline
                    .flashlight_scale(FlashlightKind::Standard, time),
            band: false,
            dim: if self
                .slider_spans
                .iter()
                .any(|&(start, end)| start <= time && time < end)
            {
                0.8
            } else {
                0.0
            },
        }
    }
}
