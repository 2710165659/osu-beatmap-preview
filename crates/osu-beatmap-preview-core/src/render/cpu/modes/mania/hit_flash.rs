//! Mania 判定线击打闪光与长条按压效果，共享 CPU 与场景绘制。

use std::sync::Arc;

use crate::domain::models::ManiaHitObject;
use crate::render::canvas::{Img, Rgba};
use crate::render::geometry::scale_px;
use crate::render::scene::{FrameSceneBuilder, SceneRect};

use super::animation::AnimationLayout;

const DURATION_MS: i64 = 320;

pub struct HitFlashes {
    lanes: Vec<LaneFlash>,
    line_offset: i64,
}

struct LaneFlash {
    // 按头部时间排序，尾部时间保存前缀最大值，避免同轨重叠长条或单点中断按压。
    hits: Vec<(i64, i64)>,
    sprite: Arc<Img>,
}

impl HitFlashes {
    pub fn new(objects: &[ManiaHitObject], palette: &[Rgba], layout: &AnimationLayout) -> Self {
        let above = scale_px(64.0, layout.render_scale).max(1);
        let below = scale_px(18.0, layout.render_scale).max(1);
        let mut lanes: Vec<_> = layout
            .column_widths
            .iter()
            .enumerate()
            .map(|(lane, &width)| LaneFlash {
                hits: Vec::new(),
                sprite: Arc::new(flash_sprite(
                    width.max(1) as u32,
                    above,
                    below,
                    palette[lane],
                )),
            })
            .collect();
        for object in objects {
            let lane = (object.lane.max(0) as usize).min(lanes.len() - 1);
            let end = if object.is_long_note {
                object.end_time.max(object.start_time)
            } else {
                object.start_time
            };
            lanes[lane].hits.push((object.start_time, end));
        }
        for lane in &mut lanes {
            lane.hits.sort_unstable();
            let mut latest_end = i64::MIN;
            for (_, end) in &mut lane.hits {
                latest_end = latest_end.max(*end);
                *end = latest_end;
            }
        }
        Self {
            lanes,
            line_offset: above,
        }
    }

    fn alpha_at(&self, lane: usize, time: i64) -> Option<f64> {
        let hits = &self.lanes[lane].hits;
        let last = hits
            .partition_point(|&(start, _)| start <= time)
            .checked_sub(1)?;
        // 长条头部到达后保持全亮，从尾部经过判定线时开始沿用普通闪光的淡出。
        let elapsed = time.saturating_sub(hits[last].1).max(0);
        (elapsed < DURATION_MS).then(|| (1.0 - elapsed as f64 / DURATION_MS as f64).powf(1.25))
    }

    // 只由谱面时间决定亮度，兼容多段 GIF、并行导出和实时 seek，无逐帧状态。
    fn visit(
        &self,
        time: i64,
        left: i64,
        layout: &AnimationLayout,
        mut draw: impl FnMut(&Arc<Img>, i64, i64, f64),
    ) {
        for (lane, flash) in self.lanes.iter().enumerate() {
            if let Some(alpha) = self.alpha_at(lane, time) {
                draw(
                    &flash.sprite,
                    left + layout.left_panel_width + layout.column_left_offsets[lane],
                    layout.playfield_top + layout.hit_position_y - self.line_offset,
                    alpha,
                );
            }
        }
    }

    pub fn draw(&self, canvas: &mut Img, time: i64, left: i64, layout: &AnimationLayout) {
        self.visit(time, left, layout, |sprite, x, y, alpha| {
            canvas.alpha_composite_scaled(sprite, x, y, alpha);
        });
    }

    pub fn draw_scene(
        &self,
        scene: &mut FrameSceneBuilder,
        time: i64,
        left: i64,
        layout: &AnimationLayout,
    ) {
        self.visit(time, left, layout, |sprite, x, y, alpha| {
            scene.sprite(
                Arc::clone(sprite),
                SceneRect {
                    x: x as f32,
                    y: y as f32,
                    width: sprite.w as f32,
                    height: sprite.h as f32,
                },
                alpha as f32,
            );
        });
    }
}

