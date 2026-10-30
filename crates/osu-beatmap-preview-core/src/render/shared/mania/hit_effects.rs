//! 回放实际命中与长条按压的光效时间轴；独立于帧顺序，CPU 与实时场景共用。
use crate::{
    gameplay::{events::EventKind, timeline::GameplayTimeline},
    render::{
        canvas::{Img, Rgba},
        cpu::modes::mania::{animation::AnimationLayout, hit_flash::flash_sprite},
        geometry::scale_px,
        shared::replay::overlay::OverlayCanvas,
    },
    replay::preparation::mania::PreparedMania,
};
use std::sync::Arc;

const FADE_IN: f64 = 80.0;
const FADE_OUT: f64 = 120.0;

struct Lane {
    intervals: Vec<(f64, f64)>,
    latest_end: Vec<f64>,
    sprite: Arc<Img>,
}

pub(super) struct HitEffects {
    lanes: Vec<Lane>,
    center_offset: f64,
}

impl HitEffects {
    pub fn new(
        prepared: &PreparedMania,
        gameplay: &GameplayTimeline,
        palette: &[Rgba],
        layout: &AnimationLayout,
    ) -> Self {
        let above = scale_px(64.0, layout.render_scale).max(1);
        let below = scale_px(18.0, layout.render_scale).max(1);
        let mut lanes: Vec<_> = layout
            .column_widths
            .iter()
            .zip(palette)
            .map(|(&w, &color)| Lane {
                intervals: Vec::new(),
                latest_end: Vec::new(),
                sprite: Arc::new(flash_sprite(w.max(1) as u32, above, below, color)),
            })
            .collect();
        for event in &gameplay.events {
            if event.kind == EventKind::Judgement && event.result.is_hit() {
                let object = &prepared.objects[event.object_id];
                lanes[object.lane as usize]
                    .intervals
                    .push((event.time, event.time + FADE_IN));
            }
        }
        for (object, state) in prepared.objects.iter().zip(&gameplay.objects) {
            if !object.is_long_note {
                continue;
            }
            for (index, &(start, holding)) in state.tracking.iter().enumerate() {
                if !holding {
                    continue;
                }
                let end = state
                    .tracking
                    .get(index + 1)
                    .map_or(gameplay.settlement_time, |p| p.0);
                let end = state.end.map_or(end, |p| end.min(p.0));
                if end > start {
                    lanes[object.lane as usize].intervals.push((start, end));
                }
            }
        }
        for lane in &mut lanes {
            lane.intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut end = f64::NEG_INFINITY;
            lane.latest_end = lane
                .intervals
                .iter()
                .map(|p| {
                    end = end.max(p.1);
                    end
                })
                .collect();
        }
        Self {
            lanes,
            center_offset: (below - above + 1) as f64 / 2.0,
        }
    }

    pub fn draw(&self, target: &mut impl OverlayCanvas, time: f64, layout: &AnimationLayout) {
        for (index, lane) in self.lanes.iter().enumerate() {
            let alpha = lane.alpha_at(time);
            if alpha > 0.0 {
                target.additive_transformed(
                    &lane.sprite,
                    [
                        (layout.playfield_left
                            + layout.left_panel_width
                            + layout.column_left_offsets[index]) as f64
                            + lane.sprite.w as f64 / 2.0,
                        (layout.playfield_top + layout.hit_position_y) as f64 + self.center_offset,
                    ],
                    1.0,
                    0.0,
                    alpha,
                );
            }
        }
    }
}

impl Lane {
    fn alpha_at(&self, time: f64) -> f64 {
        let count = self.intervals.partition_point(|p| p.0 <= time);
        let mut alpha: f64 = 0.0;
        // 前缀最大尾时间保留同轨重叠长条，不能只查询最后一次敲击。
        for index in (0..count).rev() {
            if self.latest_end[index] + FADE_OUT <= time {
                break;
            }
            let (start, end) = self.intervals[index];
            let peak = ((end - start) / FADE_IN).min(1.0);
            let value = if time < end {
                ((time - start) / FADE_IN).min(1.0)
            } else {
                peak * (1.0 - (time - end) / FADE_OUT).max(0.0)
            };
            alpha = alpha.max(value);
        }
        alpha
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        render::{
            cpu::modes::mania::{
                animation::build_layout, lane_palette, skin::load_mania_skin_config,
            },
            geometry::OutputFormat,
        },
        replay::preparation::mania::tests::fixture,
    };

    #[test]
    fn actual_hits_and_hold_intervals_survive_seek_without_flashing_misses_or_empty_presses() {
        let (prepared, document) = fixture(
            "64,192,1000,1,0\n192,192,1000,1,0\n320,192,1500,128,0,2500:0:0:0:0:",
            &[
                (0, 0),
                (980, 1),
                (990, 0),
                (1300, 8),
                (1400, 0),
                (1500, 4),
                (1800, 0),
                (1900, 4),
                (2500, 0),
                (3000, 0),
            ],
        );
        let gameplay = crate::gameplay::engine::compile_mania(&prepared, document).unwrap();
        let layout = build_layout(
            &load_mania_skin_config(4, OutputFormat::Gif),
            OutputFormat::Gif,
        );
        let effects = HitEffects::new(&prepared, &gameplay, &lane_palette(4), &layout);
        for (time, lane, expected) in [
            (1060.0, 0, 1.0),
            (970.0, 0, 0.0),
            (1020.0, 0, 0.5),
            (1180.0, 0, 0.0),
            (1200.0, 1, 0.0),
            (1340.0, 3, 0.0),
            (1750.0, 2, 1.0),
            (1860.0, 2, 0.5),
            (1900.0, 2, 1.0 / 6.0),
            (2100.0, 2, 1.0),
            (2800.0, 2, 0.0),
            (1750.0, 2, 1.0),
        ] {
            assert!(
                (effects.lanes[lane].alpha_at(time) - expected).abs() < 1e-9,
                "lane={lane} time={time}"
            );
        }
    }
}
