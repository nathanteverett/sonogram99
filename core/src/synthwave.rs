//! Synthwave: a striped sun sets between two ranges of wireframe mountains while the viewer
//! travels over a glowing grid. The grid rolls toward the viewer at a steady speed. The
//! mountains follow the spectrum slowly, bass at the outer edges and treble near the sun, so
//! their heights drift with the music but never pulse on a beat.

use std::f32::consts::TAU;

use crate::analysis::Frame;
use crate::canvas::{pack, Canvas, Rgb};
use crate::modes::{band_max, shape, Mode};
use crate::palette::Palette;

/// Lengths are in grid cells. The height of the eye above the ground.
const EYE: f32 = 2.0;
/// The horizon, as a share of the picture height from the top.
const HORIZON: f32 = 0.6;
/// The focal length, as a share of the picture height.
const FOCAL: f32 = 0.9;
/// The ground speed in cells per second, and the most that loud music adds to it.
const SPEED: f32 = 1.5;
const SPEED_LOUD: f32 = 0.6;
/// How far the ground fades into the haze: the distance where the haze is two thirds thick.
const HAZE: f32 = 35.0;
/// The sun's radius, as a share of the picture height.
const SUN: f32 = 0.24;
/// The mountains: the nearest and farthest rows, the number of rows, the spacing of the
/// columns, and the height of the tallest peaks.
const NEAR: f32 = 110.0;
const FAR: f32 = 250.0;
const ROWS: usize = 9;
const STEP: f32 = 9.0;
const PEAK: f32 = 95.0;
/// The size of a typical mountain.
const RIDGE: f32 = 42.0;
/// Spectrum bands that move the mountains, from bass to treble, and their frequency range.
const BANDS: usize = 24;
const F_LOW: f32 = 40.0;
const F_HIGH: f32 = 8_000.0;
/// How much the music can change a mountain's height, as a share of it.
const SWAY: f32 = 0.45;
/// How fast the mountains follow the music, per second, as it gets louder and quieter.
/// Slow rates smooth each beat away, so the mountains drift instead of pulse.
const RISE: f32 = 1.4;
const FALL: f32 = 0.6;
const STARS: i32 = 150;

const SKY_TOP: Rgb = [6, 2, 18];
const SKY_LOW: Rgb = [92, 14, 96];
const SUN_TOP: Rgb = [255, 236, 64];
const SUN_MID: Rgb = [255, 142, 36];
const SUN_LOW: Rgb = [255, 34, 128];
const GLOW: Rgb = [255, 64, 150];
const GROUND_NEAR: Rgb = [4, 2, 20];
const GROUND_FAR: Rgb = [70, 12, 92];
const GRID: Rgb = [30, 200, 255];
const GRID_FAR: Rgb = [210, 80, 255];
const ROCK: Rgb = [18, 4, 36];
const WIRE: Rgb = [255, 56, 226];
const STAR: Rgb = [230, 214, 255];

#[derive(Default)]
pub struct Synthwave {
    /// How far the viewer has traveled, in cells.
    travel: f32,
    /// Seconds since the visual started. It moves the sun's stripes and the stars' twinkle.
    clock: f32,
    /// Each band's slow level, 0..1.
    bands: Vec<f32>,
    /// The slow overall level, which sets the ground speed.
    drive: f32,
}

/// The camera, in pixels.
struct View {
    w: f32,
    h: f32,
    cx: f32,
    horizon: f32,
    focal: f32,
}

impl View {
    fn new(c: &Canvas) -> Self {
        let (w, h) = (c.w as f32, c.h as f32);
        View { w, h, cx: w / 2.0, horizon: h * HORIZON, focal: h * FOCAL }
    }

    /// How strongly the glow along the horizon reaches each column: most at the center.
    fn spread(&self) -> Vec<f32> {
        (0..self.w as usize).map(|x| (-((x as f32 + 0.5 - self.cx) / (self.w * 0.3)).powi(2)).exp()).collect()
    }

