//! Standard FL：先生成共享 autoplay 光标，再按游戏默认延迟跟随光标。

use crate::domain::models::BreakPeriod;
use crate::render::visibility::{AutoplayPath, Flashlight, FlashlightKind, VisibilityTimeline};
use std::sync::OnceLock;

use super::context::{RenderCache, RenderContext};
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
        // FL 即使不显示光标也必须消费同一份位置帧；初始化顺序不可反过来。
        let cursor = context
            .autoplay
            .as_ref()
            .expect("FL 必须准备自动光标")
            .get(context);
        let path = cursor.flashlight_path(context.spinner_rate);
        let mut hit_times = Vec::new();
        let mut slider_spans = Vec::new();
        let mut cache = RenderCache::default();
        for (index, object) in context.hit_objects.iter().enumerate() {
            let start = object.start_time as f64;
            let end = object.end_time as f64;
            if object.hit_type & 8 != 0 {
                hit_times.push(end);
            } else if object.hit_type & 2 != 0 {
                let data = get_slider_render_data(&mut cache, context, index);
                hit_times.push(start);
                hit_times.extend(data.ticks.iter().map(|tick| tick.time));
                let spans = object.slider_repeats.max(1) as usize;
                for span in 0..spans {
                    hit_times.push(start + (span + 1) as f64 * (end - start) / spans as f64);
                }
                slider_spans.push((object.start_time, object.end_time));
            } else {
                hit_times.push(start);
            }
        }
        Self {
            path,
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
