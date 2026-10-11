//! The twelve visual modes. Each one draws into the visualizer area of the screen.

use crate::analysis::{Frame, BINS, TD};
use crate::canvas::{mix, pack, Canvas};
use crate::font;
use crate::glow::{per_frame, Field};
use crate::interaction::Controls;
use crate::preset::Preset;
use crate::palette::{Palette, SCREEN, WARN};

pub const F_MIN: f32 = 30.0;
pub const F_MAX: f32 = 16_000.0;
const AXIS_F: [f32; 8] = [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0];

pub trait Mode {
    fn name(&self) -> &'static str;
    fn configure(&mut self, _preset: &Preset) {}
    fn interact(&mut self, _controls: Controls) {}
    fn reset(&mut self, c: &mut Canvas) {
        c.fill(SCREEN);
    }
    fn draw(&mut self, c: &mut Canvas, f: &Frame, pal: &Palette, dt: f32);
}

/// How many modes `all` returns.
pub const COUNT: usize = 12;

pub fn all() -> Vec<Box<dyn Mode>> {
    vec![
        Box::new(Bars::default()),
        Box::new(Scope { gain: 1.0 }),
        Box::new(Sonogram::default()),
        Box::new(Halo::default()),
        Box::new(Vector { gain: 2.0, ..Default::default() }),
        Box::new(crate::feedback::Tunnel::default()),
        Box::new(crate::fluid::Orbs::default()),
        Box::new(crate::kaleidoscope::Kaleidoscope::default()),
        Box::new(crate::geometry::Wireframe::default()),
        Box::new(crate::kaleidoscope::Mandala::default()),
        Box::new(crate::gravity::GravityWaves::default()),
        Box::new(crate::synthwave::Synthwave::default()),
    ]
}

// ---------- spectrum helpers ----------

/// Position of `f` on the log frequency axis, 0 at 30 Hz and 1 at 16 kHz.
fn fx(f: f32) -> f32 {
    (f / F_MIN).ln() / (F_MAX / F_MIN).ln()
}

fn f_label(f: f32) -> String {
    if f >= 1000.0 { format!("{}K", (f / 1000.0) as i32) } else { format!("{}", f as i32) }
}

fn bin_at(spec: &[f32], b: f32) -> f32 {
    if b <= 0.0 {
        return spec[0];
    }
    let i = b.floor() as usize;
    if i >= BINS - 1 {
        return spec[BINS - 1];
    }
    let t = b - i as f32;
    spec[i] * (1.0 - t) + spec[i + 1] * t
}

/// Loudest bin between two frequencies. Narrow low bands interpolate between bins.
pub(crate) fn band_max(f: &Frame, f0: f32, f1: f32) -> f32 {
    let hz = f.hz_per_bin();
    let (b0, b1) = (f0 / hz, f1 / hz);
    if b1 - b0 < 1.0 {
        return bin_at(&f.spec, (b0 + b1) * 0.5);
    }
    let e = (b1.ceil() as usize).min(BINS - 1);
    f.spec[b0.floor() as usize..=e].iter().fold(0.0f32, |m, &v| m.max(v))
}

/// Lifts the noise floor out and tilts the top end up a little, as hardware analyzers did.
pub(crate) fn shape(v: f32, x: f32) -> f32 {
    ((v - 0.1) / 0.9 * (0.92 + 0.22 * x)).clamp(0.0, 1.0)
}

fn freq_axis(c: &mut Canvas, y: i32, pal: &Palette) {
    let w = c.w as i32;
    let tick = mix(pal.skin.screen, pal.skin.ink, 0.45);
    let mut last_right = -100;
    for f in AXIS_F {
        let x = (fx(f) * w as f32).round() as i32;
        let s = f_label(f);
        let half = font::text_width(&s) / 2;
        if x - half < last_right + 6 {
            continue;
        }
        c.rect(x, y, 1, 2, tick);
        c.text_center(&s, x, y + 4, pal.skin.ink_dim);
        last_right = x + half;
    }
    if last_right < w - 20 {
        c.text_right("HZ", w - 3, y + 4, pal.skin.ink_dim);
    }
}

