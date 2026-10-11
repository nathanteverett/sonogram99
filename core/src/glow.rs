//! A light field for the glowing modes (4 to 11). Each pixel holds an amount of light and a
//! position on the skin's glow gradient. Blurs and warps move both together, so where two
//! colors smear into each other the result is a color between them on the gradient, never
//! a muddy gray.

use std::f32::consts::TAU;
use std::sync::OnceLock;

use crate::canvas::{pack, Canvas};
use crate::palette::Palette;

/// One pixel: the light, then the light times the gradient position. Storing the product
/// means every warp and smear averages the position by brightness.
type Px = [f32; 2];
const DARK: Px = [0.0, 0.0];

/// Integral of the brush profile (1 - t²)² across its width, in units of the radius.
const BRUSH_AREA: f32 = 16.0 / 15.0;

/// The per-frame factors are tuned at 60 fps. This keeps trail lengths the same at other rates.
pub fn per_frame(k: f32, dt: f32) -> f32 {
    k.powf(dt * 60.0)
}

/// Splits `out` into bands of whole rows and fills them on several threads. `f` gets the
/// index of the band's first row. Small areas stay on one thread, where spawning costs more
/// than it saves.
fn bands<T: Send>(out: &mut [T], w: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    static CORES: OnceLock<usize> = OnceLock::new();
    let cores = *CORES.get_or_init(|| std::thread::available_parallelism().map_or(1, |n| n.get()));
    if w == 0 || out.is_empty() {
        return;
    }
    let h = out.len() / w;
    let n = (out.len() / 65_536).clamp(1, cores.min(h).max(1));
    if n == 1 {
        f(0, out);
        return;
    }
    let rows = h.div_ceil(n);
    std::thread::scope(|s| {
        for (k, band) in out.chunks_mut(rows * w).enumerate() {
            let f = &f;
            s.spawn(move || f(k * rows, band));
        }
    });
}

/// A bilinear sample blurred by [1 2 1] / 4 in both directions: 4 x 4 pixels, weighted by
/// the bilinear weights spread over their neighbors. Light past the edge counts as dark.
#[inline]
fn sample_wide(src: &[Px], w: usize, h: usize, sx: f32, sy: f32, keep: f32) -> Px {
    if !(sx >= 1.0 && sy >= 1.0 && sx < w as f32 - 2.0 && sy < h as f32 - 2.0) {
        return DARK;
    }
    let (ix, iy) = (sx as usize, sy as usize);
    let (fx, fy) = (sx - ix as f32, sy - iy as f32);
    let kx = [1.0 - fx, 2.0 - fx, 1.0 + fx, fx];
    let ky = [1.0 - fy, 2.0 - fy, 1.0 + fy, fy].map(|k| k * keep / 16.0);
    let mut out = DARK;
    for (j, &wy) in ky.iter().enumerate() {
        let row = &src[(iy + j - 1) * w + ix - 1..][..4];
        let (mut e, mut eu) = (0.0, 0.0);
        for (p, &wx) in row.iter().zip(&kx) {
            e += p[0] * wx;
            eu += p[1] * wx;
        }
        out[0] += e * wy;
        out[1] += eu * wy;
    }
    out
}

/// A bilinear sample, times `keep`. Light past the edge counts as dark.
#[inline]
fn sample_sharp(src: &[Px], w: usize, h: usize, sx: f32, sy: f32, keep: f32) -> Px {
    if !(sx >= 0.0 && sy >= 0.0 && sx < w as f32 - 1.0 && sy < h as f32 - 1.0) {
        return DARK;
    }
    let (ix, iy) = (sx as usize, sy as usize);
    let (fx, fy) = (sx - ix as f32, sy - iy as f32);
    let i = iy * w + ix;
    let (p00, p10, p01, p11) = (src[i], src[i + 1], src[i + w], src[i + w + 1]);
    let (k00, k10) = ((1.0 - fx) * (1.0 - fy) * keep, fx * (1.0 - fy) * keep);
    let (k01, k11) = ((1.0 - fx) * fy * keep, fx * fy * keep);
    [
        p00[0] * k00 + p10[0] * k10 + p01[0] * k01 + p11[0] * k11,
        p00[1] * k00 + p10[1] * k10 + p01[1] * k01 + p11[1] * k11,
    ]
}

