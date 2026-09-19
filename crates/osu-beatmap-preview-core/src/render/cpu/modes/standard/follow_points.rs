//! osu!standard 跟随点（follow point）：连接关系、生命周期与 Argon 图标。
//!
//! 对照 lazer 的 `FollowPointRenderer` / `FollowPointConnection` / `ArgonFollowPoint`：
//! 相邻两个物件之间按 32 个 playfield 单位的间距铺一串跟随点，每个跟随点在
//! 「淡出时刻减去 preempt」时淡入，位置从连接上的 `fraction - 0.1` 滑到 `fraction`，
//! 到物件开始时间前淡出。转盘两端和新连击的起点不产生跟随点。
//!
//! 已知偏差：lazer 的 `ArgonFollowPoint` 使用加法混合，而这里的画布与实时场景都
//! 只有 src-over。跟随点位于物件层之下、又贴着深色的游玩区域，两种混合方式在
//! 不透明底色上结果一致，只有叠在谱面背景图上时亮度略有差别。

use crate::domain::models::StandardHitObject;
use crate::domain::shared::slider_path::build_standard_slider_path;
use crate::render::canvas::Img;

use super::constants::*;
use super::context::{py_round, stack_offset, RenderSettings};

/// 连接上的单个跟随点。
///
/// 位置是世界坐标（512×384 playfield），与输出格式无关：绘制时再按当前
/// `frame_layout.scale` 换算，因此 PNG/GIF/MP4 与实时预览共用同一份数据。
pub struct FollowPoint {
    /// 淡入开始时的位置（连接上 `fraction - 0.1` 处）。
    pub from: (f64, f64),
    /// 淡入结束时的位置（连接上 `fraction` 处）。
    pub to: (f64, f64),
    /// 朝向（度，屏幕坐标 y 向下）：图标指向连接终点。
    pub rotation: f64,
    /// 淡入时刻（绝对毫秒）。
    pub fade_in_time: f64,
    /// 淡出时刻（绝对毫秒）。
    pub fade_out_time: f64,
    /// 淡入与淡出各自的时长（毫秒，等于 `end.TimeFadeIn`）。
    pub fade_duration: f64,
}

/// 展开整张谱面的跟随点。
///
/// 连接关系与 lazer `FollowPointRenderer.addEntry` 一致：按开始时间稳定排序后
/// 相邻相连（等时物件保持谱面顺序，避免连接在等时物件之间来回切换）。
pub fn build_follow_points(
    objects: &[StandardHitObject],
    settings: &RenderSettings,
) -> Vec<FollowPoint> {
    let mut order: Vec<usize> = (0..objects.len()).collect();
    order.sort_by_key(|&index| objects[index].start_time);

    let mut points = Vec::new();
    for pair in order.windows(2) {
        let start = &objects[pair[0]];
        let end = &objects[pair[1]];
        // `FollowPointLifetimeEntry.refreshLifetimes`：转盘两端与新连击起点都没有跟随点。
        if is_spinner(start) || is_spinner(end) || end.new_combo {
            continue;
        }

        let start_position = stacked_end_position(start, settings);
        let end_position = stacked_position(end, settings);
        let offset_x = end_position.0 - start_position.0;
        let offset_y = end_position.1 - start_position.1;
        // 距离按 lazer 的 `(int)distanceVector.Length` 截断取整；循环在
        // `distance - SPACING` 之前停下，连接末端至少留出一个间距。
        // 距离不足时循环不执行，因此不会出现除零。
        let distance = offset_x.hypot(offset_y) as i64;
        let start_time = start.end_time as f64;
        let duration = end.start_time as f64 - start_time;
        let fade_duration = follow_fade_duration(settings);
        let preempt = follow_preempt(settings);
        let rotation = offset_y.atan2(offset_x).to_degrees();

        let mut travelled = (FOLLOW_POINT_SPACING as f64 * 1.5) as i64;
        while travelled < distance - FOLLOW_POINT_SPACING {
            let fraction = travelled as f64 / distance as f64;
            let fade_out_time = start_time + fraction * duration;
            points.push(FollowPoint {
                from: (
                    start_position.0 + (fraction - 0.1) * offset_x,
                    start_position.1 + (fraction - 0.1) * offset_y,
                ),
                to: (
                    start_position.0 + fraction * offset_x,
                    start_position.1 + fraction * offset_y,
                ),
                rotation,
                fade_in_time: fade_out_time - preempt,
                fade_out_time,
                fade_duration,
            });
            travelled += FOLLOW_POINT_SPACING;
        }
    }
    points
}