pub(crate) fn wave_peak(wave: &[f32], step: usize) -> f32 {
    wave.iter().step_by(step).fold(1e-4f32, |m, &v| m.max(v.abs()))
}

// ---------- 1. spectrum bars ----------

#[derive(Default)]
pub struct Bars {
    level: Vec<f32>,
    peak: Vec<f32>,
    hold: Vec<f32>,
    vel: Vec<f32>,
}

impl Mode for Bars {
    fn name(&self) -> &'static str {
        "BARS"
    }

    fn reset(&mut self, c: &mut Canvas) {
        c.fill(SCREEN);
        self.level.clear();
    }

    fn draw(&mut self, c: &mut Canvas, f: &Frame, pal: &Palette, dt: f32) {
        let (w, h) = (c.w as i32, c.h as i32);
        let n = (w / 8).clamp(16, 64) as usize;
        if self.level.len() != n {
            self.level = vec![0.0; n];
            self.peak = vec![0.0; n];
            self.hold = vec![0.0; n];
            self.vel = vec![0.0; n];
        }
        c.fill(pal.skin.screen);
        let top = 4;
        let plot_h = (h - top - 13).max(8);
        let base = top + plot_h;
        let bw = w as f32 / n as f32;
        let gap = ((bw * 0.22).round() as i32).max(1);
        let seg_h = (plot_h / 28).max(2);
        let segs = (plot_h / seg_h).max(4);
        let lit_col: Vec<_> = (0..segs).map(|s| pal.bar(s as f32 / (segs - 1) as f32)).collect();
        let dim_col: Vec<_> = lit_col.iter().map(|&col| mix(pal.skin.screen, col, 0.055)).collect();
        let ratio = F_MAX / F_MIN;
        for i in 0..n {
            let f0 = F_MIN * ratio.powf(i as f32 / n as f32);
            let f1 = F_MIN * ratio.powf((i + 1) as f32 / n as f32);
            let v = shape(band_max(f, f0, f1), i as f32 / n as f32);
            let d = &mut self.level[i];
            *d = if v > *d { v } else { v.max(*d - dt * 1.8) };
            let d = *d;
            // Peak caps hold for a moment, then fall with gravity.
            if d >= self.peak[i] {
                self.peak[i] = d;
                self.hold[i] = 0.45;
                self.vel[i] = 0.0;
            } else if self.hold[i] > 0.0 {
                self.hold[i] -= dt;
            } else {
                self.vel[i] += dt * 2.2;
                self.peak[i] = d.max(self.peak[i] - self.vel[i] * dt);
            }
            let x0 = (i as f32 * bw + gap as f32 / 2.0).round() as i32;
            let x1 = ((i + 1) as f32 * bw - gap as f32 / 2.0).round() as i32;
            let bar_w = (x1 - x0).max(1);
            let lit = (d * segs as f32).round() as i32;
            for s in 0..segs {
                let col = if s < lit { lit_col[s as usize] } else { dim_col[s as usize] };
                c.rect(x0, base - (s + 1) * seg_h + 1, bar_w, seg_h - 1, col);
            }
            if self.peak[i] > 0.015 {
                let ps = ((self.peak[i] * segs as f32).round() as i32).max(lit).min(segs - 1);
                c.rect(x0, base - (ps + 1) * seg_h + 1, bar_w, ((seg_h - 1) / 2).max(1), pal.skin.peak);
            }
        }
        freq_axis(c, base + 2, pal);
    }
}

// ---------- 2. oscilloscope ----------

pub struct Scope {
    gain: f32,
}