/// Where each pixel takes its light from, set only at the corners of a coarse grid. Pixels
/// between the corners blend the corner values, so motion that changes across the picture
/// (stronger zoom at the edges, a local swirl, a ripple) costs no more per pixel than one
/// zoom and turn for the whole picture.
pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    /// For each corner, row by row: the source x and y in pixels, and the share of light kept.
    pub pts: Vec<[f32; 3]>,
}

impl Grid {
    pub const COLS: usize = 32;
    pub const ROWS: usize = 24;

    /// Calls `f(x, y)` at each corner of a `w` by `h` picture, with x and y in pixels.
    /// `f` returns the source x, the source y, and the share of light to keep.
    pub fn build(w: usize, h: usize, f: impl Fn(f32, f32) -> [f32; 3]) -> Grid {
        let (cols, rows) = (Self::COLS, Self::ROWS);
        let step_x = w.max(2).saturating_sub(1) as f32 / cols as f32;
        let step_y = h.max(2).saturating_sub(1) as f32 / rows as f32;
        let pts = (0..=rows)
            .flat_map(|j| (0..=cols).map(move |i| (i as f32 * step_x, j as f32 * step_y)))
            .map(|(x, y)| f(x, y))
            .collect();
        Grid { cols, rows, pts }
    }
}

#[derive(Default)]
pub struct Field {
    pub w: usize,
    pub h: usize,
    /// Light near 1 is full color; highlights retain most of their saturation.
    px: Vec<Px>,
    /// Scratch for the passes that cannot work in place.
    tmp: Vec<Px>,
}

/// What fills a polygon.
#[derive(Clone, Copy, Debug)]
pub enum Fill {
    /// Light up to this level, at this gradient position.
    Light(f32, f32),
    /// A copy of the whole field, turned by `twist` radians. A step of one pixel inside the
    /// polygon moves `scale` pixels across the field.
    Copy { scale: f32, twist: f32 },
}

/// A spatially varying velocity field, measured in fractions of the shorter image side.
#[derive(Clone, Copy, Default)]
pub struct Flow {
    pub time: f32,
    pub amount: f32,
    pub dt: f32,
}

impl Field {
    /// Matches the field to the drawing area. A new size starts dark.
    pub fn fit(&mut self, w: usize, h: usize) {
        if self.w != w || self.h != h || self.px.len() != w * h {
            self.w = w;
            self.h = h;
            self.px = vec![DARK; w * h];
            self.tmp = vec![DARK; w * h];
        }
    }

    pub fn clear(&mut self) {
        self.px.fill(DARK);
    }

    /// Moves the light: the point `from` lands on `to`, and everything around it is scaled by
    /// `zoom` and turned by `rot` radians. Keeps `keep` of the light. Bilinear sampling keeps
    /// the trails smooth instead of smearing in blocks. With `spread`, the sample also takes
    /// in the pixels around it, so each frame widens the trails into soft smears.
    #[allow(clippy::too_many_arguments)]
    pub fn warp(&mut self, from: (f32, f32), to: (f32, f32), zoom: f32, rot: f32, keep: f32, spread: bool) {
        self.warp_flow(from, to, zoom, rot, keep, spread, Flow::default());
    }