/// 跟随点的淡出提前量：`PREEMPT * min(1, TimePreempt / PREEMPT_MIN)`。
///
/// AR>10（含 mod）时 preempt 会低于 450ms，这里与 lazer 一样把跟随点动画整体
/// 按同一比例缩短，而不是沿用 800/400 的固定值。
fn follow_preempt(settings: &RenderSettings) -> f64 {
    FOLLOW_POINT_PREEMPT_MS * follow_speed_ratio(settings)
}

/// 跟随点的淡入与淡出时长：`TimeFadeIn = 400 * min(1, TimePreempt / PREEMPT_MIN)`。
///
/// 注意不能用 [`RenderSettings::fade_in_ms`]：那个值在 Hidden 下会改成
/// `preempt * 0.4`，而跟随点不受 Hidden 影响，始终使用物件自己的 `TimeFadeIn`。
fn follow_fade_duration(settings: &RenderSettings) -> f64 {
    FOLLOW_POINT_FADE_IN_MS * follow_speed_ratio(settings)
}

fn follow_speed_ratio(settings: &RenderSettings) -> f64 {
    (settings.preempt_ms as f64 / FOLLOW_POINT_PREEMPT_MIN_MS).min(1.0)
}

/// 计算某一时刻跟随点的 `(透明度, 位置进度)`。
///
/// 透明度是 `FadeIn`/`FadeOut` 的线性插值（`Easing.None`）；位置进度是
/// `MoveTo`/`ScaleTo` 使用的 `Easing.Out`（二次缓出），用于插值位置与整体缩放。
/// 生命周期之外返回 `(0, 0)`。
pub fn follow_point_state(point: &FollowPoint, time: i64) -> (f64, f64) {
    // 极端 Difficulty Adjust 的 AR 会把 preempt 推到 0 或负值，淡入淡出时长随之
    // 失去意义；直接视为不显示，避免用非正时长做除法产生 NaN。
    if point.fade_duration <= 0.0 {
        return (0.0, 0.0);
    }
    let now = time as f64;
    let fade_in_end = point.fade_in_time + point.fade_duration;
    if now < point.fade_in_time || now >= point.fade_out_time + point.fade_duration {
        return (0.0, 0.0);
    }

    let progress = if now < fade_in_end {
        out_quad(((now - point.fade_in_time) / point.fade_duration).clamp(0.0, 1.0))
    } else {
        1.0
    };
    let alpha = if now < fade_in_end {
        (now - point.fade_in_time) / point.fade_duration
    } else if now < point.fade_out_time {
        1.0
    } else {
        1.0 - (now - point.fade_out_time) / point.fade_duration
    };
    (alpha.clamp(0.0, 1.0), progress)
}

/// 跟随点当前的世界坐标。
pub fn follow_point_position(point: &FollowPoint, progress: f64) -> (f64, f64) {
    (
        point.from.0 + (point.to.0 - point.from.0) * progress,
        point.from.1 + (point.to.1 - point.from.1) * progress,
    )
}

/// 跟随点图标的像素高度。
///
/// 图标外框是 8 个 playfield 单位，整体缩放为
/// `物件缩放 * (1.5 → 1)`（`ScaleTo(end.Scale, end.TimeFadeIn, Easing.Out)`）。
pub fn follow_point_height(settings: &RenderSettings, frame_scale: f64, progress: f64) -> f64 {
    let scale = FOLLOW_POINT_SCALE_START - (FOLLOW_POINT_SCALE_START - 1.0) * progress;
    FOLLOW_POINT_ICON_SIZE * settings.object_scale * scale * frame_scale
}

/// `Easing.Out`（OutQuad）缓动。
fn out_quad(progress: f64) -> f64 {
    progress * (2.0 - progress)
}

fn is_spinner(hit_object: &StandardHitObject) -> bool {
    hit_object.hit_type & 8 != 0
}

/// `StackedPosition`：物件头部位置叠加堆叠偏移。
fn stacked_position(hit_object: &StandardHitObject, settings: &RenderSettings) -> (f64, f64) {
    let offset = stack_offset(hit_object, settings);
    (hit_object.x as f64 + offset, hit_object.y as f64 + offset)
}