impl Mode for Scope {
    fn name(&self) -> &'static str {
        "SCOPE"
    }

    fn draw(&mut self, c: &mut Canvas, f: &Frame, pal: &Palette, dt: f32) {
        let (w, h) = (c.w as i32, c.h as i32);
        let ink = pal.skin.ink;
        c.fade(pal.skin.screen, 0.58);
        // Graticule: 10 x 8 divisions with minor ticks on the center lines.
        for i in 1..10 {
            c.rect_blend(i * w / 10, 0, 1, h, ink, 0.05);
        }
        for i in 1..8 {
            c.rect_blend(0, i * h / 8, w, 1, ink, 0.05);
        }
        c.rect_blend(0, h / 2, w, 1, ink, 0.11);
        c.rect_blend(w / 2, 0, 1, h, ink, 0.11);
        for i in 0..=50 {
            c.rect_blend(i * w / 50, h / 2 - 2, 1, 5, ink, 0.11);
        }
        for i in 0..=40 {
            c.rect_blend(w / 2 - 2, i * h / 40, 5, 1, ink, 0.11);
        }
        // Trigger on the first upward zero crossing, as a real scope does.
        let span = 1024;
        let wave = &f.mid;
        let start = (1..TD - span).find(|&i| wave[i - 1] < 0.0 && wave[i] >= 0.0).unwrap_or(0);
        let pk = wave_peak(&wave[start..start + span], 1);
        let target = (0.8 / pk).clamp(1.0, 8.0);
        self.gain += (target - self.gain) * (dt * 2.0).min(1.0);
        let amp = h as f32 * 0.45 * self.gain;
        let mid = h as f32 / 2.0;
        let pts: Vec<(f32, f32)> = (0..span)
            .map(|i| (i as f32 / (span - 1) as f32 * w as f32, mid - wave[start + i] * amp))
            .collect();
        c.polyline(&pts, ink, 0.18, 3, false);
        c.polyline(&pts, ink, 1.0, 1, false);
        let ms_div = span as f32 / f.sample_rate * 1000.0 / 10.0;
        c.label(&format!("CH1  {ms_div:.1} MS/DIV  GAIN X{:.1}", self.gain), 6, 6, pal.skin.ink_dim, pal.skin.screen);
    }
}

// ---------- 3. sonogram ----------

#[derive(Default)]
pub struct Sonogram {
    buf: Vec<u32>,
    bw: usize,
    bh: usize,
    lo: Vec<f32>,
    hi: Vec<f32>,
    acc: f32,
    clock: f32,
    sr: f32,
    background: crate::canvas::Rgb,
}

const SONO_GUTTER: i32 = 24;
const SONO_AXIS: i32 = 11;
/// Scroll speed in pixels per second.
const SONO_SPEED: f32 = 45.0;

impl Sonogram {
    fn setup(&mut self, w: usize, h: usize, f: &Frame, pal: &Palette) {
        self.background = pal.skin.screen;
        self.bw = w.saturating_sub(SONO_GUTTER as usize).max(1);
        self.bh = h.saturating_sub(SONO_AXIS as usize).max(1);
        self.buf = vec![pack(pal.skin.screen); self.bw * self.bh];
        let hz = f.hz_per_bin();
        let r = F_MAX / F_MIN;
        let bh = self.bh as f32;
        self.hi = (0..self.bh).map(|y| F_MIN * r.powf(1.0 - y as f32 / bh) / hz).collect();
        self.lo = (0..self.bh).map(|y| F_MIN * r.powf(1.0 - (y + 1) as f32 / bh) / hz).collect();
        self.acc = 0.0;
        self.clock = 0.0;
        self.sr = f.sample_rate;
    }

    fn paint(&mut self, x: usize, f: &Frame, pal: &Palette) {
        for y in 0..self.bh {
            let (lo, hi) = (self.lo[y], self.hi[y]);
            let v = if hi - lo < 1.0 {
                bin_at(&f.spec, (lo + hi) * 0.5)
            } else {
                let e = (hi.ceil() as usize).min(BINS - 1);
                f.spec[lo.floor() as usize..=e].iter().fold(0.0f32, |m, &v| m.max(v))
            };
            let n = ((v - 0.12) / 0.88).clamp(0.0, 1.0).powf(1.35);
            self.buf[y * self.bw + x] = pack(pal.heat(n));
        }
        let before = self.clock;
        self.clock += 1.0 / SONO_SPEED;
        if self.clock.floor() != before.floor() {
            // A tick every second, a longer one every five.
            let len = if (self.clock.floor() as i32) % 5 == 0 { 5 } else { 2 };
            let tick = pack(mix(pal.skin.screen, pal.skin.ink, 0.7));
            for y in self.bh.saturating_sub(len)..self.bh {
                self.buf[y * self.bw + x] = tick;
            }
        }
    }
}

