//! RGBA8 image type with PIL-compatible alpha compositing, Lanczos resize,
//! rotation, and 2D drawing primitives.

#[derive(Debug, Clone)]
pub struct Img {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>, // RGBA, row-major
}

pub type Rgba = [u8; 4];

impl Img {
    pub fn new(w: u32, h: u32, color: Rgba) -> Self {
        let mut data = vec![0u8; (w * h * 4) as usize];
        if color != [0, 0, 0, 0] {
            for px in data.chunks_exact_mut(4) {
                px.copy_from_slice(&color);
            }
        }
        Img { w, h, data }
    }

    #[inline]
    pub fn idx(&self, x: u32, y: u32) -> usize {
        ((y * self.w + x) * 4) as usize
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> Rgba {
        let i = self.idx(x, y);
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }

    #[inline]
    pub fn put(&mut self, x: u32, y: u32, c: Rgba) {
        let i = self.idx(x, y);
        self.data[i..i + 4].copy_from_slice(&c);
    }

    /// Straight-alpha src-over blend of a single pixel.
    #[inline]
    pub fn blend_px(&mut self, x: i64, y: i64, c: Rgba) {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            return;
        }
        let i = self.idx(x as u32, y as u32);
        blend_into(&mut self.data[i..i + 4], c);
    }

    /// PIL Image.alpha_composite: src-over `src` at (ox, oy).
    pub fn alpha_composite(&mut self, src: &Img, ox: i64, oy: i64) {
        let x0 = ox.max(0);
        let y0 = oy.max(0);
        let x1 = (ox + src.w as i64).min(self.w as i64);
        let y1 = (oy + src.h as i64).min(self.h as i64);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for y in y0..y1 {
            let sy = (y - oy) as u32;
            let drow = ((y as u32 * self.w + x0 as u32) * 4) as usize;
            let srow = ((sy * src.w + (x0 - ox) as u32) * 4) as usize;
            let count = (x1 - x0) as usize;
            let dst = &mut self.data[drow..drow + count * 4];
            let sp = &src.data[srow..srow + count * 4];
            for (d, s) in dst.chunks_exact_mut(4).zip(sp.chunks_exact(4)) {
                let sa = s[3];
                if sa == 255 {
                    d.copy_from_slice(s);
                } else if sa != 0 {
                    blend_into(d, [s[0], s[1], s[2], sa]);
                }
            }
        }
    }

