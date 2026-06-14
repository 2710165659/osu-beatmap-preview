//! 程序化 Argon（2022）风格的 catch 对象绘制：水果、水滴、香蕉、接手。
//!
//! 对照 lazer `ArgonFruitPiece` + osu-framework `CircularBlob` shader 移植：
//! 光环不是正圆，而是沿圆周受 value-noise 扰动的「有机光斑环」
//! （sh_CircularBlobUtils.h 的 blobAlphaAt），多层加色混合叠出
//! Argon 水果特有的彩色光晕质感。
//!
//! 图层结构（ArgonFruitPiece）：
//! - 大号柔光环：尺寸 1.1d，InnerRadius 0.5，alpha 0.15
//! - 中号光环：  尺寸 d，  InnerRadius 0.2，alpha 0.5
//! - 细外缘环：  尺寸 d，  InnerRadius 0.05，alpha 1.0
//! - 白色中心圆点：直径 20/128 ≈ 0.156d（普通混合，非加色）
//! - 超冲时再叠 1.15d、InnerRadius 0.08 的超冲色环

use crate::canvas::Img;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::constants::*;
use super::objects::{ObjType, RenderObject};

const HALF_PI: f64 = std::f64::consts::FRAC_PI_2;
const TWO_PI: f64 = std::f64::consts::TAU;
/// CircularBlob 默认噪声参数（lazer 未覆盖默认值）。
const BLOB_FREQUENCY: f64 = 1.5;
const BLOB_AMPLITUDE: f64 = 0.3;
/// 每种对象预生成的随机形状变体数（按对象时间散列选取）。
const SEED_VARIANTS: u64 = 8;

// ─── value-noise（移植自 thebookofshaders，与 shader 一致） ───

fn blob_random(x: f64, y: f64) -> f64 {
    let v = (x * 12.9898 + y * 78.233).sin() * 43758.5453123;
    v - v.floor()
}

fn blob_noise(x: f64, y: f64) -> f64 {
    let ix = x.floor();
    let iy = y.floor();
    let fx = x - ix;
    let fy = y - iy;
    let a = blob_random(ix, iy);
    let b = blob_random(ix + 1.0, iy);
    let c = blob_random(ix, iy + 1.0);
    let d = blob_random(ix + 1.0, iy + 1.0);
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uy = fy * fy * (3.0 - 2.0 * fy);
    a * (1.0 - ux) + b * ux + (c - a) * uy * (1.0 - ux) + (d - b) * ux * uy
}

/// GLSL smoothstep（edge0 > edge1 的反向用法，与 shader 相同）。
fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// blobAlphaAt 的径向近似：像素在归一化 [0,1]² 空间内，
/// 环半径随角度被噪声扰动，返回该像素的覆盖率（0..1）。
fn blob_alpha_at(
    px: f64, py: f64, inner_radius: f64, texel: f64,
    noise_x: f64, noise_y: f64,
) -> f64 {
    let path_radius = inner_radius * 0.25;
    let dxc = px - 0.5;
    let dyc = py - 0.5;
    let dist = (dxc * dxc + dyc * dyc).sqrt();

    let mut angle = (0.5 - py).atan2(0.5 - px) - HALF_PI;
    if angle < 0.0 {
        angle += TWO_PI;
    }
    let ca = (angle - HALF_PI).cos();
    let sa = (angle - HALF_PI).sin();
    let noise_value = blob_noise(noise_x + ca * BLOB_FREQUENCY, noise_y + sa * BLOB_FREQUENCY);
    let ring_radius = 0.5 - path_radius - texel - noise_value * 0.5 * BLOB_AMPLITUDE;

    smoothstep(texel, 0.0, (dist - ring_radius).abs() - path_radius)
}

// ─── 加色光斑 sprite 构建与缓存 ───

/// 单层 blob 描述：尺寸（相对基准直径 d 的倍数）、InnerRadius、alpha。
struct BlobLayer {
    size_ratio: f64,
    inner_radius: f64,
    alpha: f64,
    color: [u8; 3],
}