impl Mode for Sonogram {
    fn name(&self) -> &'static str {
        "SONOGRAM"
    }

    fn reset(&mut self, c: &mut Canvas) {
        c.fill(SCREEN);
        self.buf.clear();
    }

    fn draw(&mut self, c: &mut Canvas, f: &Frame, pal: &Palette, dt: f32) {
        let want_w = c.w.saturating_sub(SONO_GUTTER as usize).max(1);
        let want_h = c.h.saturating_sub(SONO_AXIS as usize).max(1);
        if self.buf.is_empty() || self.bw != want_w || self.bh != want_h || self.sr != f.sample_rate || self.background != pal.skin.screen {
            self.setup(c.w, c.h, f, pal);
        }
        self.acc += dt * SONO_SPEED;
        let k = (self.acc.floor() as usize).min(self.bw);
        if k > 0 {
            self.acc -= k as f32;
            let bw = self.bw;
            for row in 0..self.bh {
                self.buf.copy_within(row * bw + k..row * bw + bw, row * bw);
            }
            for x in bw - k..bw {
                self.paint(x, f, pal);
            }
        }
        c.fill(pal.skin.screen);
        for y in 0..self.bh.min(c.h) {
            let d = y * c.w + SONO_GUTTER as usize;
            c.px[d..d + self.bw].copy_from_slice(&self.buf[y * self.bw..(y + 1) * self.bw]);
        }
        let tick = mix(pal.skin.screen, pal.skin.ink, 0.45);
        let mut last_y = i32::MAX;
        for fr in AXIS_F {
            let y = ((1.0 - fx(fr)) * self.bh as f32).round() as i32;
            if last_y - y < 11 {
                continue;
            }
            last_y = y;
            c.rect(SONO_GUTTER - 3, y, 3, 1, tick);
            c.text_right(&f_label(fr), SONO_GUTTER - 5, y - 3, pal.skin.ink_dim);
        }
        c.text_right("HZ", SONO_GUTTER - 5, 2, pal.skin.ink_dim);
        let y = self.bh as i32 + 3;
        c.text(&format!("-{}S", (self.bw as f32 / SONO_SPEED).round() as i32), SONO_GUTTER + 2, y, pal.skin.ink_dim);
        c.text_right("NOW", c.w as i32 - 3, y, pal.skin.ink_dim);
    }
}

// ---------- 4. halo ----------

#[derive(Default)]
pub struct Halo {
    rot: f32,
    level: Vec<f32>,
    field: Field,
}