    fn project(&self, x: f32, y: f32, z: f32) -> (f32, f32) {
        (self.cx + x * self.focal / z, self.horizon + (EYE - y) * self.focal / z)
    }
}

impl Mode for Synthwave {
    fn name(&self) -> &'static str {
        "SYNTHWAVE"
    }

    fn reset(&mut self, c: &mut Canvas) {
        c.fill(SKY_TOP);
        self.bands.clear();
        self.drive = 0.0;
    }

    fn draw(&mut self, c: &mut Canvas, f: &Frame, _pal: &Palette, dt: f32) {
        self.listen(f, dt);
        self.clock += dt;
        self.travel = (self.travel + dt * (SPEED + SPEED_LOUD * self.drive)) % 1024.0;
        let view = View::new(c);
        self.sky(c, &view);
        self.stars(c, &view);
        self.ground(c, &view);
        self.mountains(c, &view);
    }
}

impl Synthwave {
    fn listen(&mut self, f: &Frame, dt: f32) {
        if self.bands.len() != BANDS {
            self.bands = vec![0.0; BANDS];
        }
        let ratio = F_HIGH / F_LOW;
        for (i, b) in self.bands.iter_mut().enumerate() {
            let f0 = F_LOW * ratio.powf(i as f32 / BANDS as f32);
            let f1 = F_LOW * ratio.powf((i + 1) as f32 / BANDS as f32);
            follow(b, shape(band_max(f, f0, f1), i as f32 / BANDS as f32), RISE, FALL, dt);
        }
        follow(&mut self.drive, f.level, 0.3, 0.3, dt);
    }

    /// The slow level at `u`, from 0 (bass) to 1 (treble).
    fn band_at(&self, u: f32) -> f32 {
        let p = u.clamp(0.0, 1.0) * (BANDS - 1) as f32;
        let i = (p as usize).min(BANDS - 2);
        let t = p - i as f32;
        self.bands[i] * (1.0 - t) + self.bands[i + 1] * t
    }

    /// The sky, the sun with its stripes, and the glow along the horizon.
    fn sky(&self, c: &mut Canvas, v: &View) {
        let rows = (v.horizon.ceil() as usize).min(c.h);
        let r = v.h * SUN;
        let (sx, sy) = (v.cx, v.horizon - r * 0.8);
        // The stripes cross the lower half of the sun. They drift down and widen as they go.
        let period = r * 0.17;
        let drift = self.clock * period * 0.3;
        let spread = v.spread();
        for y in 0..rows {
            let yc = y as f32 + 0.5;
            let sky = lerp(SKY_TOP, SKY_LOW, (yc / v.horizon).powf(1.8));
            let lift = (-(v.horizon - yc) / (v.h * 0.05)).exp();
            let u = (yc - (sy - r)) / (2.0 * r);
            let sun = if u < 0.5 { lerp(SUN_TOP, SUN_MID, u * 2.0) } else { lerp(SUN_MID, SUN_LOW, u * 2.0 - 1.0) };
            let s = (u - 0.45) / 0.55;
            let gap = if s > 0.0 {
                let p = (yc - sy - drift).rem_euclid(period);
                let width = period * (0.05 + 0.5 * s);
                let open = (p + 0.5).min(width - p + 0.5).clamp(0.0, 1.0).min(width);
                open.max((p - period + 0.5).clamp(0.0, 1.0))
            } else {
                0.0
            };
            let row = &mut c.px[y * c.w..(y + 1) * c.w];
            let dy = yc - sy;
            for (x, (out, spread)) in row.iter_mut().zip(&spread).enumerate() {
                let dx = x as f32 + 0.5 - sx;
                let d = (dx * dx + dy * dy).sqrt();
                let halo = if d < r * 4.0 { (-(d - r).max(0.0) / (r * 0.5)).exp() * 0.5 } else { 0.0 };
                let col = add(sky, GLOW, halo + lift * (0.25 + 0.5 * spread));
                let a = (r - d + 0.5).clamp(0.0, 1.0) * (1.0 - gap);
                *out = pack_f(mix_f(col, sun, a));
            }
        }
    }