fn flash_sprite(width: u32, above: i64, below: i64, color: Rgba) -> Img {
    let mut sprite = Img::new(width, (above + below + 1) as u32, [0; 4]);
    for y in 0..sprite.h {
        let distance = y as f64 - above as f64;
        let vertical = if distance <= 0.0 {
            1.0 + distance / above as f64
        } else {
            1.0 - distance / below as f64
        };
        let core = (-0.5 * (distance / (below as f64 * 0.35).max(0.5)).powi(2)).exp();
        for x in 0..width {
            let horizontal = (1.0 - ((2.0 * x as f64 + 1.0) / width as f64 - 1.0).powi(2)).max(0.0);
            let alpha = ((vertical.powf(1.35) * 0.85 + core * 0.95) * horizontal).clamp(0.0, 1.0);
            let mut pixel = color.map(|channel| (channel as f64 * 0.2 + 255.0 * 0.8).round() as u8);
            pixel[3] = (alpha * 255.0).round() as u8;
            sprite.put(x, y, pixel);
        }
    }
    sprite
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{with_config, CoreConfig};
    use crate::render::cpu::modes::mania::animation::{
        build_layout, build_video_layout, gif_frame_origin, prepare_mania_gif_frames,
    };
    use crate::render::cpu::modes::mania::{lane_palette, skin::load_mania_skin_config};
    use crate::render::geometry::OutputFormat;
    use crate::render::scene::DrawCommand;

    #[test]
    fn flashes_follow_lane_hits_and_fade_without_seek_state() {
        let skin = load_mania_skin_config(4, OutputFormat::Gif);
        let layout = build_layout(&skin, OutputFormat::Gif);
        let objects = [
            (0, 1100, 1100),
            (1, 1200, 1300),
            (1, 1000, 1500),
            (0, 1000, 1000),
            (1, 1400, 1400),
        ]
        .map(|(lane, start_time, end_time)| ManiaHitObject {
            lane,
            start_time,
            end_time,
            is_long_note: end_time > start_time,
            ..Default::default()
        });
        let flashes = HitFlashes::new(&objects, &lane_palette(4), &layout);
        // 故意乱序查询，覆盖回退 seek、同轨重击、和弦、淡出边界与长条尾部。
        for (time, lane0, lane1) in [
            (999, None, None),
            (1000, Some(1.0), Some(1.0)),
            (1160, Some((1.0_f64 - 60.0 / 320.0).powf(1.25)), Some(1.0)),
            (1100, Some(1.0), Some(1.0)),
            (1420, None, Some(1.0)),
            (1500, None, Some(1.0)),
            (1660, None, Some(0.5_f64.powf(1.25))),
            (1820, None, None),
            (1000, Some(1.0), Some(1.0)),
        ] {
            for (lane, expected) in [(0, lane0), (1, lane1), (2, None), (3, None)] {
                match (flashes.alpha_at(lane, time), expected) {
                    (Some(actual), Some(expected)) => assert!((actual - expected).abs() < 1e-12),
                    (actual, expected) => assert_eq!(actual, expected, "lane={lane} time={time}"),
                }
            }
        }
    }

    #[test]
    fn flash_sprites_follow_each_output_layout_at_fractional_scales() {
        for format in [OutputFormat::Gif, OutputFormat::Mp4] {
            for scale in [0.75, 1.0, 1.5] {
                let mut config = CoreConfig::default();
                config.render.mania.gif.SCALE = if format == OutputFormat::Gif {
                    scale
                } else {
                    2.0
                };
                config.render.mania.mp4.SCALE = if format == OutputFormat::Mp4 {
                    scale
                } else {
                    2.0
                };
                with_config(Arc::new(config), || {
                    for keys in [1, 4, 7, 18] {
                        let skin = load_mania_skin_config(keys, format);
                        let layout = if format == OutputFormat::Gif {
                            build_layout(&skin, format)
                        } else {
                            build_video_layout(&skin, format)
                        };
                        let objects: Vec<_> = (0..keys)
                            .map(|lane| ManiaHitObject {
                                lane,
                                start_time: 1000,
                                end_time: 1000,
                                ..Default::default()
                            })
                            .collect();
                        let flashes = HitFlashes::new(&objects, &lane_palette(keys), &layout);
                        let mut canvas = Img::new(
                            layout.image_width as u32,
                            layout.image_height as u32,
                            [0; 4],
                        );
                        {
                            let left = layout.playfield_left;
                            flashes.draw(&mut canvas, 1000, left, &layout);
                            let mut scene = FrameSceneBuilder::new(canvas.w, canvas.h, 1000);
                            flashes.draw_scene(&mut scene, 1000, left, &layout);
                            let scene = scene.finish();
                            assert_eq!(scene.commands.len(), keys as usize);
                            for (lane, command) in scene.commands.iter().enumerate() {
                                let x = left
                                    + layout.left_panel_width
                                    + layout.column_left_offsets[lane];
                                let y = layout.playfield_top + layout.hit_position_y;
                                let center = x + layout.column_widths[lane] / 2;
                                assert!(canvas.get(center as u32, y as u32)[3] > 0);
                                let DrawCommand::Sprite {
                                    destination, alpha, ..
                                } = command
                                else {
                                    panic!("闪光应使用共享精灵");
                                };
                                assert_eq!(destination.x, x as f32);
                                assert_eq!(destination.y + flashes.line_offset as f32, y as f32);
                                assert_eq!(destination.width, layout.column_widths[lane] as f32);
                                assert_eq!(*alpha, 1.0);
                            }
                        }
                    }
                });
            }
        }
    }

    #[test]
    fn gif_flash_switch_is_independent_and_uses_each_segment_time() {
        use crate::domain::shared::time_selection::{GifRenderOptions, TimeAxis};
        use crate::domain::timeout::RequestDeadline;
        use std::time::{Duration, Instant};

        let beatmap = crate::parse_beatmap_bytes(
            b"osu file format v14\n[General]\nMode:3\n[Difficulty]\nCircleSize:4\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n64,192,1000,1,0\n192,192,1500,128,0,2000:0:0:0:0:\n",
        ).unwrap();
        let render = |enabled| {
            let mut config = CoreConfig::default();
            config.render.mania.gif.style.SHOW_HIT_FLASH = enabled;
            config.render.mania.mp4.style.SHOW_HIT_FLASH = !enabled;
            with_config(Arc::new(config), || {
                prepare_mania_gif_frames(
                    &beatmap,
                    None,
                    GifRenderOptions::ResolvedSegments {
                        times_ms: vec![900, 1000],
                        duration_seconds: Some(1.0),
                        time_axis: TimeAxis::new(1000),
                    },
                    Some(20),
                    &RequestDeadline::new(Instant::now(), "gif", Duration::from_secs(30)),
                )
                .unwrap()
            })
        };
        let hidden = render(false);
        let shown = render(true);
        let skin = load_mania_skin_config(4, OutputFormat::Gif);
        let layout = build_layout(&skin, OutputFormat::Gif);
        for frame in [18, 10, 0, 2, 2] {
            let hidden = hidden.render(frame);
            let shown = shown.render(frame);
            for segment in 0..2 {
                let (origin_x, origin_y) = gif_frame_origin(segment as usize, &layout, true);
                let y = origin_y + layout.playfield_top + layout.hit_position_y;
                for lane in 0..2 {
                    let x = origin_x
                        + layout.playfield_left
                        + layout.left_panel_width
                        + layout.column_left_offsets[lane]
                        + layout.column_widths[lane] / 2;
                    let changed = shown.get(x as u32, y as u32) != hidden.get(x as u32, y as u32);
                    let expected = if lane == 0 {
                        matches!((frame, segment), (0, 1) | (2, 0 | 1))
                    } else {
                        900 + segment * 100 + frame as i64 * 50 >= 1500
                    };
                    assert_eq!(
                        changed, expected,
                        "frame={frame} segment={segment} lane={lane}"
                    );
                }
            }
        }
    }
}