impl Mode for Halo {
    fn name(&self) -> &'static str {
        "HALO"
    }

    fn reset(&mut self, c: &mut Canvas) {
        c.fill(SCREEN);
        self.field.clear();
    }

    fn draw(&mut self, c: &mut Canvas, f: &Frame, pal: &Palette, dt: f32) {
        const N: usize = 48;
        if self.level.len() != N {
            self.level = vec![0.0; N];
        }
        self.field.fit(c.w, c.h);
        let (w, h) = (c.w as f32, c.h as f32);
        let (cx, cy, m) = (w / 2.0, h / 2.0, w.min(h));
        // Last frame's light flies outward and swirls, so every spoke trails a flame.
        let spin = dt * (0.08 + f.level * 0.25);
        self.field.warp((cx, cy), (cx, cy), per_frame(1.009 + f.bass * 0.016, dt), spin * 3.0, per_frame(0.96, dt), true);
        self.rot += spin;
        let r0 = m * 0.19 * (1.0 + f.bass * 0.22);
        let len = m * 0.27;
        let ratio = 12_000.0 / F_MIN;
        for i in 0..N {
            let v = shape(
                band_max(f, F_MIN * ratio.powf(i as f32 / N as f32), F_MIN * ratio.powf((i + 1) as f32 / N as f32)),
                i as f32 / N as f32,
            );
            let s = &mut self.level[i];
            *s = if v > *s { v } else { v.max(*s - dt * 1.5) };
        }
        // Glowing core that swells with the kick drum.
        let rr = r0 * 0.72;
        self.field.dab(cx, cy, rr * 1.2, 0.0, 0.012 + f.bass * 0.07);
        // Spokes: bass at the bottom, treble at the top, mirrored left and right. Each runs
        // from a deep color at its root to a hot one at its tip, and loud spokes run hotter.
        let brush = (std::f32::consts::PI * r0 / N as f32 * 0.55).max(1.5);
        let mut spokes = Vec::with_capacity(N * 2);
        for i in 0..N {
            let v = self.level[i];
            let l = 2.0 + v * len;
            let u = 0.55 * i as f32 / N as f32 + 0.3 * v;
            for side in [-1.0f32, 1.0] {
                let a = std::f32::consts::FRAC_PI_2 + side * (i as f32 + 0.5) / N as f32 * std::f32::consts::PI + self.rot;
                let (sn, cs) = a.sin_cos();
                let (tx, ty) = (cx + cs * (r0 + l), cy + sn * (r0 + l));
                self.field.line(cx + cs * r0, cy + sn * r0, tx, ty, brush, u, u + 0.2, 0.2 + v * 0.35);
                self.field.dab(tx, ty, brush * 1.7, u + 0.25, v * 0.3);
                spokes.push((cx + cs * r0, cy + sn * r0, tx, ty, u, v));
            }
        }
        // Inner ring traces the waveform.
        const PTS: usize = 256;
        let stride = TD / PTS;
        let g = (0.6 / wave_peak(&f.mid, 8)).min(4.0);
        let mut pts: Vec<(f32, f32)> = (0..PTS)
            .map(|i| {
                let a = i as f32 / PTS as f32 * std::f32::consts::TAU - self.rot * 0.5;
                let rad = rr + f.mid[i * stride] * rr * 0.45 * g;
                (cx + a.cos() * rad, cy + a.sin() * rad)
            })
            .collect();
        pts.push(pts[0]);
        let ring: Vec<_> = pts.iter().map(|&(x, y)| (x, y, 0.85)).collect();
        self.field.polyline(&ring, 1.6, 0.4);
        self.field.resolve_at(c, pal, 0.72);
        for (x, y, tx, ty, u, v) in spokes {
            c.line(x, y, tx, ty, pal.glow(u + 0.12), 0.35 + v * 0.55, 1, false);
        }
        c.polyline(&pts, pal.skin.peak, 0.45, 1, false);
    }
}

// ---------- 5. vectorscope ----------

#[derive(Default)]
pub struct Vector {
    gain: f32,
    corr: f32,
    field: Field,
}

