//! 静态回放共用的 ACC 文字与评级徽标，保持各模式的颜色及字形一致。

use crate::{
    gameplay::state::GameplayState,
    processing::parse::round_half_even,
    render::{
        canvas::Img,
        text::{draw_text, text_size},
    },
};

const WHITE: [u8; 4] = [255, 255, 255, 255];
const MARKER_BACKGROUND: [u8; 4] = [20, 8, 14, 255];

pub(crate) use super::colors::rank as rank_color;

fn draw_state_text(image: &mut Img, x: i64, y: i64, label: &str, font: u32, color: [u8; 4]) {
    // 完整字形只缩放一次，保留小数倍率下的笔画空隙；快照数量有上限，不为整图分配文字图层。
    let (width, height) = text_size(label, 8);
    let mut glyphs = Img::new(width, height, [0; 4]);
    draw_text(&mut glyphs, 0, 0, label, 8, color);
    let target_width = round_half_even(f64::from(width) * f64::from(font) / 8.0).max(1) as u32;
    for dy in 0..font {
        for dx in 0..target_width {
            image.blend_px(
                x + i64::from(dx),
                y + i64::from(dy),
                glyphs.get(dx * width / target_width, dy * height / font),
            );
        }
    }
}

pub(crate) fn badge_width(state: &GameplayState, font: u32, padding: i64) -> i64 {
    let label = format!("{:.2}%", state.accuracy.unwrap_or(1.0) * 100.0);
    let rank = super::hud::display_rank(state.rank.as_deref(), state.status);
    i64::from(text_size(&label, font).0 + text_size(rank, font).0) + padding * 6
}

pub(crate) fn draw_badge(
    image: &mut Img,
    x: i64,
    label_y: i64,
    state: &GameplayState,
    font: u32,
    padding: i64,
    scale: f64,
) {
    let label = format!("{:.2}%", state.accuracy.unwrap_or(1.0) * 100.0);
    let rank = super::hud::display_rank(state.rank.as_deref(), state.status);
    let rank_color = rank_color(rank);
    let height = i64::from(font) + padding * 2;
    image.fill_rect_size(
        x,
        label_y - padding,
        (3.0 * scale).round().max(1.0) as i64,
        height,
        rank_color,
    );
    draw_state_text(image, x + padding * 2, label_y, &label, font, rank_color);
    let badge_x = x + padding * 3 + i64::from(text_size(&label, font).0);
    image.fill_rect_size(
        badge_x,
        label_y - padding,
        i64::from(text_size(rank, font).0) + padding * 2,
        height,
        rank_color,
    );
    draw_state_text(
        image,
        badge_x + padding,
        label_y,
        rank,
        font,
        if matches!(rank, "F" | "-") {
            WHITE
        } else {
            MARKER_BACKGROUND
        },
    );
}

pub(crate) struct StateSampling {
    pub min_interval_ms: f64,
    pub max_per_column: usize,
    pub max_total: usize,
    pub min_accuracy_change: f64,
}

impl Default for StateSampling {
    fn default() -> Self {
        Self {
            min_interval_ms: 5000.0,
            max_per_column: 4,
            max_total: 24,
            min_accuracy_change: 0.001,
        }
    }
}

/// 按状态变化随机稀疏采样，优先保留失败、结算和评级变化。
pub(crate) fn select_states(
    gameplay: &crate::gameplay::timeline::GameplayTimeline,
    start: f64,
    end: f64,
    seed: u32,
    scale: f64,
    locate: impl Fn(f64) -> (i64, i64),
    sampling: StateSampling,
) -> Vec<(f64, &GameplayState)> {
    use crate::domain::shared::legacy_random::LegacyRandom;
    let mut random = LegacyRandom::new(seed);
    let mut previous = &gameplay.initial;
    let mut candidates = Vec::new();
    for (time, state) in &gameplay.checkpoints {
        let status_changed = state.status != previous.status;
        let rank_changed = state.rank != previous.rank || status_changed;
        let accuracy_changed = (state.accuracy.unwrap_or(1.0) - previous.accuracy.unwrap_or(1.0))
            .abs()
            >= sampling.min_accuracy_change;
        if rank_changed || accuracy_changed {
            previous = state;
            if *time >= start && *time <= end {
                let priority = if status_changed {
                    0
                } else if rank_changed {
                    1
                } else {
                    2
                };
                candidates.push((priority, random.next_uint(), *time));
            }
        }
    }
    // 结算与失败优先，其次评级变化；同级随机抽样，同时限制时间和图上的间距。
    candidates.sort_by_key(|c| (c.0, c.1));
    let mut selected: Vec<(f64, &GameplayState)> = Vec::new();
    for (_, _, time) in candidates {
        let (column, y) = locate(time);
        let in_column = selected
            .iter()
            .filter(|(at, _)| locate(*at).0 == column)
            .count();
        if in_column >= sampling.max_per_column || selected.len() >= sampling.max_total {
            continue;
        }
        if selected.iter().any(|(at, _)| {
            let (other_column, other_y) = locate(*at);
            (time - at).abs() < sampling.min_interval_ms
                || (column == other_column && ((y - other_y).abs() as f64) < 56.0 * scale)
        }) {
            continue;
        }
        // 同刻结算快照以最终状态为准。
        selected.push((time, gameplay.state_at(time)));
    }
    selected.sort_by(|a, b| a.0.total_cmp(&b.0));
    selected
}