/// 将多层 blob 的加色贡献累加进一个 sprite：
/// rgb 通道存 src-over 混合后的颜色（非预乘），alpha 通道存累计覆盖。
fn build_blob_sprite(base_diameter: f64, layers: &[BlobLayer], seed: u64) -> Img {
    let max_ratio = layers.iter().fold(1.0f64, |m, l| m.max(l.size_ratio));
    let sprite_size = ((base_diameter * max_ratio).ceil() as i64 + 4).max(1) as u32;
    // [0]=R, [1]=G, [2]=B (非预乘), [3]=A (0..1)
    let mut acc = vec![[0.0f64; 4]; (sprite_size * sprite_size) as usize];
    let center = sprite_size as f64 / 2.0;

    // 噪声起点由 seed 决定（对应 lazer 的 Random(seed) 噪声偏移）
    let noise_x = blob_random(seed as f64 * 0.713, 17.31) * 1000.0;
    let noise_y = blob_random(seed as f64 * 0.297, 91.17) * 1000.0;

    for layer in layers {
        let layer_size = base_diameter * layer.size_ratio;
        if layer_size <= 1.0 {
            continue;
        }
        let texel = 1.5 / layer_size;
        let half = layer_size / 2.0;
        let x0 = ((center - half).floor() as i64).max(0);
        let x1 = ((center + half).ceil() as i64).min(sprite_size as i64 - 1);
        for y in x0..=x1 {
            for x in x0..=x1 {
                // 像素在该层的归一化坐标
                let px = (x as f64 + 0.5 - (center - half)) / layer_size;
                let py = (y as f64 + 0.5 - (center - half)) / layer_size;
                if !(0.0..=1.0).contains(&px) || !(0.0..=1.0).contains(&py) {
                    continue;
                }
                let cov = blob_alpha_at(px, py, layer.inner_radius, texel, noise_x, noise_y);
                if cov <= 0.0 {
                    continue;
                }
                let src_a = cov * layer.alpha;
                let cell = &mut acc[(y as u32 * sprite_size + x as u32) as usize];
                let dst_a = cell[3];
                let inv_src_a = 1.0 - src_a;
                let new_a = src_a + dst_a * inv_src_a;
                if new_a > 0.0 {
                    let src_weight = src_a / new_a;
                    let dst_weight = 1.0 - src_weight;
                    cell[0] = layer.color[0] as f64 * src_weight + cell[0] * dst_weight;
                    cell[1] = layer.color[1] as f64 * src_weight + cell[1] * dst_weight;
                    cell[2] = layer.color[2] as f64 * src_weight + cell[2] * dst_weight;
                }
                cell[3] = new_a;
            }
        }
    }

    let mut sprite = Img::new(sprite_size, sprite_size, [0, 0, 0, 0]);
    for (i, cell) in acc.iter().enumerate() {
        if cell[3] <= 0.0 {
            continue;
        }
        let idx = i * 4;
        sprite.data[idx] = cell[0].round().clamp(0.0, 255.0) as u8;
        sprite.data[idx + 1] = cell[1].round().clamp(0.0, 255.0) as u8;
        sprite.data[idx + 2] = cell[2].round().clamp(0.0, 255.0) as u8;
        sprite.data[idx + 3] = (cell[3] * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    sprite
}

/// src-over 合成预构建的 blob sprite（标准 alpha 混合，替代加色混合）。
fn composite_blob_sprite(image: &mut Img, sprite: &Img, cx: f64, cy: f64) {
    let ox = (cx - sprite.w as f64 / 2.0).round() as i64;
    let oy = (cy - sprite.h as f64 / 2.0).round() as i64;
    image.alpha_composite(sprite, ox, oy);
}

/// sprite 缓存键：对象种类 + 颜色 + 像素直径 + 形状变体 + 超冲。
type SpriteKey = (u8, [u8; 3], u32, u8, bool);

thread_local! {
    static SPRITE_CACHE: RefCell<HashMap<SpriteKey, Rc<Img>>> = RefCell::new(HashMap::new());
}

/// 取（或构建）某对象的 blob sprite。
fn cached_blob_sprite(
    kind: u8, color: [u8; 3], diameter: f64, seed: u64, hyper: bool,
    build: impl FnOnce(u64) -> Img,
) -> Rc<Img> {
    let key: SpriteKey = (
        kind,
        color,
        diameter.round().max(1.0) as u32,
        (seed % SEED_VARIANTS) as u8,
        hyper,
    );
    SPRITE_CACHE.with(|cache| {
        if let Some(sprite) = cache.borrow().get(&key) {
            return Rc::clone(sprite);
        }
        let sprite = Rc::new(build(seed % SEED_VARIANTS));
        cache.borrow_mut().insert(key, Rc::clone(&sprite));
        sprite
    })
}

/// 超冲提示颜色（skin.ini [CatchTheBeat] HyperDash）。
fn hyper_dash_color() -> [u8; 3] {
    crate::skin::skin().hyper_dash
}

// ─── 对象绘制 ───

/// Argon 水果：三层噪声光环 + 白色中心点；超冲叠加超冲色外环。
pub(crate) fn draw_argon_fruit(
    image: &mut Img, cx: f64, cy: f64, d: f64, color: [u8; 3], hyper: bool, seed: u64,
) {
    let sprite = cached_blob_sprite(0, color, d, seed, hyper, |variant| {
        let mut layers = vec![
            BlobLayer { size_ratio: 1.1, inner_radius: 0.5, alpha: 0.15, color },
            BlobLayer { size_ratio: 1.0, inner_radius: 0.2, alpha: 0.5, color },
            BlobLayer { size_ratio: 1.0, inner_radius: 0.05, alpha: 1.0, color },
        ];
        if hyper {
            layers.push(BlobLayer {
                size_ratio: 1.15,
                inner_radius: 0.08,
                alpha: 1.0,
                color: hyper_dash_color(),
            });
        }
        build_blob_sprite(d, &layers, variant)
    });
    composite_blob_sprite(image, &sprite, cx, cy);
    // 白色中心圆点（20/128 ≈ 0.156d 直径，普通混合）
    image.fill_circle_aa(cx, cy, d * 0.078, [255, 255, 255, 255]);
}

/// Argon 水滴：0.7 缩放的双层光环 + 白色中心点（ArgonDropletPiece）。
pub(crate) fn draw_argon_droplet(
    image: &mut Img, cx: f64, cy: f64, d: f64, color: [u8; 3], hyper: bool, seed: u64,
) {
    let sprite = cached_blob_sprite(1, color, d, seed, hyper, |variant| {
        let mut layers = vec![
            BlobLayer { size_ratio: 0.7, inner_radius: 0.5, alpha: 0.15, color },
            // 内层：blob 自身再缩放 0.7（0.7 × 0.7 = 0.49）
            BlobLayer { size_ratio: 0.49, inner_radius: 0.4, alpha: 0.5, color },
        ];
        if hyper {
            layers.push(BlobLayer {
                size_ratio: 0.7,
                inner_radius: 0.5,
                alpha: 0.15,
                color: hyper_dash_color(),
            });
        }
        build_blob_sprite(d, &layers, variant)
    });
    composite_blob_sprite(image, &sprite, cx, cy);
    image.fill_circle_aa(cx, cy, d * 0.078, [255, 255, 255, 255]);
}

/// Argon 香蕉：与水果同构的光环（ArgonBananaPiece 继承自 FruitPiece）。
pub(crate) fn draw_argon_banana(
    image: &mut Img, cx: f64, cy: f64, d: f64, color: [u8; 3], seed: u64,
) {
    let sprite = cached_blob_sprite(2, color, d, seed, false, |variant| {
        let layers = [
            BlobLayer { size_ratio: 1.1, inner_radius: 0.5, alpha: 0.15, color },
            BlobLayer { size_ratio: 1.0, inner_radius: 0.2, alpha: 0.5, color },
            BlobLayer { size_ratio: 1.0, inner_radius: 0.05, alpha: 1.0, color },
        ];
        build_blob_sprite(d, &layers, variant)
    });
    composite_blob_sprite(image, &sprite, cx, cy);
    image.fill_circle_aa(cx, cy, d * 0.078, [255, 255, 255, 230]);
}

/// 按对象类型分发绘制；形状变体种子取自对象时间，同一对象稳定、不同对象有差异。
pub(crate) fn draw_catch_object(
    image: &mut Img, obj: &RenderObject, cx: f64, cy: f64, diameter: f64,
) {
    let seed = (obj.start_time as u64).wrapping_mul(2654435761);
    match obj.object_type {
        ObjType::Fruit => {
            draw_argon_fruit(image, cx, cy, diameter, obj.color, obj.hyper_dash, seed)
        }
        ObjType::Droplet | ObjType::TinyDroplet => {
            draw_argon_droplet(image, cx, cy, diameter, obj.color, obj.hyper_dash, seed)
        }
        ObjType::Banana => draw_argon_banana(image, cx, cy, diameter, obj.color, seed),
    }
}

/// 对象直径：基础半径 × 对象缩放 × 类型缩放 × playfield 缩放。
pub(crate) fn object_diameter(object_scale: f64, playfield_scale: f64, scale_factor: f64) -> f64 {
    OBJECT_RADIUS * 2.0 * object_scale * scale_factor * playfield_scale
}