impl Mode for Vector {
    fn name(&self) -> &'static str {
        "VECTOR"
    }

    fn reset(&mut self, c: &mut Canvas) {
        c.fill(SCREEN);
        self.field.clear();
    }

    fn draw(&mut self, c: &mut Canvas, f: &Frame, pal: &Palette, dt: f32) {
        let ink = pal.skin.ink;
        let dim = pal.skin.ink_dim;
        let (w, h) = (c.w as i32, c.h as i32);
        let meter = 22;
        let ph = (h - meter).max(1);
        let (cx, cy) = (w as f32 / 2.0, ph as f32 / 2.0);
        let r = (w.min(ph) as f32) * 0.44;
        let d = r * std::f32::consts::FRAC_1_SQRT_2;
        // The trace glows like phosphor: the light lingers, spreads, and drifts slowly outward.
        self.field.fit(w as usize, ph as usize);
        self.field.warp((cx, cy), (cx, cy), per_frame(1.006, dt), 0.0, per_frame(0.95, dt), true);

        let (mut slr, mut sll, mut srr, mut pk) = (0.0f32, 0.0f32, 0.0f32, 1e-4f32);
        for i in 0..TD {
            let (l, r) = (f.left[i], f.right[i]);
            slr += l * r;
            sll += l * l;
            srr += r * r;
            pk = pk.max((l + r).abs().max((l - r).abs()) * std::f32::consts::FRAC_1_SQRT_2);
        }
        let corr = if sll > 1e-9 && srr > 1e-9 { slr / (sll * srr).sqrt() } else { 0.0 };
        self.corr += (corr - self.corr) * (dt * 4.0).min(1.0);
        let target = (0.85 / pk).clamp(1.0, 10.0);
        self.gain += (target - self.gain) * (dt * 1.5).min(1.0);
        // Goniometer: mono is vertical, left-only leans to the upper left. The beam spends the
        // same time on every step, so slow movements burn bright and fast jumps stay faint, as
        // on a real tube. The color runs from deep at the center to hot at the rim.
        let k = r * self.gain * std::f32::consts::FRAC_1_SQRT_2;
        let brush = (r * 0.022).max(1.5);
        let beam = 0.22;
        let mut last: Option<(f32, f32, f32)> = None;
        for i in 0..TD {
            let (l, rt) = (f.left[i], f.right[i]);
            let (x, y) = (cx + (rt - l) * k, cy - (l + rt) * k);
            let u = 0.05 + 0.95 * ((x - cx).hypot(y - cy) / r).min(1.0);
            if let Some((px, py, pu)) = last {
                let seg = (x - px).hypot(y - py).max(brush * 0.5);
                self.field.line(px, py, x, y, brush, pu, u, beam / seg);
            }
            last = Some((x, y, u));
        }
        self.field.resolve(c, pal);
        // Grid: the outer circle, the M/S axes, and the L/R diagonals.
        let circle: Vec<(f32, f32)> = (0..=120)
            .map(|i| {
                let a = i as f32 / 120.0 * std::f32::consts::TAU;
                (cx + a.cos() * r, cy + a.sin() * r)
            })
            .collect();
        c.polyline(&circle, ink, 0.2, 1, false);
        c.line(cx - r, cy, cx + r, cy, ink, 0.12, 1, false);
        c.line(cx, cy - r, cx, cy + r, ink, 0.12, 1, false);
        c.line(cx - d, cy - d, cx + d, cy + d, ink, 0.12, 1, false);
        c.line(cx + d, cy - d, cx - d, cy + d, ink, 0.12, 1, false);
        c.label("M", cx as i32 - 2, (cy - r) as i32 - 11, dim, pal.skin.screen);
        c.label("L", (cx - d) as i32 - 12, (cy - d) as i32 - 10, dim, pal.skin.screen);
        c.label("R", (cx + d) as i32 + 7, (cy - d) as i32 - 10, dim, pal.skin.screen);

        // Phase correlation meter: +1 is mono, 0 is unrelated, below 0 the channels cancel.
        c.rect(0, ph, w, meter, pal.skin.screen);
        let (mx, mw) = (w / 5 + 10, w * 3 / 5 - 20);
        let (my, mh) = (ph + 9, 5);
        c.rect(mx, my, mw, mh, mix(pal.skin.screen, ink, 0.14));
        let zero = mx + mw / 2;
        let pos = mx + (((self.corr + 1.0) / 2.0).clamp(0.0, 1.0) * mw as f32) as i32;
        if self.corr < 0.0 {
            c.rect(pos, my, zero - pos, mh, WARN);
        } else {
            // The fill heats up along the glow gradient as the sound narrows to mono.
            for x in zero..pos {
                c.rect(x, my, 1, mh, pal.glow((x - zero) as f32 / (mw / 2) as f32));
            }
        }
        c.rect(pos - 1, my - 2, 2, mh + 4, pal.skin.peak);
        for i in 0..=4 {
            c.rect(mx + i * mw / 4, my + mh + 1, 1, 2, mix(pal.skin.screen, ink, 0.5));
        }
        c.text_right("-1", mx - 5, my - 1, dim);
        c.text("+1", mx + mw + 5, my - 1, dim);
        c.text("PHASE", mx, ph, dim);
        c.text_right(&format!("{:+.2}", self.corr), mx + mw, ph, ink);
    }
}

// 6. Tunnel lives in feedback.rs: it draws everything from the recipe.