    /// Fixed stars that twinkle slowly. They never cover the sun.
    fn stars(&self, c: &mut Canvas, v: &View) {
        let r = v.h * SUN;
        let (sx, sy) = (v.cx, v.horizon - r * 0.8);
        for i in 0..STARS {
            let x = hash(i, 0, 11) * v.w;
            let y = hash(i, 1, 11).powf(1.4) * v.horizon * 0.8;
            if (x - sx).hypot(y - sy) < r + 3.0 {
                continue;
            }
            let twinkle = 0.75 + 0.25 * (self.clock * (0.4 + hash(i, 3, 11)) + hash(i, 4, 11) * TAU).sin();
            let a = (0.2 + 0.8 * hash(i, 2, 11).powi(3)) * twinkle;
            let (px, py) = (x as i32, y as i32);
            c.add(px, py, STAR, a);
            if a > 0.6 {
                for (ox, oy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    c.add(px + ox, py + oy, STAR, a * 0.3);
                }
            }
        }
    }

    /// The ground below the horizon: a grid of glowing lines that rolls toward the viewer.
    fn ground(&self, c: &mut Canvas, v: &View) {
        let k = EYE * v.focal;
        let first = (v.horizon.floor() as usize).min(c.h);
        let spread = v.spread();
        for y in first..c.h {
            let dy = y as f32 + 0.5 - v.horizon;
            if dy <= 0.0 {
                continue;
            }
            let z = k / dy;
            let haze = 1.0 - (-z / HAZE).exp();
            // Lines across the view, one a cell. The travel moves them toward the viewer.
            let cell = dy * dy / k;
            let wz = z + self.travel;
            let across = (wz - wz.round()).abs() * cell;
            let fade_across = smooth(0.8, 3.5, cell);
            let line_across = core(across) * fade_across;
            let glow_across = (-across / 2.5).exp() * fade_across;
            let scale = v.focal / z;
            let base = lerp(GROUND_NEAR, GROUND_FAR, haze);
            let grid = lerp(GRID, GRID_FAR, haze.powf(0.7));
            let dim = 1.0 - haze * 0.5;
            let lift = (-dy / (v.h * 0.03)).exp();
            let row = &mut c.px[y * c.w..(y + 1) * c.w];
            for (x, (out, spread)) in row.iter_mut().zip(&spread).enumerate() {
                let sx = x as f32 + 0.5 - v.cx;
                // Lines into the distance meet at the center of the horizon. Their distance
                // across the screen shrinks as they lean, so measure it square to the line.
                let wx = sx / scale;
                let lean = dy / (sx * sx + dy * dy).sqrt();
                let along = (wx - wx.round()).abs() * scale * lean;
                let fade_along = smooth(0.8, 3.5, scale * lean);
                let line = (core(along) * fade_along).max(line_across);
                let glow = ((-along / 2.5).exp() * fade_along).max(glow_across);
                let col = add_f(base, grid, (line * 0.95 + glow * 0.3) * dim);
                *out = pack_f(add(col, GLOW, lift * (0.3 + 0.5 * spread)));
            }
        }
    }