/// `StackedEndPosition`：圆是圆心，滑条是路径末端，两者都叠加堆叠偏移。
fn stacked_end_position(hit_object: &StandardHitObject, settings: &RenderSettings) -> (f64, f64) {
    if hit_object.hit_type & 2 == 0 {
        return stacked_position(hit_object, settings);
    }
    let offset = stack_offset(hit_object, settings);
    let path = build_standard_slider_path(
        hit_object.x,
        hit_object.y,
        &hit_object.slider_points,
        hit_object.slider_type.as_deref().unwrap_or("B"),
        hit_object.slider_pixel_length,
    );
    let tail = path
        .points
        .last()
        .copied()
        .unwrap_or((hit_object.x as f64, hit_object.y as f64));
    (tail.0 + offset, tail.1 + offset)
}

// ——— 图标 ———

/// 图标超采样倍数。
///
/// chevron 的笔画在正常输出尺寸下不足一个像素，而画布的折线光栅化只按像素中心
/// 覆盖、没有抗锯齿，直接画会整条丢失；先按该倍数放大绘制再 Lanczos 降采样，
/// 与滑条主体使用超采样的思路相同。
const SPRITE_SUPERSAMPLE: f64 = 8.0;

/// 跟随点图标（含旋转余量）的外接方框边长。
fn sprite_side(height: f64) -> f64 {
    let height = height.max(1.0);
    let width = height * (FOLLOW_POINT_CHEVRON_ASPECT + FOLLOW_POINT_CHEVRON_GAP_RATIO);
    width.hypot(height)
}

/// 程序化绘制 Argon 跟随点图标（指向 +X，未旋转）。
///
/// 对照 lazer `ArgonFollowPoint`：两个 FontAwesome Solid `ChevronRight`
/// （`SpriteIcon` 尺寸 8），第二个相对第一个沿 +X 偏移半个图标高度；整体套用
/// `FC618F → BB1A41` 的竖向渐变，前一个 chevron 再乘 `OsuColour.Gray(0.2)` 压暗。
/// 渐变作用在旋转前的容器上，因此随图标一起旋转。
///
/// `rotation_deg` 直接烘焙进光栅化，避免对几像素大的成品再做一次重采样。
pub fn build_sprite(height: f64, rotation_deg: f64) -> Img {
    let side = sprite_side(height);
    let canvas = (side * SPRITE_SUPERSAMPLE).ceil().max(1.0) as u32;
    // 用实际画布尺寸反推放大倍数，保证取整后图标仍然正好铺满外接方框。
    let scale = canvas as f64 / side;
    let height = height.max(1.0) * scale;
    let width = height * FOLLOW_POINT_CHEVRON_ASPECT;
    let thickness = (height * FOLLOW_POINT_CHEVRON_THICKNESS_RATIO).max(1.0);
    let gap = height * FOLLOW_POINT_CHEVRON_GAP_RATIO;
    let center = canvas as f64 / 2.0;
    let (sin, cos) = rotation_deg.to_radians().sin_cos();
    let rotate = |local: (f64, f64)| {
        (
            center + local.0 * cos - local.1 * sin,
            center + local.0 * sin + local.1 * cos,
        )
    };

    let mut layer = Img::new(canvas, canvas, [0, 0, 0, 0]);
    // 折线的中线盒 = 墨迹盒在四周各内缩半个笔画（端帽与折角都按圆头处理）。
    let arm = (width - thickness) / 2.0;
    let half = (height - thickness) / 2.0;
    for offset in [-gap / 2.0, gap / 2.0] {
        layer.stroke_polyline(
            &[
                rotate((offset - arm, -half)),
                rotate((offset + arm, 0.0)),
                rotate((offset - arm, half)),
            ],
            thickness,
            [255, 255, 255, 255],
            true,
        );
    }

    // 竖向渐变按图标自身坐标系取色：逐像素逆旋转回局部坐标再查渐变。
    for y in 0..canvas {
        for x in 0..canvas {
            let index = layer.idx(x, y);
            if layer.data[index + 3] == 0 {
                continue;
            }
            let offset_x = x as f64 + 0.5 - center;
            let offset_y = y as f64 + 0.5 - center;
            let local_x = offset_x * cos + offset_y * sin;
            let local_y = -offset_x * sin + offset_y * cos;
            let color = gradient((local_y / height + 0.5).clamp(0.0, 1.0));
            // 拖后的 chevron（局部 x 为负）叠加 Gray(0.2)，与 lazer 一致。
            let factor = if local_x < 0.0 {
                ARGON_FOLLOW_POINT_DIM
            } else {
                1.0
            };
            layer.data[index] = (color[0] as f64 * factor).round() as u8;
            layer.data[index + 1] = (color[1] as f64 * factor).round() as u8;
            layer.data[index + 2] = (color[2] as f64 * factor).round() as u8;
        }
    }

    let target = (side.round() as u32).max(1);
    layer.resize(target, target)
}

