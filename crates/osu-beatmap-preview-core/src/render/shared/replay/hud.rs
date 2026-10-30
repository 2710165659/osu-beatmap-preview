//! 回放基础 HUD：血量、分数、ACC、评级、Combo、误差条与时间。
//!
//! 血条用代码近似 LegacyHealthDisplay 的窄条样式并贴合左上角；
//! 判定条是带命中窗口配色的细横轴，叠加逐次判定刻度与平均偏移标记。
use super::overlay::OverlayCanvas;
use crate::gameplay::events::HitResult;
use crate::gameplay::state::ReplayStatus;
use crate::gameplay::timeline::GameplayTimeline;
use crate::processing::timeline::TimeAxis;
use crate::render::canvas::Img;
use crate::render::text::{format_mmss_floor, scaled_bitmap_font_height, shared_text_sprite};
use std::sync::Arc;

pub(crate) struct HudVisuals {
    pub font: Option<super::font::ArgonFont>,
    pub combo_center: Option<[i64; 2]>,
    pub hit_error_bottom: f64,
    pub hit_error_axis: Arc<Img>,
    health: HealthAnimation,
    windows: [f64; 3],
}
impl HudVisuals {
    pub fn new(
        gameplay: &GameplayTimeline,
        font: Option<super::font::ArgonFont>,
        scale: f64,
        windows: [f64; 3],
    ) -> Self {
        Self {
            combo_center: None,
            hit_error_bottom: HIT_ERROR_BOTTOM,
            font,
            hit_error_axis: build_hit_error_axis(scale, windows),
            health: HealthAnimation::new(gameplay),
            windows,
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) struct HudLayout {
    pub width: i64,
    pub height: i64,
    pub scale: f64,
    pub time_axis: TimeAxis,
}

/// 判定条配色与 lazer OsuColour 的命中结果一致（BlueLight / Green / Yellow）。
const HIT_ERROR_GREAT: [u8; 3] = [153, 238, 255];
const HIT_ERROR_OK: [u8; 3] = [136, 179, 0];
const HIT_ERROR_MEH: [u8; 3] = [255, 204, 34];
const HIT_ERROR_MISS: [u8; 3] = [237, 17, 33];

// —— 逻辑尺寸 ——
// 所有数值都以「当前格式 SCALE = 1 的帧」为单位，绘制前统一乘以 SCALE。
/// 仿 Legacy 的窄条尺寸；位置固定在画布左上角，尺寸使用当前格式 SCALE。
const HEALTH_WIDTH: f64 = 150.0;
const HEALTH_HEIGHT: f64 = 5.0;
/// 下方偏移条保持原有大小，只将轴线移到距底部 16 个逻辑像素的位置。
const HIT_ERROR_WIDTH: f64 = 150.0;
const HIT_ERROR_BOTTOM: f64 = 16.0;
const HIT_ERROR_THICKNESS: f64 = 2.0;
const HIT_ERROR_TICK: f64 = 7.0;
/// 判定刻度的存活时间与同屏上限，对应 Argon BarHitErrorMeter。
const HIT_ERROR_TICK_LIFETIME: f64 = 5000.0;
const HIT_ERROR_MAX_TICKS: usize = 50;
/// 刻度展开后的最大不透明度（Argon 用 0.6）。
const HIT_ERROR_TICK_ALPHA: f64 = 0.6;
/// 平均偏移标记的加权系数：与新判定按 0.9 / 0.1 折叠。
const HIT_ERROR_AVERAGE_OLD: f64 = 0.9;

pub(crate) fn display_rank(rank: Option<&str>, status: ReplayStatus) -> &str {
    if status == ReplayStatus::Failed {
        "F"
    } else {
        match rank {
            Some("X" | "XH") => "SS",
            Some("SH") => "S",
            Some(rank) => rank,
            None => "-",
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    image: &mut impl OverlayCanvas,
    layout: HudLayout,
    gameplay: &GameplayTimeline,
    visuals: &HudVisuals,
    time: f64,
    total: i64,
) {
    let scale = layout.scale;
    let px = |value: f64| (value * scale).round() as i64;
    let width = layout.width;
    let height = layout.height;
    let state = gameplay.state_at(time);
    let pink = [252, 97, 143, 255];
    draw_health(image, visuals.health.at(time), scale);
    if !matches!(
        gameplay.policy,
        crate::replay::compatibility::ReplayPolicy::Catch(_)
    ) {
        draw_hit_error_meter(
            image,
            gameplay,
            visuals,
            time,
            width,
            height,
            px,
            visuals.windows[2],
        );
    }
    draw_score(image, state, visuals, width, px, scale);
    draw_summary(image, state, visuals, width, px, scale);
    draw_combo(image, state.combo, visuals, height, px, scale);
    draw_progress(
        image,
        layout.time_axis,
        time,
        total,
        visuals,
        width,
        height,
        px,
        scale,
    );
    draw_failure(image, state.status, visuals, width, px, scale, pink);
}

const HEALTH_TRANSITION_MS: f64 = 200.0;
const HEALTH_PULSE_MS: f64 = 180.0;

/// 预计算每次血量变更的连续起点，逐帧查询不推进状态，支持 seek 和并行导出。
pub(crate) struct HealthAnimation {
    initial: f64,
    changes: Vec<HealthChange>,
    pulse_times: Vec<f64>,
}

struct HealthChange {
    time: f64,
    from: f64,
    to: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct HealthFrame {
    value: f64,
    pulse: f64,
}

impl HealthChange {
    fn value_at(&self, time: f64) -> f64 {
        let progress = ((time - self.time) / HEALTH_TRANSITION_MS).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * (1.0 - (1.0 - progress).powi(5))
    }
}

impl HealthAnimation {
    pub(crate) fn new(gameplay: &GameplayTimeline) -> Self {
        let mut animation = Self::from_samples(
            gameplay.initial.health,
            gameplay
                .checkpoints
                .iter()
                .map(|(time, state)| (*time, state.health)),
        );
        // 满血时判定不会改变血量，脉冲必须独立消费判定事件，不能只观察血量差。
        animation.pulse_times = gameplay
            .events
            .iter()
            .filter(|event| {
                event.kind == crate::gameplay::events::EventKind::Judgement
                    && event.result.is_hit()
                    && !matches!(
                        event.result,
                        HitResult::IgnoreHit | HitResult::SmallBonus | HitResult::LargeBonus
                    )
            })
            .map(|event| event.time)
            .collect();
        animation
    }

    fn from_samples(initial: f64, samples: impl IntoIterator<Item = (f64, f64)>) -> Self {
        let mut animation = Self {
            initial: initial.clamp(0.0, 1.0),
            changes: Vec::new(),
            pulse_times: Vec::new(),
        };
        let mut previous = animation.initial;
        for (time, health) in samples {
            let target = health.clamp(0.0, 1.0);
            if target == previous {
                continue;
            }
            // 动画未结束就再次变更时，从该时刻的显示值续接，避免连续判定导致跳变。
            let from = animation
                .changes
                .last()
                .map_or(animation.initial, |change| change.value_at(time));
            animation.changes.push(HealthChange {
                time,
                from,
                to: target,
            });
            previous = target;
        }
        animation
    }

    fn at(&self, time: f64) -> HealthFrame {
        let change = self
            .changes
            .partition_point(|change| change.time <= time)
            .checked_sub(1)
            .map(|index| &self.changes[index]);
        let hit_time = self
            .pulse_times
            .partition_point(|hit| *hit <= time)
            .checked_sub(1)
            .map(|index| self.pulse_times[index]);
        let pulse_at = |start: f64| {
            (1.0 - (time - start) / HEALTH_PULSE_MS)
                .clamp(0.0, 1.0)
                .powi(3)
        };
        // 掉血也点亮末端，让 Miss 后的缩短过程可辨识；成功判定和掉血共用一套时钟。
        let damage_pulse = change
            .filter(|change| change.to < change.from)
            .map_or(0.0, |change| pulse_at(change.time));
        HealthFrame {
            value: change.map_or(self.initial, |change| change.value_at(time)),
            pulse: hit_time.map_or(0.0, pulse_at).max(damage_pulse),
        }
    }
}

/// 对照 LegacyHealthDisplay.getFillColour：半血以下逐渐变暗，20% 以下转红。
fn legacy_health_tint(health: f64) -> [u8; 3] {
    if health < 0.2 {
        [((0.2 - health) / 0.2 * 255.0).round() as u8, 0, 0]
    } else if health < 0.5 {
        let level = (health / 0.5 * 255.0).round() as u8;
        [level; 3]
    } else {
        [255; 3]
    }
}

/// 薄条保留左上角直边，底槽和填充右侧按同一半圆收口；圆形标记贴住移动端。
fn draw_health(image: &mut impl OverlayCanvas, frame: HealthFrame, scale: f64) {
    let health = frame.value.clamp(0.0, 1.0);
    let width = (HEALTH_WIDTH * scale).round().max(1.0);
    let height = (HEALTH_HEIGHT * scale).round().max(1.0) as i64;
    let radius = height as f64 / 2.0;
    let filled = width * health;
    let tint = legacy_health_tint(health);
    for row in 0..height {
        // 在每条扫描线上计算半圆边界，两条后端都用互不重叠的矩形，避免圆帽拼接缝。
        let dy = row as f64 + 0.5 - radius;
        let inset = radius - (radius * radius - dy * dy).max(0.0).sqrt();
        let background_width = (width - inset).round().max(0.0) as i64;
        image.fill_rect_size(0, row, background_width, 1, [24, 27, 32, 220]);
        let fill_width = (filled - inset).round().max(0.0) as i64;
        let progress = row as f64 / (height - 1).max(1) as f64;
        let bottom = [170.0, 218.0, 240.0];
        let colour: [u8; 3] = std::array::from_fn(|channel| {
            let value = 255.0 + (bottom[channel] - 255.0) * progress;
            (value * f64::from(tint[channel]) / 255.0).round() as u8
        });
        image.fill_rect_size(
            0,
            row,
            fill_width,
            1,
            [colour[0], colour[1], colour[2], 255],
        );
    }
    if filled < 0.5 {
        return;
    }
    // 标记的右边缘与填充末端对齐；脉冲只让圆环略微鼓起，始终与薄条相连。
    let marker_radius = radius * (1.15 + 0.3 * frame.pulse);
    let centre = filled - radius;
    let glow = (45.0 + 65.0 * frame.pulse).round() as u8;
    image.blend_circle(
        centre,
        radius,
        marker_radius + 1.5 * scale,
        [tint[0], tint[1], tint[2], glow],
    );
    image.blend_circle(
        centre,
        radius,
        marker_radius,
        [tint[0], tint[1], tint[2], 255],
    );
    let inner: [u8; 3] = std::array::from_fn(|channel| {
        (f64::from([145, 210, 236][channel]) * f64::from(tint[channel]) / 255.0).round() as u8
    });
    image.blend_circle(
        centre,
        radius,
        marker_radius * 0.58,
        [inner[0], inner[1], inner[2], 255],
    );
}

/// 判定条颜色轴：中段 300 窗、外侧 100 窗、最外 50 窗，命中窗末端 20% 渐隐。
///
/// 颜色轴只与 OD 和当前格式 SCALE 有关，准备阶段生成一次即可，逐帧重建只会多出
/// 上百次像素写入。
pub(crate) fn build_hit_error_axis(scale: f64, windows: [f64; 3]) -> Arc<Img> {
    let width = (HIT_ERROR_WIDTH * scale).round().max(3.0) as u32;
    let height = (HIT_ERROR_THICKNESS * scale).round().max(1.0) as u32;
    let half = (f64::from(width) - 1.0) / 2.0;
    let max_window = windows[2].max(1.0);
    let great_edge = windows[0] / max_window;
    let ok_edge = windows[1] / max_window;
    // Argon 只有最外段（50 窗）带渐变，且从该段 80% 处开始淡出。
    let fade_start = ok_edge + (1.0 - ok_edge) * 0.8;
    let mut axis = Img::new(width, height, [0; 4]);
    for x in 0..width {
        let distance = (f64::from(x) - half).abs() / half.max(1.0);
        let colour = if distance <= great_edge {
            HIT_ERROR_GREAT
        } else if distance <= ok_edge {
            HIT_ERROR_OK
        } else {
            HIT_ERROR_MEH
        };
        let alpha = if distance > fade_start {
            ((1.0 - distance) / (1.0 - fade_start)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let alpha = (alpha * 255.0).round() as u8;
        for y in 0..height {
            axis.put(x, y, [colour[0], colour[1], colour[2], alpha]);
        }
    }
    Arc::new(axis)
}

/// 判定刻度：100ms 内展开并淡入到最大不透明度，之后 5 秒按 InQuint 收回并淡出。
fn tick_shape(age: f64) -> (f64, f64) {
    const GROW: f64 = 100.0;
    if age < GROW {
        let progress = (age / GROW).clamp(0.0, 1.0);
        (progress, HIT_ERROR_TICK_ALPHA * progress)
    } else {
        let progress = ((age - GROW) / HIT_ERROR_TICK_LIFETIME).clamp(0.0, 1.0);
        (
            1.0 - progress.powi(5),
            HIT_ERROR_TICK_ALPHA * (1.0 - progress),
        )
    }
}

fn tick_colour(result: HitResult, alpha: f64) -> [u8; 4] {
    let base = match result {
        HitResult::Perfect | HitResult::Great | HitResult::SliderTailHit => HIT_ERROR_GREAT,
        HitResult::Good | HitResult::Ok | HitResult::LargeTickHit | HitResult::SmallTickHit => {
            HIT_ERROR_OK
        }
        HitResult::Meh => HIT_ERROR_MEH,
        HitResult::Miss | HitResult::LargeTickMiss | HitResult::SmallTickMiss => HIT_ERROR_MISS,
        _ => [200, 200, 200],
    };
    [
        base[0],
        base[1],
        base[2],
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

#[allow(clippy::too_many_arguments)]
fn draw_hit_error_meter(
    image: &mut impl OverlayCanvas,
    gameplay: &GameplayTimeline,
    visuals: &HudVisuals,
    time: f64,
    width: i64,
    height: i64,
    px: impl Fn(f64) -> i64,
    max_window: f64,
) {
    let axis = &visuals.hit_error_axis;
    let axis_y = height as f64 - px(visuals.hit_error_bottom) as f64;
    image.shared_sprite(
        axis,
        width / 2 - axis.w as i64 / 2,
        (axis_y - axis.h as f64 / 2.0).round() as i64,
    );
    let max_window = max_window.max(1.0);
    let half_width = axis.w as f64 / 2.0;
    let to_x = |error: f64| width as f64 / 2.0 + (error / max_window).clamp(-1.0, 1.0) * half_width;

    let end = gameplay.events.partition_point(|event| event.time <= time);
    let mut hits = gameplay.events[..end]
        .iter()
        .rev()
        .filter(|event| event.error.is_some() && event.result.is_hit());
    // 移动平均按时间顺序折叠：倒序回放、任意 seek 与改变帧率都得到同一结果。
    let mut errors = [0.0_f64; HIT_ERROR_MAX_TICKS];
    let mut samples = 0;
    for event in hits.clone().take(HIT_ERROR_MAX_TICKS) {
        errors[samples] = event.error.unwrap_or_default();
        samples += 1;
    }
    let mut average = 0.0;
    for error in errors[..samples].iter().rev() {
        average = average * HIT_ERROR_AVERAGE_OLD + error * (1.0 - HIT_ERROR_AVERAGE_OLD);
    }
    let tick_height = px(HIT_ERROR_TICK).max(1) as f64;
    let tick_width = px(1.75).max(1) as f64;
    for event in hits.by_ref().take(HIT_ERROR_MAX_TICKS) {
        let age = time - event.time;
        if age > HIT_ERROR_TICK_LIFETIME {
            break;
        }
        let (grow, alpha) = tick_shape(age);
        let half = tick_height * grow / 2.0;
        let x = to_x(event.error.unwrap_or_default());
        image.blend_line(
            x,
            axis_y - half,
            x,
            axis_y + half,
            tick_width,
            tick_colour(event.result, alpha),
        );
    }
    // 中心标记采用 Argon 的 Circle 样式：外圈浅蓝、内圈深蓝。
    let centre = width as f64 / 2.0;
    image.blend_circle(centre, axis_y, px(1.5) as f64, [173, 228, 255, 220]);
    image.blend_circle(centre, axis_y, px(0.7) as f64, [36, 84, 156, 235]);
    // 平均偏移标记：轴线上方的 V 形，指向当前平均判定位置。
    if samples > 1 {
        let x = to_x(average);
        let tip = axis_y - tick_height / 2.0 - px(2.0) as f64;
        let arm = px(1.5).max(1) as f64;
        let stroke = px(1.0).max(1) as f64;
        image.blend_line(x - arm, tip - arm, x, tip, stroke, [255, 255, 255, 130]);
        image.blend_line(x + arm, tip - arm, x, tip, stroke, [255, 255, 255, 130]);
    }
}

fn draw_score(
    image: &mut impl OverlayCanvas,
    state: &crate::gameplay::state::GameplayState,
    visuals: &HudVisuals,
    width: i64,
    px: impl Fn(f64) -> i64,
    scale: f64,
) {
    let score = format!("{:07}", state.score);
    let score = hud_text(visuals, &score, 36.0, [255; 4], scale);
    image.shared_sprite(&score, width - score.w as i64 - px(12.0), px(4.0));
}

fn draw_summary(
    image: &mut impl OverlayCanvas,
    state: &crate::gameplay::state::GameplayState,
    visuals: &HudVisuals,
    width: i64,
    px: impl Fn(f64) -> i64,
    scale: f64,
) {
    let accuracy = state.accuracy.map_or_else(
        || "--.--%".to_owned(),
        |value| format!("{:.2}%", value * 100.0),
    );
    let summary = hud_text(
        visuals,
        &format!(
            "{}  {}",
            display_rank(state.rank.as_deref(), state.status),
            accuracy
        ),
        21.0,
        [255; 4],
        scale,
    );
    image.shared_sprite(&summary, width - summary.w as i64 - px(12.0), px(30.0));
}

fn draw_combo(
    image: &mut impl OverlayCanvas,
    combo_value: u32,
    visuals: &HudVisuals,
    height: i64,
    px: impl Fn(f64) -> i64,
    scale: f64,
) {
    if let Some([x, y]) = visuals.combo_center {
        if combo_value > 0 {
            let combo = hud_text(visuals, &combo_value.to_string(), 38.0, [255; 4], scale);
            image.shared_sprite(&combo, x - combo.w as i64 / 2, y - combo.h as i64 / 2);
        }
    } else {
        let combo = hud_text(visuals, &format!("{}x", combo_value), 36.0, [255; 4], scale);
        image.shared_sprite(&combo, px(12.0), height - combo.h as i64 - px(8.0));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_progress(
    image: &mut impl OverlayCanvas,
    time_axis: TimeAxis,
    time: f64,
    total: i64,
    visuals: &HudVisuals,
    width: i64,
    height: i64,
    px: impl Fn(f64) -> i64,
    scale: f64,
) {
    let progress = hud_text(
        visuals,
        &format!(
            "{}/{}",
            format_mmss_floor(time_axis.to_display(time as i64)),
            format_mmss_floor(time_axis.to_display(total))
        ),
        28.0,
        [232, 232, 232, 255],
        scale,
    );
    image.shared_sprite(
        &progress,
        width - progress.w as i64 - px(12.0),
        height - progress.h as i64 - px(8.0),
    );
}

fn draw_failure(
    image: &mut impl OverlayCanvas,
    status: ReplayStatus,
    visuals: &HudVisuals,
    width: i64,
    px: impl Fn(f64) -> i64,
    scale: f64,
    pink: [u8; 4],
) {
    if status != ReplayStatus::Failed {
        return;
    }
    let label = hud_text(visuals, "REPLAY FAILED", 45.0, pink, scale);
    let w = label.w;
    image.fill_rect_size(
        (width - w as i64) / 2 - px(16.0),
        px(66.0),
        w as i64 + px(32.0),
        px(42.0),
        [71, 18, 40, 235],
    );
    image.shared_sprite(&label, (width - w as i64) / 2, px(76.0));
}

pub(crate) fn hud_text(
    visuals: &HudVisuals,
    value: &str,
    size: f64,
    color: [u8; 4],
    scale: f64,
) -> std::sync::Arc<Img> {
    visuals.font.as_ref().map_or_else(
        || shared_text_sprite(value, scaled_bitmap_font_height(size as u32, scale), color),
        |font| font.sprite_arc(value, size * scale, 0.0, [color[0], color[1], color[2]]),
    )
}

/// 各模式共用右侧居中的按键面板；按键语义由模式转换后传入。
pub(crate) fn draw_keys(
    target: &mut impl OverlayCanvas,
    visuals: &HudVisuals,
    layout: HudLayout,
    keys: &[(bool, &str)],
) {
    let px = |value: f64| (value * layout.scale).round() as i64;
    for (i, &(active, name)) in keys.iter().enumerate() {
        let x = layout.width - px(43.0);
        let y = layout.height / 2 + px(i as f64 * 31.0 - keys.len() as f64 * 15.5);
        target.fill_rect_size(
            x,
            y,
            px(34.0),
            px(26.0),
            if active {
                [252, 97, 143, 255]
            } else {
                [27, 23, 39, 205]
            },
        );
        let label = hud_text(visuals, name, 18.0, [255; 4], layout.scale);
        target.shared_sprite(
            &label,
            x + (px(34.0) - label.w as i64) / 2,
            y + (px(26.0) - label.h as i64) / 2,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::canvas::Img;
    use crate::render::scene::{DrawCommand, FrameSceneBuilder};

    #[test]
    fn uses_ss_and_f_in_the_hud() {
        assert_eq!(display_rank(Some("X"), ReplayStatus::Completed), "SS");
        assert_eq!(display_rank(Some("S"), ReplayStatus::Failed), "F");
        for rank in ["S", "A", "B", "C", "D"] {
            assert_eq!(display_rank(Some(rank), ReplayStatus::Playing), rank);
        }
    }

    /// 判定条颜色轴按 OD 的实际命中窗分段，并在最外段淡出到全透明。
    #[test]
    fn hit_error_axis_splits_at_hit_windows_and_fades_out() {
        let axis = build_hit_error_axis(
            1.0,
            crate::replay::calculate::standard::circle::windows(5.0),
        );
        let middle = axis.h / 2;
        let centre = axis.w / 2;
        assert_eq!(
            axis.get(centre, middle),
            [
                HIT_ERROR_GREAT[0],
                HIT_ERROR_GREAT[1],
                HIT_ERROR_GREAT[2],
                255
            ],
            "中心必须是 300 窗配色"
        );
        assert_eq!(axis.get(0, middle)[3], 0, "两端必须完全透明");
        let windows = crate::replay::calculate::standard::circle::windows(5.0);
        let ok_sample = (f64::from(centre) * (windows[0] + windows[1]) / (2.0 * windows[2])) as u32;
        assert_eq!(
            &axis.get(centre + ok_sample, middle)[..3],
            HIT_ERROR_OK.as_slice()
        );
        // 窗口更大的谱面（低 OD）中，300 窗颜色应覆盖更宽的范围。
        let wide = build_hit_error_axis(
            1.0,
            crate::replay::calculate::standard::circle::windows(0.0),
        );
        let count = |image: &Img| {
            (0..image.w)
                .filter(|&x| &image.get(x, middle)[..3] == HIT_ERROR_GREAT.as_slice())
                .count()
        };
        assert!(count(&wide) > count(&axis));
    }

    /// 刻度在 100ms 内展开并淡入，随后在 5 秒内收回、淡出到不可见。
    #[test]
    fn judgement_ticks_grow_then_fade_away() {
        assert_eq!(tick_shape(0.0), (0.0, 0.0));
        let (grow, alpha) = tick_shape(100.0);
        assert!((grow - 1.0).abs() < 1e-9);
        assert!((alpha - 0.6).abs() < 1e-9);
        let (grown, faded) = tick_shape(2000.0);
        assert!(grown < 1.0 && grown > 0.9);
        assert!(faded < 0.6 && faded > 0.0);
        let (end, invisible) = tick_shape(HIT_ERROR_TICK_LIFETIME + 100.0);
        assert_eq!((end, invisible), (0.0, 0.0));
    }

    /// 左上角保持贴边，右端半圆和圆形标记在多倍率下都与主条连通。
    #[test]
    fn legacy_health_keeps_a_connected_round_tip() {
        for scale in [0.5, 1.0, 1.5, 2.0] {
            let width = (HEALTH_WIDTH * scale).round() as u32;
            let height = (HEALTH_HEIGHT * scale).round() as u32;
            for health in [0.0, 0.1, 0.35, 1.0] {
                let frame = HealthFrame {
                    value: health,
                    pulse: 0.0,
                };
                let mut image = Img::new(width + 20, height + 20, [0; 4]);
                draw_health(&mut image, frame, scale);
                assert!(image.get(0, 0)[3] > 0, "主条必须贴左上角");
                let filled = (f64::from(width) * health).round() as u32;
                for x in 0..filled {
                    assert!(image.get(x, height / 2)[3] > 0, "圆形末端必须与条体相连");
                }
                if scale >= 1.0 && health < 1.0 {
                    assert_eq!(
                        image.get(width - 1, height - 1)[3],
                        0,
                        "底槽最右下角应由半圆收口"
                    );
                }
                assert_eq!(image.get(width + 10, 0), [0; 4]);
                let mut scene = FrameSceneBuilder::new(width + 20, height + 20, 0);
                draw_health(&mut scene, frame, scale);
                assert_eq!(
                    scene
                        .finish()
                        .commands
                        .iter()
                        .filter(|command| matches!(command, DrawCommand::Circle { .. }))
                        .count(),
                    if filled == 0 { 0 } else { 3 }
                );
            }
        }
    }

    /// 连续变更从当前显示值续接，动画查询顺序和帧率不影响结果。
    #[test]
    fn health_animation_is_continuous_and_seek_independent() {
        let animation =
            HealthAnimation::from_samples(1.0, [(100.0, 0.5), (180.0, 0.8), (400.0, 1.0)]);
        assert_eq!(animation.at(99.0).value, 1.0);
        assert_eq!(animation.at(100.0).value, 1.0);
        let early = animation.at(140.0);
        assert!(early.value > 0.5 && early.value < 1.0);
        assert!((animation.at(180.0).value - animation.at(179.999).value).abs() < 0.0001);
        assert_eq!(animation.at(600.0).value, 1.0);
        assert_eq!(animation.at(140.0), early);
        assert_eq!(animation.at(140.0), animation.at(140.0));
    }

    /// 满血时仍可在成功判定上显示脉冲，暂停在同一时刻不会重复放大。
    #[test]
    fn full_health_marker_pulses_without_changing_the_fill() {
        let mut animation = HealthAnimation::from_samples(1.0, []);
        animation.pulse_times = vec![100.0];
        assert_eq!(animation.at(99.0).pulse, 0.0);
        assert_eq!(
            animation.at(100.0),
            HealthFrame {
                value: 1.0,
                pulse: 1.0
            }
        );
        assert!(animation.at(150.0).pulse > 0.0 && animation.at(150.0).pulse < 1.0);
        assert_eq!(animation.at(280.0).pulse, 0.0);
        let render = |time| {
            let mut image = Img::new(180, 20, [0; 4]);
            draw_health(&mut image, animation.at(time), 1.0);
            image.data
        };
        assert_ne!(render(99.0), render(100.0));
        assert_eq!(render(100.0), render(100.0));
    }

    #[test]
    fn legacy_health_uses_official_low_health_colours() {
        assert_eq!(legacy_health_tint(1.0), [255; 3]);
        assert_eq!(legacy_health_tint(0.5), [255; 3]);
        assert_eq!(legacy_health_tint(0.25), [128; 3]);
        assert_eq!(legacy_health_tint(0.2), [102; 3]);
        assert_eq!(legacy_health_tint(0.1), [128, 0, 0]);
        assert_eq!(legacy_health_tint(0.0), [255, 0, 0]);
    }
}