    /// Two ranges of wireframe mountains on the horizon, with a low valley under the sun.
    /// Rows are drawn from the back to the front, so nearer rock hides the wires behind it.
    fn mountains(&self, c: &mut Canvas, v: &View) {
        let reach = (v.w * 0.5 + 40.0) * FAR / v.focal;
        let n = (reach / STEP).ceil() as i32;
        let cols = (2 * n + 1) as usize;
        let depth = (FAR - NEAR) / (ROWS - 1) as f32;
        let mut pts = Vec::with_capacity(ROWS * cols);
        for j in 0..ROWS as i32 {
            for i in -n..=n {
                // Uneven spacing breaks up the lattice. The front row stays on the ground.
                let x = i as f32 * STEP + (hash(i, j, 1) - 0.5) * STEP * 0.6;
                let z = NEAR + j as f32 * depth + if j > 0 { (hash(i, j, 2) - 0.5) * depth * 0.5 } else { 0.0 };
                // Across the screen, 0 at the center and 1 at the edges.
                let a = (x / z * v.focal / (v.w * 0.5)).abs();
                let valley = 0.1 + 0.9 * smooth(0.14, 0.62, a);
                let p = (z - NEAR) / (FAR - NEAR);
                let profile = smooth(0.0, 0.4, p) * (0.6 + 0.4 * p);
                let sway = 1.0 - SWAY * 0.5 + SWAY * self.band_at(1.0 - a);
                let height = PEAK * valley * profile * ridged(x / RIDGE, z / RIDGE) * sway;
                let (px, py) = v.project(x, height, z);
                pts.push((px, py, p));
            }
        }
        for j in (1..ROWS).rev() {
            for i in 0..cols - 1 {
                let near = (j - 1) * cols + i;
                let far = j * cols + i;
                let [a, b, d, e] = [pts[near], pts[near + 1], pts[far], pts[far + 1]];
                let left = a.0.min(d.0);
                let right = b.0.max(e.0);
                if right < -2.0 || left > v.w + 2.0 {
                    continue;
                }
                let p = (a.2 + b.2 + d.2 + e.2) / 4.0;
                let rock = pack_f(lerp(ROCK, SKY_LOW, p * 0.3));
                let wire = mix_f(f3(WIRE), f3(SKY_LOW), p * 0.45);
                let wire = [wire[0] as u8, wire[1] as u8, wire[2] as u8];
                // Alternate the diagonal so the mesh looks less regular.
                let tris = if hash(i as i32 - n, j as i32, 3) < 0.5 { [[a, b, e], [a, e, d]] } else { [[a, b, d], [b, e, d]] };
                for t in tris {
                    fill(c, [(t[0].0, t[0].1), (t[1].0, t[1].1), (t[2].0, t[2].1)], rock);
                }
                for t in tris {
                    for (s, f) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                        c.line(s.0, s.1, f.0, f.1, wire, 0.1, 3, true);
                        c.line(s.0, s.1, f.0, f.1, wire, 0.9, 1, false);
                    }
                }
            }
        }
    }
}