    pub fn crop(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> Img {
        let w = x1.saturating_sub(x0);
        let h = y1.saturating_sub(y0);
        let mut out = Img::new(w, h, [0, 0, 0, 0]);
        for y in 0..h {
            let si = (((y0 + y) * self.w + x0) * 4) as usize;
            let di = ((y * w) * 4) as usize;
            out.data[di..di + (w * 4) as usize]
                .copy_from_slice(&self.data[si..si + (w * 4) as usize]);
        }
        out
    }

    /// Bounding box of non-zero alpha, like PIL getbbox() on alpha channel.
    pub fn alpha_bbox(&self) -> Option<(u32, u32, u32, u32)> {
        let mut min_x = self.w;
        let mut min_y = self.h;
        let mut max_x = 0u32;
        let mut max_y = 0u32;
        let mut found = false;
        for y in 0..self.h {
            for x in 0..self.w {
                if self.data[self.idx(x, y) + 3] != 0 {
                    found = true;
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }
        if found {
            Some((min_x, min_y, max_x + 1, max_y + 1))
        } else {
            None
        }
    }

    /// Scale the alpha channel by factor (0..=1).
    pub fn scale_alpha(&self, factor: f64) -> Img {
        let mut out = self.clone();
        let f = factor.clamp(0.0, 1.0);
        let mut lut = [0u8; 256];
        for (v, e) in lut.iter_mut().enumerate() {
            *e = (v as f64 * f) as u8;
        }
        for px in out.data.chunks_exact_mut(4) {
            px[3] = lut[px[3] as usize];
        }
        out
    }

    /// Lanczos-3 resize (PIL Image.LANCZOS).
    pub fn resize(&self, nw: u32, nh: u32) -> Img {
        if nw == 0 || nh == 0 {
            return Img::new(nw.max(1), nh.max(1), [0, 0, 0, 0]);
        }
        if nw == self.w && nh == self.h {
            return self.clone();
        }
        let horizontal = resample_axis(self, nw, true);
        resample_axis(&horizontal, nh, false)
    }

    /// Rotate counterclockwise by `angle_deg` with expand=true, bilinear sampling.
    pub fn rotate_expand(&self, angle_deg: f64) -> Img {
        let theta = angle_deg.to_radians();
        let (sin_t, cos_t) = theta.sin_cos();
        let w = self.w as f64;
        let h = self.h as f64;
        let nw = (w * cos_t.abs() + h * sin_t.abs()).round().max(1.0) as u32;
        let nh = (w * sin_t.abs() + h * cos_t.abs()).round().max(1.0) as u32;
        let cx = w / 2.0;
        let cy = h / 2.0;
        let ncx = nw as f64 / 2.0;
        let ncy = nh as f64 / 2.0;
        let mut out = Img::new(nw, nh, [0, 0, 0, 0]);
        for y in 0..nh {
            for x in 0..nw {
                let dx = x as f64 + 0.5 - ncx;
                let dy = y as f64 + 0.5 - ncy;
                // inverse rotation (CCW image rotation = CW coordinate rotation)
                let sx = dx * cos_t - dy * sin_t + cx - 0.5;
                let sy = dx * sin_t + dy * cos_t + cy - 0.5;
                let c = self.sample_bilinear(sx, sy);
                if c[3] != 0 {
                    out.put(x, y, c);
                }
            }
        }
        out
    }

    fn sample_bilinear(&self, x: f64, y: f64) -> Rgba {
        if x < -1.0 || y < -1.0 || x > self.w as f64 || y > self.h as f64 {
            return [0, 0, 0, 0];
        }
        let x0 = x.floor() as i64;
        let y0 = y.floor() as i64;
        let fx = x - x0 as f64;
        let fy = y - y0 as f64;
        let mut acc = [0.0f64; 4];
        let mut weight_total = 0.0;
        for (dy, wy) in [(0i64, 1.0 - fy), (1, fy)] {
            for (dx, wx) in [(0i64, 1.0 - fx), (1, fx)] {
                let px = x0 + dx;
                let py = y0 + dy;
                let w = wx * wy;
                if w <= 0.0 {
                    continue;
                }
                weight_total += w;
                if px >= 0 && py >= 0 && px < self.w as i64 && py < self.h as i64 {
                    let c = self.get(px as u32, py as u32);
                    let a = c[3] as f64;
                    acc[0] += c[0] as f64 * a * w;
                    acc[1] += c[1] as f64 * a * w;
                    acc[2] += c[2] as f64 * a * w;
                    acc[3] += a * w;
                }
            }
        }
        if weight_total <= 0.0 || acc[3] <= 0.0 {
            return [0, 0, 0, 0];
        }
        let a = acc[3] / weight_total;
        [
            (acc[0] / acc[3]).round().clamp(0.0, 255.0) as u8,
            (acc[1] / acc[3]).round().clamp(0.0, 255.0) as u8,
            (acc[2] / acc[3]).round().clamp(0.0, 255.0) as u8,
            a.round().clamp(0.0, 255.0) as u8,
        ]
    }

    // ─── drawing primitives ───

    pub fn fill_rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, color: Rgba) {
        let xa = x0.max(0) as u32;
        let ya = y0.max(0) as u32;
        let xb = (x1 + 1).clamp(0, self.w as i64) as u32;
        let yb = (y1 + 1).clamp(0, self.h as i64) as u32;
        if color[3] == 255 {
            for y in ya..yb {
                let i = self.idx(xa, y);
                for px in self.data[i..i + ((xb - xa) * 4) as usize].chunks_exact_mut(4) {
                    px.copy_from_slice(&color);
                }
            }
        } else if color[3] > 0 {
            for y in ya..yb {
                let i = self.idx(xa, y);
                for px in self.data[i..i + ((xb - xa) * 4) as usize].chunks_exact_mut(4) {
                    blend_into(px, color);
                }
            }
        }
    }

    /// Overwrite rect pixels (no blending) — like ImageDraw on RGBA.
    pub fn set_rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, color: Rgba) {
        let xa = x0.max(0) as u32;
        let ya = y0.max(0) as u32;
        let xb = (x1 + 1).clamp(0, self.w as i64) as u32;
        let yb = (y1 + 1).clamp(0, self.h as i64) as u32;
        for y in ya..yb {
            let i = self.idx(xa, y);
            for px in self.data[i..i + ((xb.saturating_sub(xa)) * 4) as usize].chunks_exact_mut(4) {
                px.copy_from_slice(&color);
            }
        }
    }

    /// Filled ellipse in bbox [x0,y0,x1,y1] (inclusive, PIL semantics), pixels overwritten.
    pub fn fill_ellipse(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Rgba) {
        let cx = (x0 + x1) / 2.0;
        let cy = (y0 + y1) / 2.0;
        let rx = (x1 - x0) / 2.0;
        let ry = (y1 - y0) / 2.0;
        if rx <= 0.0 || ry <= 0.0 {
            return;
        }
        let ya = (cy - ry).floor().max(0.0) as i64;
        let yb = (cy + ry).ceil().min(self.h as f64 - 1.0) as i64;
        for y in ya..=yb {
            let dy = (y as f64 - cy) / ry;
            let rem = 1.0 - dy * dy;
            if rem < 0.0 {
                continue;
            }
            let half = rem.sqrt() * rx;
            let xa = ((cx - half).ceil().max(0.0)) as i64;
            let xb2 = ((cx + half).floor().min(self.w as f64 - 1.0)) as i64;
            for x in xa..=xb2 {
                if x >= 0 && y >= 0 {
                    self.put_unchecked(x, y, color);
                }
            }
        }
    }

    /// Anti-aliased filled disc, blended.
    pub fn fill_circle_aa(&mut self, cx: f64, cy: f64, r: f64, color: Rgba) {
        if r <= 0.0 {
            return;
        }
        let ya = (cy - r - 1.0).floor().max(0.0) as i64;
        let yb = (cy + r + 1.0).ceil().min(self.h as f64 - 1.0) as i64;
        let xa = (cx - r - 1.0).floor().max(0.0) as i64;
        let xb = (cx + r + 1.0).ceil().min(self.w as f64 - 1.0) as i64;
        for y in ya..=yb {
            for x in xa..=xb {
                let dx = x as f64 + 0.5 - cx;
                let dy = y as f64 + 0.5 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                let cov = (r - d + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let mut c = color;
                    c[3] = (color[3] as f64 * cov) as u8;
                    self.blend_px(x, y, c);
                }
            }
        }
    }

    #[inline]
    fn put_unchecked(&mut self, x: i64, y: i64, c: Rgba) {
        if x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h {
            self.put(x as u32, y as u32, c);
        }
    }

    /// 1px-ish line (overwrite), like PIL draw.line width=w without joints.
    pub fn draw_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, width: f64, color: Rgba) {
        self.stroke_polyline(&[(x0, y0), (x1, y1)], width, color, false);
    }

