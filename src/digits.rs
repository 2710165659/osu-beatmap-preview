//! 程序化 combo 数字绘制（替代原先内嵌的 default-0..9 字体图片）。
//!
//! 数字以「圆头单线笔画」风格在 100×150 的设计网格上定义，
//! 2 倍超采样描边后缩小，得到平滑的白色数字位图，
//! 视觉上接近 lazer Argon 皮肤的圆体数字。

use crate::canvas::Img;
use std::sync::OnceLock;

/// 设计网格尺寸（宽 × 高）。
const CELL_W: f64 = 120.0;
const CELL_H: f64 = 154.0;
/// 设计网格中的笔画宽度（加粗，避免缩小后过细）。
const STROKE: f64 = 27.0;
/// 字形横向加宽系数（绕中轴拉伸，纠正过于细长的观感）。
const X_WIDEN: f64 = 1.22;
/// 超采样倍数（先放大描边再缩小，获得抗锯齿效果）。
const SUPERSAMPLE: f64 = 2.0;

/// 获取数字 0-9 的白色位图（已按字形裁剪到实际边界，全局缓存）。
pub fn digit_image(digit: usize) -> &'static Img {
    static CACHE: OnceLock<Vec<Img>> = OnceLock::new();
    let all = CACHE.get_or_init(|| (0..10).map(render_digit).collect());
    &all[digit.min(9)]
}

/// 在椭圆上取一段圆弧的折线点（角度单位为度，0°指向 +x，90°指向 +y，即屏幕向下）。
fn arc(cx: f64, cy: f64, rx: f64, ry: f64, deg0: f64, deg1: f64) -> Vec<(f64, f64)> {
    let steps = 28;
    (0..=steps)
        .map(|i| {
            let t = (deg0 + (deg1 - deg0) * i as f64 / steps as f64).to_radians();
            (cx + rx * t.cos(), cy + ry * t.sin())
        })
        .collect()
}

/// 每个数字的笔画集合（多条折线，坐标位于设计网格内）。
fn digit_strokes(digit: usize) -> Vec<Vec<(f64, f64)>> {
    match digit {
        0 => vec![arc(52.0, 77.0, 33.0, 61.0, 0.0, 360.0)],
        1 => vec![vec![(32.0, 32.0), (56.0, 14.0), (56.0, 140.0)]],
        2 => {
            // 顶部圆弧 + 斜线 + 底部横线
            let mut path = arc(52.0, 44.0, 31.0, 30.0, 180.0, 355.0);
            path.push((24.0, 140.0));
            path.push((84.0, 140.0));
            vec![path]
        }
        3 => vec![
            arc(48.0, 46.0, 30.0, 32.0, 195.0, 350.0),
            arc(48.0, 108.0, 32.0, 34.0, -70.0, 165.0),
        ],
        4 => vec![
            vec![(66.0, 14.0), (22.0, 102.0), (88.0, 102.0)],
            vec![(70.0, 57.0), (70.0, 140.0)],
        ],
        5 => {
            // 上半折线接底部圆弧
            let mut path = vec![(80.0, 14.0), (28.0, 14.0), (26.0, 68.0)];
            path.extend(arc(49.0, 101.0, 33.0, 39.0, -95.0, 140.0));
            vec![path]
        }
        6 => vec![
            vec![(71.0, 14.0), (55.0, 40.0), (41.0, 70.0), (28.0, 102.0)],
            arc(51.0, 106.0, 30.0, 34.0, 0.0, 360.0),
        ],
        7 => vec![vec![(22.0, 14.0), (84.0, 14.0), (46.0, 140.0)]],
        8 => vec![
            arc(52.0, 46.0, 27.0, 32.0, 0.0, 360.0),
            arc(52.0, 110.0, 31.0, 32.0, 0.0, 360.0),
        ],
        _ => vec![
            arc(53.0, 48.0, 30.0, 34.0, 0.0, 360.0),
            vec![(80.0, 60.0), (67.0, 102.0), (53.0, 140.0)],
        ],
    }
}

/// 渲染单个数字：横向加宽 → 超采样描边 → 缩小 → 按 alpha 边界裁剪。
fn render_digit(digit: usize) -> Img {
    let scale = SUPERSAMPLE;
    let big_w = (CELL_W * scale) as u32;
    let big_h = (CELL_H * scale) as u32;
    let mut big = Img::new(big_w, big_h, [0, 0, 0, 0]);

    // 笔画坐标原以 ~104 宽网格设计；绕中轴（x=52）横向加宽
    const DESIGN_MID_X: f64 = 52.0;
    let cell_mid = CELL_W / 2.0;
    for stroke_path in digit_strokes(digit) {
        let pts: Vec<(f64, f64)> = stroke_path
            .iter()
            .map(|&(x, y)| {
                let widened_x = cell_mid + (x - DESIGN_MID_X) * X_WIDEN;
                (widened_x * scale, y * scale)
            })
            .collect();
        big.stroke_polyline(&pts, STROKE * scale, [255, 255, 255, 255], true);
    }

    let small = big.resize(CELL_W as u32, CELL_H as u32);
    match small.alpha_bbox() {
        Some((x0, y0, x1, y1)) => small.crop(x0, y0, x1, y1),
        None => small,
    }
}