fn follow(value: &mut f32, target: f32, rise: f32, fall: f32, dt: f32) {
    let rate = if target > *value { rise } else { fall };
    *value += (target - *value) * (1.0 - (-rate * dt).exp());
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of a one-pixel line covers a pixel `d` pixels from its center.
fn core(d: f32) -> f32 {
    (1.1 - d).clamp(0.0, 1.0)
}

fn f3(c: Rgb) -> [f32; 3] {
    [c[0] as f32, c[1] as f32, c[2] as f32]
}

fn lerp(a: Rgb, b: Rgb, t: f32) -> [f32; 3] {
    mix_f(f3(a), f3(b), t.clamp(0.0, 1.0))
}

fn mix_f(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn add(a: [f32; 3], b: Rgb, t: f32) -> [f32; 3] {
    add_f(a, f3(b), t)
}

fn add_f(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + b[0] * t, a[1] + b[1] * t, a[2] + b[2] * t]
}

fn pack_f(c: [f32; 3]) -> u32 {
    pack([c[0].clamp(0.0, 255.0) as u8, c[1].clamp(0.0, 255.0) as u8, c[2].clamp(0.0, 255.0) as u8])
}

/// Fills the pixels whose centers lie inside the triangle.
fn fill(c: &mut Canvas, p: [(f32, f32); 3], col: u32) {
    let edge = |a: (f32, f32), b: (f32, f32), x: f32, y: f32| (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
    let area = edge(p[0], p[1], p[2].0, p[2].1);
    if area.abs() < 1e-6 {
        return;
    }
    let x0 = p[0].0.min(p[1].0).min(p[2].0).floor().max(0.0) as usize;
    let x1 = (p[0].0.max(p[1].0).max(p[2].0).ceil().max(0.0) as usize).min(c.w);
    let y0 = p[0].1.min(p[1].1).min(p[2].1).floor().max(0.0) as usize;
    let y1 = (p[0].1.max(p[1].1).max(p[2].1).ceil().max(0.0) as usize).min(c.h);
    for y in y0..y1 {
        let yc = y as f32 + 0.5;
        for x in x0..x1 {
            let xc = x as f32 + 0.5;
            let inside = [(1, 2), (2, 0), (0, 1)].iter().all(|&(s, e)| edge(p[s], p[e], xc, yc) * area >= 0.0);
            if inside {
                c.px[y * c.w + x] = col;
            }
        }
    }
}

/// A repeatable number from 0 to 1 for a pair of whole numbers.
fn hash(a: i32, b: i32, k: u32) -> f32 {
    let mut h = (a as u32).wrapping_mul(0x8DA6_B343) ^ (b as u32).wrapping_mul(0xD816_3841) ^ k.wrapping_mul(0xCB1A_B31F);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0xFF_FFFF) as f32 / 16_777_216.0
}

/// Smooth value noise from 0 to 1.
fn noise(x: f32, y: f32, k: u32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (tx, ty) = (x - xi, y - yi);
    let (tx, ty) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let (i, j) = (xi as i32, yi as i32);
    let top = hash(i, j, k) + (hash(i + 1, j, k) - hash(i, j, k)) * tx;
    let bottom = hash(i, j + 1, k) + (hash(i + 1, j + 1, k) - hash(i, j + 1, k)) * tx;
    top + (bottom - top) * ty
}

/// Noise with sharp crests, as on a mountain ridge. From 0 to 1.
fn ridged(x: f32, y: f32) -> f32 {
    let (mut sum, mut amp, mut freq, mut total) = (0.0, 1.0, 1.0, 0.0);
    for octave in 0..4 {
        let n = 1.0 - (noise(x * freq, y * freq, 20 + octave) * 2.0 - 1.0).abs();
        sum += n * n * amp;
        total += amp;
        amp *= 0.5;
        freq *= 2.1;
    }
    sum / total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analyzer;

    fn frame(level: f32) -> Frame {
        let mut audio = Analyzer::new();
        audio.frame.spec.fill(level);
        audio.frame.level = level;
        audio.frame.clone()
    }

    #[test]
    fn the_ground_moves_toward_the_viewer() {
        let pal = Palette::new(0);
        let mut mode = Synthwave::default();
        let mut c = Canvas::new(320, 200);
        mode.reset(&mut c);
        mode.draw(&mut c, &frame(0.0), &pal, 1.0 / 60.0);
        let before = c.px.clone();
        mode.draw(&mut c, &frame(0.0), &pal, 0.1);
        let bottom = 180 * 320..200 * 320;
        assert_ne!(before[bottom.clone()], c.px[bottom], "The grid near the viewer moves");
    }

    #[test]
    fn the_mountains_drift_instead_of_pulse() {
        let mut mode = Synthwave::default();
        mode.listen(&frame(0.0), 1.0 / 60.0);
        // One loud frame, as on a beat, barely moves the mountains.
        mode.listen(&frame(1.0), 1.0 / 60.0);
        assert!(mode.band_at(0.0) < 0.05, "A single beat must not make the mountains jump");
        // A second of loud music raises them most of the way.
        for _ in 0..60 {
            mode.listen(&frame(1.0), 1.0 / 60.0);
        }
        assert!(mode.band_at(0.0) > 0.6);
    }
}
