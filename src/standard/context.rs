//! Render context, difficulty, skin, combo info, row timing, visible indexes.

use crate::canvas::Img;
use crate::errors::{PreviewError, Result};
use crate::models::{Beatmap, BreakPeriod, HitObjects, StandardHitObject};
use crate::mods::ModSettings;
use crate::parser::round_half_even;
use crate::common::time_selection::PreviewTimeSelector;
use std::collections::HashMap;
use std::sync::Arc;

use super::constants::*;
use super::slider::SliderRenderData;

// ——— helpers ———

#[inline]
pub(crate) fn py_round(v: f64) -> i64 {
    round_half_even(v)
}

#[inline]
pub(crate) fn color_id(base: u64, color: [u8; 3]) -> u64 {
    base | (color[0] as u64) << 32 | (color[1] as u64) << 40 | (color[2] as u64) << 48
}

// ——— data structs ———

#[derive(Clone, Copy)]
pub(crate) struct FrameLayout {
    pub(crate) playfield_left: f64,
    pub(crate) playfield_top: f64,
    pub(crate) scale: f64,
}

#[derive(Clone, Copy)]
pub(crate) struct ComboInfo {
    pub(crate) color: [u8; 3],
    pub(crate) number: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct RenderSettings {
    pub(crate) circle_diameter: i64,
    pub(crate) preempt_ms: i64,
    pub(crate) fade_in_ms: f64,
    pub(crate) hidden: bool,
    pub(crate) traceable: bool,
}

pub(crate) struct CachedLayer {
    pub(crate) image: Img,
    pub(crate) offset: (i64, i64),
}

#[derive(Default)]
pub(crate) struct RenderCache {
    pub(crate) resized_alpha: HashMap<(u64, (u32, u32), u8), Img>,
    pub(crate) procedural: HashMap<(u64, [u8; 3]), Img>,
    pub(crate) slider_data: HashMap<usize, Arc<SliderRenderData>>,
    pub(crate) slider_body_layers: HashMap<(usize, bool), CachedLayer>,
    pub(crate) slider_body_alpha_layers: HashMap<(usize, u8), Img>,
    pub(crate) reverse_arrows: HashMap<(i64, [u8; 3]), Img>,
    pub(crate) reverse_edges: HashMap<(i64,), Img>,
}

/// std 渲染使用的皮肤参数。
pub(crate) struct Skin {
    /// 0-9 数字位图（程序化生成，已裁剪到字形边界）。
    pub(crate) digit_crops: Vec<&'static Img>,
    /// combo 数字重叠量（来自 skin.ini [Fonts] HitCircleOverlap）。
    pub(crate) hitcircle_overlap: i64,
    /// combo 颜色（谱面 [Colours] 优先，否则用 skin.ini 的配色）。
    pub(crate) combo_colors: Vec<[u8; 3]>,
}

pub(crate) struct RenderContext {
    pub(crate) hit_objects: Vec<StandardHitObject>,
    pub(crate) combo_info: Vec<ComboInfo>,
    pub(crate) skin: Skin,
    pub(crate) settings: RenderSettings,
    pub(crate) frame_layout: FrameLayout,
    pub(crate) frame_circle_diameter: i64,
    pub(crate) slider_body_width: i64,
    pub(crate) spinner_size: i64,
    pub(crate) slider_follow_size: i64,
    pub(crate) slider_ball_size: i64,
}

pub(crate) struct RowTiming {
    pub(crate) start_time: i64,
    pub(crate) is_preview: bool,
    pub(crate) break_periods: Vec<BreakPeriod>,
}

// ——— object helpers ———

pub(crate) fn standard_objects(beatmap: &Beatmap) -> Result<Vec<StandardHitObject>> {
    match &beatmap.hit_objects {
        HitObjects::Standard(v) if !v.is_empty() => Ok(v.clone()),
        HitObjects::Standard(_) => Err(PreviewError::render("standard beatmap has no hit objects")),
        _ => Err(PreviewError::render("beatmap is not an osu!standard beatmap")),
    }
}

pub(crate) fn apply_standard_object_mods(
    hit_objects: Vec<StandardHitObject>,
    mods: Option<&ModSettings>,
) -> Vec<StandardHitObject> {
    let hard_rock = mods.map(|m| m.hard_rock).unwrap_or(false);
    if !hard_rock {
        return hit_objects;
    }
    hit_objects
        .into_iter()
        .map(|mut ho| {
            ho.y = PLAYFIELD_HEIGHT as i32 - ho.y;
            ho.slider_points = ho
                .slider_points
                .iter()
                .map(|&(x, y)| (x, PLAYFIELD_HEIGHT as i32 - y))
                .collect();
            ho
        })
        .collect()
}

// ——— difficulty ———

struct EffectiveDifficulty {
    circle_size: f64,
    approach_rate: f64,
}

fn effective_difficulty(beatmap: &Beatmap, mods: Option<&ModSettings>) -> EffectiveDifficulty {
    let od = beatmap.difficulty.get_f64_or("OverallDifficulty", 5.0);
    let mut cs = beatmap.difficulty.get_f64_or("CircleSize", 5.0);
    let mut ar = beatmap.difficulty.get_f64("ApproachRate").unwrap_or(od);

    if let Some(m) = mods {
        if m.easy {
            cs *= 0.5;
            ar *= 0.5;
        }
        if m.hard_rock {
            cs = (cs * 1.3).min(10.0);
            ar = (ar * 1.4).min(10.0);
        }
        if m.has_da() {
            if let Some(v) = m.da_cs {
                cs = v;
            }
            if let Some(v) = m.da_ar {
                ar = v;
            }
        }
    }
    EffectiveDifficulty {
        circle_size: cs,
        approach_rate: ar,
    }
}

pub(crate) fn build_render_settings(
    beatmap: &Beatmap,
    mods: Option<&ModSettings>,
) -> RenderSettings {
    let difficulty = effective_difficulty(beatmap, mods);
    let scale = (1.0 - 0.7 * ((difficulty.circle_size - 5.0) / 5.0)) / 2.0
        * BROKEN_GAMEFIELD_ROUNDING_ALLOWANCE;
    let circle_radius = OBJECT_RADIUS * scale;
    let circle_diameter = py_round(circle_radius * 2.0).max(1);
    let preempt_ms = difficulty_range_int(difficulty.approach_rate, 1800, 1200, 450);
    let hidden = mods.map(|m| m.hidden).unwrap_or(false);
    let traceable = mods.map(|m| m.traceable).unwrap_or(false);
    let fade_in_ms = if hidden {
        preempt_ms as f64 * 0.4
    } else {
        400.0 * (preempt_ms as f64 / 450.0).min(1.0)
    };
    RenderSettings {
        circle_diameter,
        preempt_ms,
        fade_in_ms,
        hidden,
        traceable,
    }
}

fn difficulty_range_int(difficulty: f64, minimum: i64, middle: i64, maximum: i64) -> i64 {
    if difficulty > 5.0 {
        (middle as f64 + (maximum - middle) as f64 * ((difficulty - 5.0) / 5.0)) as i64
    } else if difficulty < 5.0 {
        (middle as f64 + (middle - minimum) as f64 * ((difficulty - 5.0) / 5.0)) as i64
    } else {
        middle
    }
}

/// 计算 playfield 在单帧中的位置与缩放，与游戏内 1080p（16:9）布局一致。
///
/// lazer 在 16:9 窗口下的布局推导（OsuPlayfieldAdjustmentContainer）：
/// 游戏空间为 1365.33×768，playfield 容器取 80% 后按 4:3 适配，
/// 得到 819.2×614.4，即 512×384 的 1.6 倍，居中放置并整体下移 8×scale
/// （与 storyboard 对齐的历史偏移）。本帧 683×384 恰为游戏空间的一半，
/// 因此缩放为 0.8，上下左右留白与游戏内完全等比。
pub(crate) fn build_frame_layout() -> FrameLayout {
    let scale = PLAYFIELD_VIEWPORT_RATIO;
    let playfield_width = PLAYFIELD_WIDTH * scale;
    let playfield_height = PLAYFIELD_HEIGHT * scale;
    FrameLayout {
        playfield_left: (IMAGE_WIDTH as f64 - playfield_width) / 2.0,
        playfield_top: (IMAGE_HEIGHT as f64 - playfield_height) / 2.0
            + PLAYFIELD_STORYBOARD_SHIFT * scale,
        scale,
    }
}

pub(crate) fn build_combo_info(
    hit_objects: &[StandardHitObject],
    combo_colors: &[[u8; 3]],
) -> Vec<ComboInfo> {
    let mut combo_info = Vec::with_capacity(hit_objects.len());
    let mut color_index: usize = 0;
    let mut number: u32 = 0;
    let mut previous_was_spinner = false;

    for (index, hit_object) in hit_objects.iter().enumerate() {
        let is_spinner = hit_object.hit_type & 8 != 0;
        let starts_combo =
            index == 0 || previous_was_spinner || (hit_object.new_combo && !is_spinner);
        if starts_combo {
            if index > 0 {
                color_index = (color_index + hit_object.combo_offset as usize + 1)
                    % combo_colors.len();
            }
            number = 1;
        } else {
            number += 1;
        }
        combo_info.push(ComboInfo {
            color: combo_colors[color_index],
            number,
        });
        previous_was_spinner = is_spinner;
    }
    combo_info
}

/// 加载 std 皮肤：数字位图程序化生成，颜色与重叠量来自统一 skin.ini。
pub(crate) fn load_skin(beatmap: &Beatmap) -> Skin {
    let skin_config = crate::skin::skin();
    let digit_crops = (0..10).map(super::digits::digit_image).collect();
    // 谱面自带 [Colours] 时优先使用；否则用 skin.ini 配色；都没有则回退 Argon 默认
    let combo_colors = if !beatmap.combo_colors.is_empty() {
        beatmap.combo_colors.clone()
    } else if !skin_config.combo_colors.is_empty() {
        skin_config.combo_colors.clone()
    } else {
        ARGON_COMBO_COLORS.to_vec()
    };
    Skin {
        digit_crops,
        hitcircle_overlap: skin_config.hitcircle_overlap,
        combo_colors,
    }
}

pub(crate) fn build_render_context(
    beatmap: &Beatmap,
    hit_objects: Vec<StandardHitObject>,
    mods: Option<&ModSettings>,
) -> RenderContext {
    let skin = load_skin(beatmap);
    let settings = build_render_settings(beatmap, mods);
    let frame_layout = build_frame_layout();
    let combo_info = build_combo_info(&hit_objects, &skin.combo_colors);
    let frame_circle_diameter =
        py_round(settings.circle_diameter as f64 * frame_layout.scale).max(1);
    RenderContext {
        hit_objects,
        combo_info,
        skin,
        settings,
        frame_layout,
        frame_circle_diameter,
        slider_body_width: py_round(
            settings.circle_diameter as f64 * ARGON_SLIDER_WIDTH_RATIO * frame_layout.scale,
        )
        .max(1),
        spinner_size: py_round(
            PLAYFIELD_WIDTH.min(PLAYFIELD_HEIGHT) * 0.95 * frame_layout.scale,
        )
        .max(1),
        slider_follow_size: py_round(
            settings.circle_diameter as f64 * 2.4 * frame_layout.scale,
        )
        .max(1),
        slider_ball_size: py_round(
            settings.circle_diameter as f64 * ARGON_SLIDER_WIDTH_RATIO * frame_layout.scale,
        )
        .max(1),
    }
}

// ——— row timing ———

pub(crate) fn choose_row_start_times(
    beatmap: &Beatmap,
    hit_objects: &[StandardHitObject],
    row_count: usize,
    images_per_row: usize,
    ms_per_row_duration: i64,
    requested_start_times: Option<Vec<i64>>,
) -> Result<Vec<RowTiming>> {
    let row_duration = (images_per_row as i64 - 1) * ms_per_row_duration;
    let spans: Vec<(i64, i64)> = hit_objects
        .iter()
        .map(|o| (o.start_time, o.end_time))
        .collect();
    let chosen = PreviewTimeSelector::new(
        beatmap,
        spans,
        row_count,
        row_duration,
        requested_start_times,
    )?
    .choose()?;
    Ok(chosen
        .into_iter()
        .map(|t| RowTiming {
            start_time: t.start_time,
            is_preview: t.is_preview,
            break_periods: t.break_periods,
        })
        .collect())
}

// ——— canvas sizes ———

pub(crate) fn png_canvas_size() -> (i64, i64) {
    let width = HORIZONTAL_PAGE_MARGIN * 2
        + PNG_IMAGES_PER_ROW as i64 * IMAGE_WIDTH
        + (PNG_IMAGES_PER_ROW as i64 - 1) * INTRA_ROW_IMAGE_GAP;
    let row_height = IMAGE_HEIGHT + TIME_LABEL_TOP_GAP + TIME_LABEL_HEIGHT;
    let height = VERTICAL_PAGE_MARGIN * 2
        + PNG_ROW_COUNT as i64 * row_height
        + (PNG_ROW_COUNT as i64 - 1) * INTER_ROW_GAP;
    (width, height)
}

pub(crate) fn gif_canvas_size() -> (i64, i64) {
    let row_height = IMAGE_HEIGHT + TIME_LABEL_TOP_GAP + TIME_LABEL_HEIGHT;
    let width = HORIZONTAL_PAGE_MARGIN * 2
        + GIF_IMAGES_PER_ROW as i64 * IMAGE_WIDTH
        + (GIF_IMAGES_PER_ROW as i64 - 1) * GIF_GRID_GAP;
    let height = VERTICAL_PAGE_MARGIN * 2
        + GIF_ROW_COUNT as i64 * row_height
        + (GIF_ROW_COUNT as i64 - 1) * GIF_GRID_GAP;
    (width, height)
}

pub(crate) fn gif_frame_origin(segment_index: usize) -> (i64, i64) {
    let row_index = (segment_index / GIF_IMAGES_PER_ROW) as i64;
    let image_index = (segment_index % GIF_IMAGES_PER_ROW) as i64;
    let row_height = IMAGE_HEIGHT + TIME_LABEL_TOP_GAP + TIME_LABEL_HEIGHT;
    let x = HORIZONTAL_PAGE_MARGIN + image_index * (IMAGE_WIDTH + GIF_GRID_GAP);
    let y = VERTICAL_PAGE_MARGIN + row_index * (row_height + GIF_GRID_GAP);
    (x, y)
}

// ——— visible indexes ———

pub(crate) fn build_visible_indexes_by_snapshot(
    hit_objects: &[StandardHitObject],
    snapshot_times: &[i64],
    preempt_ms: i64,
) -> Vec<Vec<usize>> {
    let mut visible_starts: Vec<(i64, usize)> = hit_objects
        .iter()
        .enumerate()
        .map(|(i, o)| (o.start_time - preempt_ms, i))
        .collect();
    visible_starts.sort_unstable();
    let mut visible_ends: Vec<(i64, usize)> = hit_objects
        .iter()
        .enumerate()
        .map(|(i, o)| (visible_end_time(o), i))
        .collect();
    visible_ends.sort_unstable();

    let mut active_indexes: Vec<usize> = Vec::new();
    let mut start_pointer = 0usize;
    let mut end_pointer = 0usize;
    let mut visible_groups = Vec::with_capacity(snapshot_times.len());

    for &snapshot_time in snapshot_times {
        while start_pointer < visible_starts.len()
            && visible_starts[start_pointer].0 <= snapshot_time
        {
            active_indexes.push(visible_starts[start_pointer].1);
            start_pointer += 1;
        }
        while end_pointer < visible_ends.len() && visible_ends[end_pointer].0 < snapshot_time {
            let ended_index = visible_ends[end_pointer].1;
            if let Some(pos) = active_indexes.iter().position(|&v| v == ended_index) {
                active_indexes.remove(pos);
            }
            end_pointer += 1;
        }
        visible_groups.push(active_indexes.iter().rev().copied().collect());
    }
    visible_groups
}

pub(crate) fn visible_end_time(hit_object: &StandardHitObject) -> i64 {
    if hit_object.hit_type & 2 != 0 {
        return hit_object.end_time + SLIDER_FADE_OUT_MS;
    }
    if hit_object.hit_type & 8 != 0 {
        return hit_object.end_time + SPINNER_FADE_OUT_MS;
    }
    hit_object.start_time + POST_HIT_FADE_MS
}

// ——— coordinate transform ———

pub(crate) fn to_frame_point(x: f64, y: f64, frame_layout: &FrameLayout) -> (f64, f64) {
    (
        frame_layout.playfield_left + x * frame_layout.scale,
        frame_layout.playfield_top + y * frame_layout.scale,
    )
}