    #[allow(clippy::too_many_arguments)]
    pub fn warp_flow(&mut self, from: (f32, f32), to: (f32, f32), zoom: f32, rot: f32, keep: f32, spread: bool, flow: Flow) {
        let (w, h) = (self.w, self.h);
        if w < 2 || h < 2 {
            return;
        }
        let (sn, cs) = rot.sin_cos();
        let inv = 1.0 / zoom;
        let src = &self.px;
        let m = w.min(h) as f32;
        // Evaluate the flow's oscillators per row/column, not per pixel.
        let flow_x: Vec<_> = (0..h).map(|y| {
            let p = (y as f32 - to.1) / m;
            ((p * 8.0 + flow.time * 0.7).sin() + 0.4 * (p * 15.0 - flow.time * 0.4).sin()) * m * flow.amount * flow.dt
        }).collect();
        let flow_y: Vec<_> = (0..w).map(|x| {
            let p = (x as f32 - to.0) / m;
            ((p * 7.0 - flow.time * 0.6).sin() + 0.4 * (p * 13.0 + flow.time * 0.5).sin()) * m * flow.amount * flow.dt
        }).collect();
        bands(&mut self.tmp, w, |y0, band| {
            for (r, out) in band.chunks_exact_mut(w).enumerate() {
                let dy = (y0 + r) as f32 - to.1;
                let row_x = from.0 + sn * dy * inv;
                let row_y = from.1 + cs * dy * inv;
                for (x, o) in out.iter_mut().enumerate() {
                    let dx = x as f32 - to.0;
                    let sx = row_x + cs * dx * inv + flow_x[y0 + r];
                    let sy = row_y - sn * dx * inv + flow_y[x];
                    *o = if spread {
                        sample_wide(src, w, h, sx, sy, keep)
                    } else {
                        sample_sharp(src, w, h, sx, sy, keep)
                    };
                }
            }
        });
        std::mem::swap(&mut self.px, &mut self.tmp);
    }

    /// Moves the light by a grid: each pixel takes its light from the place the grid gives
    /// for it, blended between the four nearest corners. `spread` widens trails as in `warp`.
    pub fn warp_grid(&mut self, grid: &Grid, spread: bool) {
        let (w, h) = (self.w, self.h);
        let (cols, rows) = (grid.cols, grid.rows);
        if w < 2 || h < 2 || cols == 0 || rows == 0 || grid.pts.len() != (cols + 1) * (rows + 1) {
            return;
        }
        // Each column's cell and its place across that cell, the same on every row.
        let per_x = cols as f32 / (w - 1) as f32;
        let cells: Vec<(usize, f32)> = (0..w).map(|x| {
            let p = x as f32 * per_x;
            let i = (p as usize).min(cols - 1);
            (i, p - i as f32)
        }).collect();
        let per_y = rows as f32 / (h - 1) as f32;
        let src = &self.px;
        bands(&mut self.tmp, w, |y0, band| {
            let mut row = vec![[0.0f32; 3]; cols + 1];
            for (r, out) in band.chunks_exact_mut(w).enumerate() {
                let p = (y0 + r) as f32 * per_y;
                let j = (p as usize).min(rows - 1);
                let t = p - j as f32;
                let top = &grid.pts[j * (cols + 1)..][..cols + 1];
                let bottom = &grid.pts[(j + 1) * (cols + 1)..][..cols + 1];
                for ((v, a), b) in row.iter_mut().zip(top).zip(bottom) {
                    *v = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
                }
                for (o, &(i, s)) in out.iter_mut().zip(&cells) {
                    let (a, b) = (row[i], row[i + 1]);
                    let (sx, sy, keep) = (a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s, a[2] + (b[2] - a[2]) * s);
                    *o = if spread { sample_wide(src, w, h, sx, sy, keep) } else { sample_sharp(src, w, h, sx, sy, keep) };
                }
            }
        });
        std::mem::swap(&mut self.px, &mut self.tmp);
    }

    /// Adds a soft round spot of light at gradient position `u`: `a` at the center, falling
    /// to nothing at radius `r`.
    pub fn dab(&mut self, x: f32, y: f32, r: f32, u: f32, a: f32) {
        if !(x.is_finite() && y.is_finite() && a > 0.0) {
            return;
        }
        let r = r.max(0.7);
        let au = a * u.clamp(0.0, 1.0);
        let inv = 1.0 / (r * r);
        let (x0, y0) = ((x - r).ceil().max(0.0) as i32, (y - r).ceil().max(0.0) as i32);
        let x1 = ((x + r).floor() as i32).min(self.w as i32 - 1);
        let y1 = ((y + r).floor() as i32).min(self.h as i32 - 1);
        for py in y0..=y1 {
            let dy = py as f32 - y;
            let row = py as usize * self.w;
            for px in x0..=x1 {
                let dx = px as f32 - x;
                let q = 1.0 - (dx * dx + dy * dy) * inv;
                if q > 0.0 {
                    let p = &mut self.px[row + px as usize];
                    p[0] += a * q * q;
                    p[1] += au * q * q;
                }
            }
        }
    }