    /// Thick polyline with optional round joints/caps (PIL joint="curve" + end ellipses).
    /// Pixels are overwritten (no blending), matching ImageDraw behaviour on an empty layer.
    pub fn stroke_polyline(&mut self, pts: &[(f64, f64)], width: f64, color: Rgba, round_caps: bool) {
        if pts.is_empty() {
            return;
        }
        let half = width / 2.0;
        if pts.len() == 1 {
            if round_caps {
                self.fill_ellipse(
                    pts[0].0 - half,
                    pts[0].1 - half,
                    pts[0].0 + half,
                    pts[0].1 + half,
                    color,
                );
            }
            return;
        }
        // Compute overall bbox
        let mut min_x = f64::MAX;
        let mut min_y = f64::MAX;
        let mut max_x = f64::MIN;
        let mut max_y = f64::MIN;
        for &(x, y) in pts {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        let xa = (min_x - half - 1.0).floor().max(0.0) as i64;
        let ya = (min_y - half - 1.0).floor().max(0.0) as i64;
        let xb = (max_x + half + 1.0).ceil().min(self.w as f64 - 1.0) as i64;
        let yb = (max_y + half + 1.0).ceil().min(self.h as f64 - 1.0) as i64;
        if xa > xb || ya > yb {
            return;
        }

        // Distance-to-polyline rasterization on a grid; for long paths use
        // per-segment bboxes to avoid O(area * segments).
        let grid_w = (xb - xa + 1) as usize;
        let grid_h = (yb - ya + 1) as usize;
        let mut covered = vec![false; grid_w * grid_h];
        let half_sq = half * half;

        let mark_segment = |covered: &mut Vec<bool>, a: (f64, f64), b: (f64, f64)| {
            let sx0 = ((a.0.min(b.0) - half - 1.0).floor().max(xa as f64)) as i64;
            let sy0 = ((a.1.min(b.1) - half - 1.0).floor().max(ya as f64)) as i64;
            let sx1 = ((a.0.max(b.0) + half + 1.0).ceil().min(xb as f64)) as i64;
            let sy1 = ((a.1.max(b.1) + half + 1.0).ceil().min(yb as f64)) as i64;
            let abx = b.0 - a.0;
            let aby = b.1 - a.1;
            let len_sq = abx * abx + aby * aby;
            for y in sy0..=sy1 {
                let py = y as f64 + 0.5;
                for x in sx0..=sx1 {
                    let gi = (y - ya) as usize * grid_w + (x - xa) as usize;
                    if covered[gi] {
                        continue;
                    }
                    let px = x as f64 + 0.5;
                    let dist_sq = if len_sq <= 1e-12 {
                        (px - a.0).powi(2) + (py - a.1).powi(2)
                    } else {
                        let t = (((px - a.0) * abx + (py - a.1) * aby) / len_sq).clamp(0.0, 1.0);
                        (px - (a.0 + t * abx)).powi(2) + (py - (a.1 + t * aby)).powi(2)
                    };
                    if dist_sq <= half_sq {
                        covered[gi] = true;
                    }
                }
            }
        };

        for win in pts.windows(2) {
            mark_segment(&mut covered, win[0], win[1]);
        }
        if !round_caps {
            // joints still rounded via segment distance overlap; caps are square-ish — PIL
            // butt caps differ negligibly at our widths.
        }
        for gy in 0..grid_h {
            for gx in 0..grid_w {
                if covered[gy * grid_w + gx] {
                    self.put_unchecked(xa + gx as i64, ya + gy as i64, color);
                }
            }
        }
    }

    /// Rounded rectangle fill (PIL rounded_rectangle).
    pub fn fill_rounded_rect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, radius: f64, color: Rgba) {
        let r = radius.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
        let ya = y0.floor().max(0.0) as i64;
        let yb = y1.ceil().min(self.h as f64 - 1.0) as i64;
        for y in ya..=yb {
            let py = y as f64 + 0.5;
            if py < y0 || py > y1 {
                continue;
            }
            let dy = if py < y0 + r {
                y0 + r - py
            } else if py > y1 - r {
                py - (y1 - r)
            } else {
                0.0
            };
            let inset = if dy > 0.0 {
                r - (r * r - dy * dy).max(0.0).sqrt()
            } else {
                0.0
            };
            let xa = ((x0 + inset).floor().max(0.0)) as i64;
            let xb = ((x1 - inset).ceil().min(self.w as f64 - 1.0)) as i64;
            for x in xa..=xb {
                let px = x as f64 + 0.5;
                if px >= x0 + inset && px <= x1 - inset {
                    self.put_unchecked(x, y, color);
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn to_rgb_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity((self.w * self.h * 3) as usize);
        for px in self.data.chunks_exact(4) {
            out.extend_from_slice(&px[..3]);
        }
        out
    }
}

#[inline]
fn blend_into(dst: &mut [u8], src: Rgba) {
    let sa = src[3] as u32;
    if sa == 0 {
        return;
    }
    let da = dst[3] as u32;
    if sa == 255 || da == 0 {
        dst.copy_from_slice(&src);
        return;
    }
    let out_a = sa * 255 + da * (255 - sa); // scaled by 255
    for c in 0..3 {
        let sc = src[c] as u32;
        let dc = dst[c] as u32;
        dst[c] = ((sc * sa * 255 + dc * da * (255 - sa) + out_a / 2) / out_a) as u8;
    }
    dst[3] = ((out_a + 127) / 255) as u8;
}

// ─── Lanczos-3 separable resampling ───

fn lanczos3(x: f64) -> f64 {
    if x.abs() >= 3.0 {
        return 0.0;
    }
    if x == 0.0 {
        return 1.0;
    }
    let pix = std::f64::consts::PI * x;
    3.0 * (pix.sin() * (pix / 3.0).sin()) / (pix * pix)
}

fn resample_axis(src: &Img, new_size: u32, horizontal: bool) -> Img {
    let (old_size, other) = if horizontal {
        (src.w, src.h)
    } else {
        (src.h, src.w)
    };
    if old_size == new_size {
        return src.clone();
    }
    let scale = old_size as f64 / new_size as f64;
    let filter_scale = scale.max(1.0);
    let support = 3.0 * filter_scale;

    // Precompute weights per output coordinate.
    let mut all_weights: Vec<(i64, Vec<f64>)> = Vec::with_capacity(new_size as usize);
    for o in 0..new_size {
        let center = (o as f64 + 0.5) * scale;
        let lo = (center - support).floor().max(0.0) as i64;
        let hi = ((center + support).ceil() as i64).min(old_size as i64);
        let mut weights = Vec::with_capacity((hi - lo) as usize);
        let mut sum = 0.0;
        for i in lo..hi {
            let w = lanczos3((i as f64 + 0.5 - center) / filter_scale);
            weights.push(w);
            sum += w;
        }
        if sum != 0.0 {
            for w in &mut weights {
                *w /= sum;
            }
        }
        all_weights.push((lo, weights));
    }

    let (nw, nh) = if horizontal {
        (new_size, other)
    } else {
        (other, new_size)
    };
    let mut out = Img::new(nw, nh, [0, 0, 0, 0]);

    for j in 0..other {
        for (o, (lo, weights)) in all_weights.iter().enumerate() {
            let mut acc = [0.0f64; 4];
            for (k, &w) in weights.iter().enumerate() {
                let i = (*lo + k as i64) as u32;
                let c = if horizontal {
                    src.get(i, j)
                } else {
                    src.get(j, i)
                };
                // premultiply to avoid halo from transparent pixels
                let a = c[3] as f64;
                acc[0] += c[0] as f64 * a * w;
                acc[1] += c[1] as f64 * a * w;
                acc[2] += c[2] as f64 * a * w;
                acc[3] += a * w;
            }
            let a = acc[3];
            let px = if a <= 0.0 {
                [0, 0, 0, 0]
            } else {
                [
                    (acc[0] / a).round().clamp(0.0, 255.0) as u8,
                    (acc[1] / a).round().clamp(0.0, 255.0) as u8,
                    (acc[2] / a).round().clamp(0.0, 255.0) as u8,
                    a.round().clamp(0.0, 255.0) as u8,
                ]
            };
            if horizontal {
                out.put(o as u32, j, px);
            } else {
                out.put(j, o as u32, px);
            }
        }
    }
    out
}