/// 跟随点图标的竖向渐变：`t = 0` 为图标顶部。
fn gradient(t: f64) -> [u8; 3] {
    let mix = |top: u8, bottom: u8| {
        (top as f64 + (bottom as f64 - top as f64) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [
        mix(ARGON_FOLLOW_POINT_TOP[0], ARGON_FOLLOW_POINT_BOTTOM[0]),
        mix(ARGON_FOLLOW_POINT_TOP[1], ARGON_FOLLOW_POINT_BOTTOM[1]),
        mix(ARGON_FOLLOW_POINT_TOP[2], ARGON_FOLLOW_POINT_BOTTOM[2]),
    ]
}

/// 图标在当前时刻的整数像素高度：CPU 路径按整数高度缓存精灵。
pub fn sprite_height_key(settings: &RenderSettings, frame_scale: f64, progress: f64) -> i64 {
    py_round(follow_point_height(settings, frame_scale, progress).max(1.0)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circle(x: i32, y: i32, start_time: i64, new_combo: bool) -> StandardHitObject {
        StandardHitObject {
            x,
            y,
            start_time,
            end_time: start_time,
            hit_type: 1,
            new_combo,
            ..Default::default()
        }
    }

    fn settings(preempt_ms: i64) -> RenderSettings {
        RenderSettings {
            circle_diameter: 128,
            object_scale: 1.0,
            preempt_ms,
            fade_in_ms: 400.0,
            hidden: false,
            traceable: false,
        }
    }

    /// 水平方向相距 300 个 playfield 单位、相隔 1000ms 的两个圆圈。
    fn two_circles() -> Vec<StandardHitObject> {
        vec![circle(100, 192, 1000, false), circle(400, 192, 2000, false)]
    }

    fn ink_bounds(image: &Img) -> (u32, u32, u32, u32) {
        image.alpha_bbox().expect("图标必须有不透明像素")
    }

    #[test]
    fn spacing_matches_the_game_loop() {
        let points = build_follow_points(&two_circles(), &settings(1200));
        // 距离 300：48、80、…、240 共 7 个（d < distance - SPACING）。
        assert_eq!(points.len(), 7);
        let first = &points[0];
        assert!((first.to.0 - 148.0).abs() < 1e-9);
        assert!((first.from.0 - 118.0).abs() < 1e-9);
        assert_eq!(first.rotation, 0.0);
    }

    #[test]
    fn fade_times_use_preempt_and_fraction() {
        let points = build_follow_points(&two_circles(), &settings(1200));
        let first = &points[0];
        // duration = 1000ms，fraction = 48/300，preempt = 800ms，TimeFadeIn = 400ms。
        assert!((first.fade_out_time - 1160.0).abs() < 1e-9);
        assert!((first.fade_in_time - 360.0).abs() < 1e-9);
        assert!((first.fade_duration - 400.0).abs() < 1e-9);

        // AR>10 时 preempt 低于 450ms，跟随点动画按同一比例整体缩短。
        let fast = build_follow_points(&two_circles(), &settings(300));
        assert!((fast[0].fade_duration - 400.0 * 300.0 / 450.0).abs() < 1e-9);
        assert!((follow_preempt(&settings(300)) - 800.0 * 300.0 / 450.0).abs() < 1e-9);
    }

    #[test]
    fn spinners_and_new_combo_breaks_connections() {
        let mut spinner = circle(256, 192, 2000, false);
        spinner.hit_type = 8;
        spinner.end_time = 3000;

        let mut combo_break = two_circles();
        combo_break[1].new_combo = true;
        assert!(build_follow_points(&combo_break, &settings(1200)).is_empty());

        let with_spinner = vec![circle(100, 192, 1000, false), spinner];
        assert!(build_follow_points(&with_spinner, &settings(1200)).is_empty());
    }

    #[test]
    fn close_objects_produce_no_follow_points() {
        let objects = vec![circle(100, 192, 1000, false), circle(180, 192, 2000, false)];
        // 距离 80 只够放下一个间距的开头（48 < 80 - 32 = 48 不成立）。
        assert!(build_follow_points(&objects, &settings(1200)).is_empty());
    }

    #[test]
    fn slider_connections_start_at_the_slider_tail() {
        let slider = StandardHitObject {
            x: 100,
            y: 192,
            start_time: 1000,
            end_time: 2000,
            hit_type: 2,
            slider_type: Some("L".to_string()),
            slider_points: vec![(400, 192)],
            slider_pixel_length: 300.0,
            ..Default::default()
        };
        let objects = vec![slider, circle(600, 192, 3000, false)];
        let points = build_follow_points(&objects, &settings(1200));

        // 连接从滑条末端 (400, 192) 到 (600, 192)，距离 200：48、80、112、144。
        assert_eq!(points.len(), 4);
        assert!((points[0].to.0 - 448.0).abs() < 1e-9);
        assert!((points[0].from.0 - 428.0).abs() < 1e-9);
    }

    #[test]
    fn alpha_fades_in_holds_and_fades_out() {
        let points = build_follow_points(&two_circles(), &settings(1200));
        let point = &points[0];
        assert_eq!(follow_point_state(point, 359), (0.0, 0.0));
        assert_eq!(follow_point_state(point, 360), (0.0, 0.0));
        assert_eq!(follow_point_state(point, 560).0, 0.5);
        assert_eq!(follow_point_state(point, 760).0, 1.0);
        assert_eq!(follow_point_state(point, 1160).0, 1.0);
        assert_eq!(follow_point_state(point, 1360).0, 0.5);
        assert_eq!(follow_point_state(point, 1560), (0.0, 0.0));
        // 位置在淡入期间按 OutQuad 缓动：中点进度 0.5 -> 0.75。
        let (_, progress) = follow_point_state(point, 560);
        assert!((progress - 0.75).abs() < 1e-9);
    }

    #[test]
    fn position_lerps_between_fraction_and_previous_tenth() {
        let points = build_follow_points(&two_circles(), &settings(1200));
        let point = &points[0];
        assert_eq!(follow_point_position(point, 0.0), (118.0, 192.0));
        assert_eq!(follow_point_position(point, 1.0), (148.0, 192.0));
        assert_eq!(follow_point_position(point, 0.5), (133.0, 192.0));
    }

    #[test]
    fn icon_height_shrinks_from_one_and_a_half_scale() {
        let settings = settings(1200);
        let start = follow_point_height(&settings, 0.8, 0.0);
        assert!((start - FOLLOW_POINT_ICON_SIZE * 1.5 * 0.8).abs() < 1e-9);
        assert!(
            (follow_point_height(&settings, 0.8, 1.0) - FOLLOW_POINT_ICON_SIZE * 0.8).abs() < 1e-9
        );
        assert_eq!(sprite_height_key(&settings, 0.8, 0.0), 10);
        assert_eq!(sprite_height_key(&settings, 0.8, 1.0), 6);
    }

    #[test]
    fn sprite_is_centred_and_dim_chevron_trails() {
        let sprite = build_sprite(8.0, 0.0);
        assert_eq!(sprite.w, sprite.h);
        let (left, top, right, bottom) = ink_bounds(&sprite);
        // 墨迹在方框内居中（允许 1px 取整误差）。
        let left_margin = left as i64;
        let right_margin = sprite.w as i64 - right as i64;
        assert!((left_margin - right_margin).abs() <= 1);
        assert!((top as i64 - (sprite.h as i64 - bottom as i64)).abs() <= 1);

        // 两个 chevron：朝向 +X 时拖后的那个（左半）明显更暗。
        let centre = sprite.w as i64 / 2;
        let mean_red = |range: std::ops::Range<i64>| {
            let mut sum = 0.0;
            let mut count = 0.0;
            for y in top..bottom {
                for x in range.clone() {
                    let pixel = sprite.get(x as u32, y);
                    if pixel[3] > 0 {
                        sum += pixel[0] as f64;
                        count += 1.0;
                    }
                }
            }
            assert!(count > 0.0, "左右两半都必须有墨迹");
            sum / count
        };
        assert!(mean_red(0..centre) < mean_red(centre..sprite.w as i64) * 0.6);
    }

    #[test]
    fn rotation_turns_the_icon_towards_the_connection() {
        // 水平连接：两个 chevron 左右排列，墨迹总宽大于高。
        let horizontal = build_sprite(40.0, 0.0);
        let (left, top, right, bottom) = ink_bounds(&horizontal);
        assert!(right - left > bottom - top);

        // 竖直向下（rotation = 90°）时上下排列，墨迹总高大于宽。
        let vertical = build_sprite(40.0, 90.0);
        let (left, top, right, bottom) = ink_bounds(&vertical);
        assert!(bottom - top > right - left);
    }
}