    /// Adds a hard round spot of light `a` at gradient position `u`, with radius `r`. Its edge
    /// fades over one pixel.
    pub fn disc(&mut self, x: f32, y: f32, r: f32, u: f32, a: f32) {
        if !(x.is_finite() && y.is_finite() && a > 0.0) {
            return;
        }
        // Below this, a dot between pixels would flicker as it moves.
        let r = r.max(0.6);
        let au = a * u.clamp(0.0, 1.0);
        let reach = r + 0.5;
        let (x0, y0) = ((x - reach).ceil().max(0.0) as i32, (y - reach).ceil().max(0.0) as i32);
        let x1 = ((x + reach).floor() as i32).min(self.w as i32 - 1);
        let y1 = ((y + reach).floor() as i32).min(self.h as i32 - 1);
        for py in y0..=y1 {
            let dy = py as f32 - y;
            let row = py as usize * self.w;
            for px in x0..=x1 {
                let dx = px as f32 - x;
                let cover = (r - dx.hypot(dy) + 0.5).clamp(0.0, 1.0);
                if cover > 0.0 {
                    let p = &mut self.px[row + px as usize];
                    p[0] += a * cover;
                    p[1] += au * cover;
                }
            }
        }
    }

    /// A soft line of brush radius `r`, its gradient position running from `u0` to `u1`.
    /// `a` is the light along the middle of the line. A zero-length line leaves a dot.
    #[allow(clippy::too_many_arguments)]
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, u0: f32, u1: f32, a: f32) {
        let r = r.max(0.7);
        let len = (x1 - x0).hypot(y1 - y0);
        if !len.is_finite() {
            return;
        }
        // The end point is left for the next segment, so polylines do not double their joints.
        let n = ((len / (r * 0.5)).ceil() as usize).clamp(1, 256);
        let share = a * (len / n as f32).max(r * 0.5) / (r * BRUSH_AREA);
        for i in 0..n {
            let t = i as f32 / n as f32;
            self.dab(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, r, u0 + (u1 - u0) * t, share);
        }
    }

    /// A soft polyline. Each point carries its own gradient position.
    pub fn polyline(&mut self, pts: &[(f32, f32, f32)], r: f32, a: f32) {
        for s in pts.windows(2) {
            self.line(s[0].0, s[0].1, s[1].0, s[1].1, r, s[0].2, s[1].2, a);
        }
    }

    /// A filled triangle of light `a` at gradient position `u`. Each edge fades over one pixel,
    /// so triangles that share an edge add up to an even fill.
    pub fn triangle(&mut self, p: [(f32, f32); 3], u: f32, a: f32) {
        if a.is_nan() || a <= 0.0 || p.iter().any(|q| !(q.0.is_finite() && q.1.is_finite())) {
            return;
        }
        let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[1].1 - p[0].1) * (p[2].0 - p[0].0);
        if area.abs() < 1e-3 {
            return;
        }
        // Each edge as a signed distance in pixels, positive inside.
        let edges = [0, 1, 2].map(|i| {
            let (s, e) = (p[i], p[(i + 1) % 3]);
            let (dx, dy) = (e.0 - s.0, e.1 - s.1);
            let k = area.signum() / dx.hypot(dy);
            (-dy * k, dx * k, (dy * s.0 - dx * s.1) * k)
        });
        let au = a * u.clamp(0.0, 1.0);
        let (lo, hi) = (|v: f32| (v - 1.0).floor().max(0.0) as usize, |v: f32| (v + 1.0).ceil().max(0.0) as usize);
        let x0 = lo(p[0].0.min(p[1].0).min(p[2].0));
        let y0 = lo(p[0].1.min(p[1].1).min(p[2].1));
        let x1 = hi(p[0].0.max(p[1].0).max(p[2].0)).min(self.w);
        let y1 = hi(p[0].1.max(p[1].1).max(p[2].1)).min(self.h);
        for y in y0..y1 {
            let qy = y as f32 + 0.5;
            // Each edge limits the row to the pixels it leaves some cover on. A margin of one
            // pixel keeps the limits safe from rounding, so no lit pixel is skipped.
            let (mut from, mut to) = (x0 as f32, x1 as f32);
            for e in &edges {
                let c = e.1 * qy + e.2;
                if e.0 == 0.0 {
                    if c <= -0.5 {
                        to = from;
                    }
                } else {
                    // The pixel where this edge's cover reaches 0.
                    let at = (-0.5 - c) / e.0 - 0.5;
                    if e.0 > 0.0 { from = from.max(at - 1.0); } else { to = to.min(at + 2.0); }
                }
            }
            for x in from.floor().max(x0 as f32) as usize..(to.ceil() as usize).min(x1) {
                let qx = x as f32 + 0.5;
                let d = edges.iter().fold(f32::INFINITY, |d, e| d.min(e.0 * qx + e.1 * qy + e.2));
                let cover = (d + 0.5).clamp(0.0, 1.0);
                if cover > 0.0 {
                    let px = &mut self.px[y * self.w + x];
                    px[0] += a * cover;
                    px[1] += au * cover;
                }
            }
        }
    }

    /// Adds a frame of light `a` around the edge, `size` pixels wide, at gradient position
    /// `u`. Its inner edge fades over one pixel.
    pub fn border(&mut self, size: f32, u: f32, a: f32) {
        if !(a > 0.0 && size > 0.0) {
            return;
        }
        let (w, h) = (self.w, self.h);
        let au = a * u.clamp(0.0, 1.0);
        let reach = size.ceil() as usize + 1;
        for y in 0..h {
            let edge_y = y.min(h - 1 - y);
            // Rows away from the top and bottom only need their two ends.
            let xs: Box<dyn Iterator<Item = usize>> = if edge_y < reach {
                Box::new(0..w)
            } else {
                Box::new((0..reach.min(w)).chain(w.saturating_sub(reach).max(reach)..w))
            };
            for x in xs {
                let d = x.min(w - 1 - x).min(edge_y) as f32;
                let cover = (size - d + 0.5).clamp(0.0, 1.0);
                if cover > 0.0 {
                    let p = &mut self.px[y * w + x];
                    p[0] += a * cover;
                    p[1] += au * cover;
                }
            }
        }
    }

    /// Fills a regular polygon, centered at `c` with corners `radius` away, the first at
    /// `angle` radians. Each covered pixel moves `alpha` of the way toward the fill. Edges
    /// fade over one pixel.
    pub fn polygon(&mut self, c: (f32, f32), radius: f32, sides: usize, angle: f32, fill: Fill, alpha: f32) {
        let sides = sides.clamp(3, 64);
        if !(radius > 0.5 && alpha > 0.0 && c.0.is_finite() && c.1.is_finite() && angle.is_finite()) {
            return;
        }
        let (w, h) = (self.w, self.h);
        let step = TAU / sides as f32;
        let apothem = radius * (step * 0.5).cos();
        // Each sector's edge normal: the edge between corners k and k + 1.
        let normals: Vec<(f32, f32)> = (0..sides).map(|k| {
            let (s, c) = (angle + (k as f32 + 0.5) * step).sin_cos();
            (c, s)
        }).collect();
        let x0 = (c.0 - radius).floor().max(0.0) as usize;
        let y0 = (c.1 - radius).floor().max(0.0) as usize;
        let x1 = ((c.0 + radius).ceil() + 1.0).clamp(0.0, w as f32) as usize;
        let y1 = ((c.1 + radius).ceil() + 1.0).clamp(0.0, h as f32) as usize;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        if matches!(fill, Fill::Copy { .. }) {
            self.tmp.copy_from_slice(&self.px);
        }
        let (mid_x, mid_y) = (w as f32 * 0.5, h as f32 * 0.5);
        let alpha = alpha.min(1.0);
        for y in y0..y1 {
            let dy = y as f32 - c.1;
            for x in x0..x1 {
                let dx = x as f32 - c.0;
                let sector = ((dy.atan2(dx) - angle) / step).floor().rem_euclid(sides as f32) as usize;
                let (nx, ny) = normals[sector.min(sides - 1)];
                let cover = (apothem - (dx * nx + dy * ny) + 0.5).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    continue;
                }
                let target = match fill {
                    Fill::Light(light, u) => [light, light * u.clamp(0.0, 1.0)],
                    Fill::Copy { scale, twist } => {
                        let (s, co) = twist.sin_cos();
                        let (sx, sy) = (mid_x + (co * dx - s * dy) * scale, mid_y + (s * dx + co * dy) * scale);
                        sample_sharp(&self.tmp, w, h, sx, sy, 1.0)
                    }
                };
                let k = cover * alpha;
                let p = &mut self.px[y * w + x];
                p[0] += (target[0] - p[0]) * k;
                p[1] += (target[1] - p[1]) * k;
            }
        }
    }

    /// The light at a point, and the light times its gradient position, bilinearly
    /// interpolated. Points outside the field are dark.
    #[inline]
    pub fn sample(&self, x: f32, y: f32) -> (f32, f32) {
        if !(x >= 0.0 && y >= 0.0 && x < self.w as f32 - 1.0 && y < self.h as f32 - 1.0) {
            return (0.0, 0.0);
        }
        let (ix, iy) = (x as usize, y as usize);
        let (fx, fy) = (x - ix as f32, y - iy as f32);
        let i = iy * self.w + ix;
        let (p00, p10, p01, p11) = (self.px[i], self.px[i + 1], self.px[i + self.w], self.px[i + self.w + 1]);
        let top = |c: usize| p00[c] + (p10[c] - p00[c]) * fx;
        let bottom = |c: usize| p01[c] + (p11[c] - p01[c]) * fx;
        (top(0) + (bottom(0) - top(0)) * fy, top(1) + (bottom(1) - top(1)) * fy)
    }

    /// Replace the field with a procedural light layer. Each sample is (energy, hue).
    pub fn paint(&mut self, sample: impl Fn(usize, usize) -> (f32, f32) + Sync) {
        let w = self.w;
        bands(&mut self.px, w, |y0, band| {
            for (r, row) in band.chunks_exact_mut(w).enumerate() {
                for (x, p) in row.iter_mut().enumerate() {
                    let (e, u) = sample(x, y0 + r);
                    *p = [e, e * u.clamp(0.0, 1.0)];
                }
            }
        });
    }

    pub fn add_scaled(&mut self, other: &Field, amount: f32) {
        assert_eq!((self.w, self.h), (other.w, other.h));
        for (p, q) in self.px.iter_mut().zip(&other.px) {
            p[0] += q[0] * amount;
            p[1] += q[1] * amount;
        }
    }

    pub fn composite(&self, c: &mut Canvas, pal: &Palette, opacity: f32) {
        for (p, &[e, eu]) in c.px.iter_mut().zip(&self.px) {
            if e > 1e-5 {
                let alpha = (1.0 - 1.0 / (1.0 + e * (1.0 + e * 0.5))) * opacity;
                *p = pack(crate::canvas::mix(crate::canvas::unpack(*p), pal.glow(eu / e), alpha));
            }
        }
    }

    /// Paints the field onto the top left of `c`. The light sets how far each pixel rises from
    /// the screen color toward its gradient color. The brightest pixels run hot toward the
    /// skin's peak color.
    pub fn resolve(&self, c: &mut Canvas, pal: &Palette) {
        self.resolve_at(c, pal, 1.0);
    }

    /// Limit background brightness so a separately drawn foreground remains prominent.
    pub fn resolve_at(&self, c: &mut Canvas, pal: &Palette, brightness: f32) {
        let bg = pal.skin.screen.map(|v| v as f32);
        let hot = pal.skin.peak.map(|v| v as f32);
        let screen = pack(pal.skin.screen);
        let (fw, cw) = (self.w, c.w);
        let w = fw.min(cw);
        let rows = self.h.min(c.h);
        bands(&mut c.px[..rows * cw], cw, |y0, band| {
            for (r, out) in band.chunks_exact_mut(cw).enumerate() {
                let src = &self.px[(y0 + r) * fw..][..w];
                for (o, &[e, eu]) in out[..w].iter_mut().zip(src) {
                    *o = if e < 1e-3 {
                        screen
                    } else {
                        let col = pal.glow(eu / e);
                        // Close to 1 - exp(-e), for the cost of one division.
                        let lit = (1.0 - 1.0 / (1.0 + e * (1.0 + e * (0.5 + e / 6.0)))) * brightness;
                        let heat = ((e - 3.0) * 0.06).clamp(0.0, 0.16);
                        let mut out = 0u32;
                        for ch in 0..3 {
                            let v = col[ch] as f32 + (hot[ch] - col[ch] as f32) * heat;
                            out = out << 8 | (bg[ch] + (v - bg[ch]) * lit) as u32;
                        }
                        out
                    };
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(w: usize, h: usize) -> Field {
        let mut f = Field::default();
        f.fit(w, h);
        for k in 0..12 {
            let k = k as f32;
            f.dab(8.0 + k * 9.0, 10.0 + (k * 1.7).sin() * 20.0 + k * 3.0, 4.0 + k * 0.5, k / 12.0, 0.4 + k * 0.05);
        }
        f.line(5.0, 70.0, 110.0, 15.0, 2.5, 0.1, 0.9, 0.8);
        f
    }

    fn close(a: &Field, b: &Field, tolerance: f32) -> bool {
        a.px.iter().zip(&b.px).all(|(p, q)| (p[0] - q[0]).abs() <= tolerance && (p[1] - q[1]).abs() <= tolerance)
    }

    #[test]
    fn identity_grid_leaves_the_light_in_place() {
        let (w, h) = (120, 80);
        let start = lit(w, h);
        let mut f = lit(w, h);
        f.warp_grid(&Grid::build(w, h, |x, y| [x, y, 1.0]), false);
        // The last row and column sample past the edge, as in `warp`.
        for y in 0..h - 1 {
            for x in 0..w - 1 {
                let (p, q) = (f.px[y * w + x], start.px[y * w + x]);
                assert!((p[0] - q[0]).abs() < 1e-4 && (p[1] - q[1]).abs() < 1e-4, "({x}, {y})");
            }
        }
    }

    #[test]
    fn grid_of_one_zoom_and_turn_matches_warp() {
        let (w, h) = (120, 80);
        let (from, to, zoom, rot, keep) = ((61.0, 39.5), (58.0, 42.0), 1.03, 0.05, 0.97);
        for spread in [false, true] {
            let mut by_warp = lit(w, h);
            by_warp.warp(from, to, zoom, rot, keep, spread);
            let mut by_grid = lit(w, h);
            let (sn, cs) = f32::sin_cos(rot);
            by_grid.warp_grid(&Grid::build(w, h, |x, y| {
                let (dx, dy) = (x - to.0, y - to.1);
                [from.0 + (cs * dx + sn * dy) / zoom, from.1 + (cs * dy - sn * dx) / zoom, keep]
            }), spread);
            assert!(close(&by_warp, &by_grid, 1e-3), "spread {spread}");
        }
    }

    #[test]
    fn grid_can_keep_less_light_in_some_places() {
        let (w, h) = (120, 80);
        let mut f = lit(w, h);
        let before: f32 = f.px.iter().map(|p| p[0]).sum();
        f.warp_grid(&Grid::build(w, h, |x, y| [x, y, if x < 60.0 { 1.0 } else { 0.0 }]), false);
        let left: f32 = (0..h).flat_map(|y| (0..30).map(move |x| (x, y))).map(|(x, y)| f.px[y * w + x][0]).sum();
        let right: f32 = (0..h).flat_map(|y| (w - 30..w).map(move |x| (x, y))).map(|(x, y)| f.px[y * w + x][0]).sum();
        assert!(left > 0.1 && right == 0.0, "left {left}, right {right}, before {before}");
    }
}
