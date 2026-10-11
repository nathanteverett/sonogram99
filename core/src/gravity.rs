//! Gravity Waves: two black holes orbit each other, spiral in, and collide. Then a new
//! partner falls in from outside the picture. The orbit sends out a two-armed spiral of
//! gravitational waves. Bass pulls the holes together, so they orbit faster and the arms wind
//! tighter. The waves travel outward at a fixed speed, so each change of the orbit sweeps
//! across the picture, and the picture shows the last few seconds of the orbit at once.

use std::collections::VecDeque;
use std::f32::consts::{FRAC_1_SQRT_2, PI, TAU};

use crate::analysis::Frame;
use crate::canvas::{mix, pack, unpack, Canvas};
use crate::glow::{per_frame, Field, Flow};
use crate::interaction::{variation, Controls, Hand};
use crate::modes::Mode;
use crate::palette::{Palette, SCREEN};

/// Lengths are in shorter sides of the picture, times the mouse-wheel scale.
/// The speed of the waves, per second.
const SPEED: f32 = 0.2;
/// The separation of the pair at the start of an inspiral.
const WIDE: f32 = 0.17;
/// The radius of a hole of mass 1. The two holes of a pair have a total mass of 2.
const HORIZON: f32 = 0.028;
/// The mass of the hole after a merger. The waves carry the rest away.
const REMNANT: f32 = 1.9;
/// Where a new partner starts to fall in, beyond the edge of the picture.
const FAR: f32 = 1.1;
/// The angular speed of the orbit at the separation `WIDE`, in radians per second.
const SPIN: f32 = 2.5;
/// The most that bass and beats can close the pair, as a share of its separation.
const SQUEEZE: f32 = 0.55;
/// About how long the pair takes to spiral in with loud music, in seconds. Quiet music
/// takes up to three times as long.
const CYCLE: f32 = 30.0;
/// Seconds the merged hole rings before a new partner appears, and the time constant of
/// the ringing.
const RINGDOWN: f32 = 1.8;
const RING_DECAY: f32 = 0.35;
/// Seconds a new partner takes to fall in to the separation `WIDE`.
const CAPTURE: f32 = 3.5;
/// The time constant of the flash at a merger, in seconds.
const FLASH: f32 = 0.3;
/// Einstein radius of a hole, as a multiple of its own radius. Light that passes near a hole
/// bends around it.
const LENS: f32 = 1.5;
/// The faint threads of space that ripple as the waves pass, and the small crosses that
/// stretch and squeeze.
const THREADS: u32 = 8;
const MARKERS: u32 = 40;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    /// The pair spirals in. The progress runs from 0 toward 1, where the separation reaches 0.
    Inspiral(f32),
    /// The merged hole rings. Seconds since the merger, and the angular speed of the ringing.
    Ringdown(f32, f32),
    /// A new partner falls in. Seconds since it started.
    Capture(f32),
}

/// What the pair sent out at one moment.
#[derive(Clone, Copy)]
struct Sample {
    t: f64,
    /// The phase of the waves, twice the orbital angle, and its rate in radians per second.
    phase: f64,
    rate: f64,
    amp: f32,
    /// The center of mass, as a share of the width and the height.
    x: f32,
    y: f32,
}

/// The waves at one distance from the source: what the pair sent out when the waves that are
/// now at that distance left it. The center is in pixels.
#[derive(Clone, Copy, Default)]
struct Ring {
    cos: f32,
    sin: f32,
    amp: f32,
    x: f32,
    y: f32,
}

pub struct GravityWaves {
    /// The waves as the pair sends them now.
    body: Field,
    /// The waves smeared by their motion.
    trails: Field,
    /// Newest last. It reaches back as long as the waves take to cross the picture.
    history: VecDeque<Sample>,
    /// One ring for each pixel of distance from the source.
    table: Vec<Ring>,
    t: f64,
    /// The orbital angle of the pair.
    angle: f64,
    stage: Stage,
    /// Merged pairs since the reset. Each new pair has its own mass ratio.
    cycle: u32,
    /// How far bass and beats close the pair, 0 to `SQUEEZE`.
    squeeze: f32,
    energy: f32,
    /// The amplitude of the ringing after a merger, and the flash of light at the merger.
    ring: f32,
    flash: f32,
    controls: Controls,
    hand: Hand,
}

impl Default for GravityWaves {
    fn default() -> Self {
        GravityWaves {
            body: Field::default(),
            trails: Field::default(),
            history: VecDeque::new(),
            table: Vec::new(),
            t: 0.0,
            angle: 0.0,
            stage: Stage::Inspiral(0.0),
            cycle: 0,
            squeeze: 0.0,
            energy: 0.0,
            ring: 0.0,
            flash: 0.0,
            controls: Controls::default(),
            hand: Hand::default(),
        }
    }
}

fn follow(value: &mut f32, target: f32, attack: f32, release: f32, dt: f32) {
    let rate = if target > *value { attack } else { release };
    *value += (target - *value) * (1.0 - (-rate * dt).exp());
}

/// The angular speed of an orbit at separation `sep`. Closer pairs orbit faster (Kepler).
fn kepler(sep: f32) -> f32 {
    SPIN * (WIDE / sep.max(1e-3)).powf(1.5)
}

/// The ring at `d` pixels from the source, between the two nearest entries.
#[inline]
fn ring(table: &[Ring], d: f32) -> Ring {
    let i = d as usize;
    if i + 1 >= table.len() {
        return table[table.len() - 1];
    }
    let k = d - i as f32;
    let (a, b) = (table[i], table[i + 1]);
    Ring {
        cos: a.cos + (b.cos - a.cos) * k,
        sin: a.sin + (b.sin - a.sin) * k,
        amp: a.amp + (b.amp - a.amp) * k,
        x: a.x + (b.x - a.x) * k,
        y: a.y + (b.y - a.y) * k,
    }
}

/// The waves in the picture: the table, where the pair is now in pixels, and `stretch`,
/// which undoes the tilt of the orbit.
struct Waves<'a> {
    table: &'a [Ring],
    now: (f32, f32),
    stretch: f32,
    /// Whether the pair moved while it sent the waves that are now in the picture.
    moved: bool,
}

impl Waves<'_> {
    /// The waves at the pixel (x, y): the height from -1 (trough) to 1 (crest), the
    /// amplitude, and the distance in pixels from where the pair was when it sent them. A
    /// pair that moved sent its earlier waves from where it was then, so the distance is
    /// then measured twice.
    #[inline]
    fn at(&self, x: f32, y: f32) -> (f32, f32, f32) {
        let from = if self.moved {
            let (dx, dy) = (x - self.now.0, (y - self.now.1) * self.stretch);
            let then = ring(self.table, (dx * dx + dy * dy).sqrt());
            (then.x, then.y)
        } else {
            self.now
        };
        let (dx, dy) = (x - from.0, (y - from.1) * self.stretch);
        let d2 = dx * dx + dy * dy;
        if d2 < 1e-6 {
            return (0.0, self.table[0].amp, 0.0);
        }
        let d = d2.sqrt();
        let ring = ring(self.table, d);
        // cos(2 * direction - phase), from the direction's cosine and sine of twice the angle.
        ((ring.cos * (dx * dx - dy * dy) + ring.sin * 2.0 * dx * dy) / d2, ring.amp, d)
    }
}

/// A black disk with an edge that fades over one pixel.
fn disk(c: &mut Canvas, x: f32, y: f32, r: f32) {
    if !(x.is_finite() && y.is_finite() && r > 0.0) {
        return;
    }
    let x0 = (x - r - 1.0).floor().max(0.0) as usize;
    let y0 = (y - r - 1.0).floor().max(0.0) as usize;
    let x1 = ((x + r + 2.0).max(0.0) as usize).min(c.w);
    let y1 = ((y + r + 2.0).max(0.0) as usize).min(c.h);
    for py in y0..y1 {
        for px in x0..x1 {
            let d = (px as f32 - x).hypot(py as f32 - y);
            let cover = (r - d + 0.5).clamp(0.0, 1.0);
            if cover > 0.0 {
                let p = &mut c.px[py * c.w + px];
                *p = pack(mix(unpack(*p), [0, 0, 0], cover));
            }
        }
    }
}

impl GravityWaves {
    /// The orbit turns one way or the other.
    fn dir(&self) -> f32 {
        if variation(self.controls.seed, 0x4452) < 0.5 { 1.0 } else { -1.0 }
    }

    /// The orbit is tilted away from the viewer, so circles on it look like ellipses. This is
    /// the height of such an ellipse as a share of its width.
    fn squash(&self) -> f32 {
        0.82 + 0.14 * variation(self.controls.seed, 0x5449)
    }

    /// The masses of the larger and the smaller hole of a pair, 2 in total.
    fn masses(&self) -> (f32, f32) {
        let q = 0.55 + 0.45 * variation(self.controls.seed, 0x4D41_0000 + self.cycle);
        (2.0 / (1.0 + q), 2.0 * q / (1.0 + q))
    }

    /// The separation of the pair now, and the two masses. After a merger there is one hole,
    /// and the second mass is 0.
    fn pair(&self) -> (f32, [f32; 2]) {
        let (m1, m2) = self.masses();
        let close = 1.0 - self.squeeze;
        match self.stage {
            Stage::Inspiral(p) => (WIDE * (1.0 - p).max(0.0).powf(0.25) * close, [m1, m2]),
            Stage::Ringdown(..) => (0.0, [REMNANT, 0.0]),
            Stage::Capture(t) => {
                let far = 1.0 - (t / CAPTURE).min(1.0);
                ((WIDE + (FAR - WIDE) * far * far) * close, [m1 + (REMNANT - m1) * far, m2])
            }
        }
    }

    /// The center of mass, as a share of the width and the height. Dragging moves it.
    fn center(&self) -> (f32, f32) {
        let pull = self.hand.force * 0.8;
        (0.5 + (self.hand.x - 0.5) * pull, 0.5 + (self.hand.y - 0.5) * pull)
    }

    /// Each hole's center and radius, in pixels.
    fn holes(&self, now: (f32, f32), unit: f32) -> Vec<(f32, f32, f32)> {
        let (sep, [ma, mb]) = self.pair();
        let total = ma + mb;
        let (sn, cs) = (self.angle.rem_euclid(std::f64::consts::TAU) as f32).sin_cos();
        let squash = self.squash();
        [(ma, -mb / total), (mb, ma / total)]
            .into_iter()
            .filter(|(m, _)| *m > 0.0)
            .map(|(m, side)| {
                let d = sep * side * unit;
                (now.0 + cs * d, now.1 + sn * d * squash, HORIZON * m * unit)
            })
            .collect()
    }

    /// Moves the pair on by `dt` and records what it sends out.
    fn step(&mut self, f: &Frame, dt: f32) {
        let music = &f.music;
        let loud = (f.peak * 7.0).clamp(0.0, 1.0);
        // Bass and beats pull the pair together. It drifts apart again between hits.
        let pull = (0.12 * music.bands[0] + 0.4 * music.pulse + 0.2 * (music.rel[0] - 1.0).clamp(0.0, 1.0))
            .min(SQUEEZE);
        if self.history.is_empty() {
            // The first frame starts in step with the music, so the waves that fill the
            // picture at once match it.
            self.energy = loud;
            self.squeeze = pull;
            self.stage = Stage::Inspiral(0.3 * variation(self.controls.seed, 0x5354));
        } else {
            follow(&mut self.energy, loud, 12.0, 3.0, dt);
            follow(&mut self.squeeze, pull, 14.0, 3.5, dt);
        }
        self.flash *= (-dt / FLASH).exp();
        self.t += dt as f64;
        self.stage = match self.stage {
            Stage::Inspiral(p) => Stage::Inspiral(p + dt * (0.35 + 0.65 * self.energy) / CYCLE),
            Stage::Ringdown(t, rate) if t + dt < RINGDOWN => Stage::Ringdown(t + dt, rate),
            Stage::Ringdown(..) => {
                self.cycle = self.cycle.wrapping_add(1);
                Stage::Capture(0.0)
            }
            Stage::Capture(t) if t + dt < CAPTURE => Stage::Capture(t + dt),
            Stage::Capture(_) => Stage::Inspiral(0.0),
        };
        let (sep, [ma, mb]) = self.pair();
        // Closer pairs send stronger waves, and unequal pairs weaker ones.
        let energy = self.energy;
        let strength = |sep: f32| energy * (WIDE / sep.max(1e-3)).min(3.5) * 4.0 * ma * mb / (ma + mb).powi(2);
        if matches!(self.stage, Stage::Inspiral(_)) && sep <= HORIZON * (ma + mb) * 1.1 {
            // The holes touch and merge. The new hole rings fast and briefly.
            self.ring = strength(sep) * 1.6;
            self.flash = 1.0;
            self.stage = Stage::Ringdown(0.0, kepler(sep) * 1.3);
        }
        let (rate, amp) = match self.stage {
            Stage::Ringdown(t, rate) => (rate, self.ring * (-t / RING_DECAY).exp()),
            _ => (kepler(sep), strength(sep)),
        };
        let rate = (self.dir() * rate) as f64;
        self.angle += rate * dt as f64;
        let (x, y) = self.center();
        let sample = Sample { t: self.t, phase: 2.0 * self.angle, rate: 2.0 * rate, amp, x, y };
        match self.history.back_mut() {
            // No time passed, so the new sample replaces the last one.
            Some(last) if last.t >= sample.t => *last = sample,
            _ => self.history.push_back(sample),
        }
    }

    /// Fills the table: at each distance from the source, the waves that the pair sent out
    /// `distance / SPEED` seconds ago. `unit` is the number of pixels in a shorter side.
    fn retard(&mut self, w: usize, h: usize, unit: f32) {
        let n = ((w as f32).hypot(h as f32) / self.squash()) as usize + 3;
        let per_px = 1.0 / (unit as f64 * SPEED as f64);
        // Keep enough history for the waves to cross the picture at the smallest scale.
        let keep = n as f64 * per_px * self.controls.scale as f64 / 0.5 + 1.0;
        while self.history.len() > 2 && self.history[1].t < self.t - keep {
            self.history.pop_front();
        }
        let (history, table) = (&self.history, &mut self.table);
        table.clear();
        let mut i = history.len() - 1;
        for r in 0..n {
            let t = self.t - r as f64 * per_px;
            while i > 0 && history[i].t > t {
                i -= 1;
            }
            let a = history[i];
            let (phase, amp, x, y) = match history.get(i + 1) {
                _ if t < a.t => (a.phase - a.rate * (a.t - t), a.amp, a.x, a.y),
                Some(b) => {
                    let k = (t - a.t) / (b.t - a.t);
                    let kf = k as f32;
                    (a.phase + (b.phase - a.phase) * k, a.amp + (b.amp - a.amp) * kf,
                        a.x + (b.x - a.x) * kf, a.y + (b.y - a.y) * kf)
                }
                None => (a.phase, a.amp, a.x, a.y),
            };
            let (sin, cos) = phase.sin_cos();
            table.push(Ring { cos: cos as f32, sin: sin as f32, amp, x: x * w as f32, y: y * h as f32 });
        }
    }
}

impl Mode for GravityWaves {
    fn name(&self) -> &'static str { "GRAVITY WAVES" }

    fn configure(&mut self, preset: &crate::preset::Preset) { self.controls = Controls::new(preset.seed); }

    fn interact(&mut self, controls: Controls) { self.controls = controls; }

    fn reset(&mut self, c: &mut Canvas) {
        c.fill(SCREEN);
        self.body.clear();
        self.trails.clear();
        self.history.clear();
        self.t = 0.0;
        self.angle = 0.0;
        self.cycle = 0;
        self.ring = 0.0;
        self.flash = 0.0;
        self.hand = Hand::default();
    }

    fn draw(&mut self, c: &mut Canvas, f: &Frame, pal: &Palette, dt: f32) {
        let dt = dt.clamp(0.0, 0.2);
        self.hand.update(&self.controls, dt);
        self.step(f, dt);
        let (w, h) = (c.w, c.h);
        let unit = w.min(h).max(1) as f32 * self.controls.scale;
        self.retard(w, h, unit);
        let (cx, cy) = self.center();
        let now = (cx * w as f32, cy * h as f32);
        let holes = self.holes(now, unit);
        let (sep, _) = self.pair();
        let biggest = holes.iter().fold(0.0f32, |r, hole| r.max(hole.2));
        let reach = (w as f32).hypot(h as f32) * 0.5;
        let waves = Waves {
            table: &self.table,
            now,
            stretch: 1.0 / self.squash(),
            moved: self.table.iter().any(|r| (r.x - now.0).abs() > 0.01 || (r.y - now.1).abs() > 0.01),
        };

        // The waves as they leave the pair now. Crests run from the skin's middle color near
        // the pair to its hot color at the edge; troughs keep a deep glow. Light that passes
        // near a hole bends around it.
        let lenses: Vec<_> = holes.iter().map(|&(x, y, r)| (x, y, (LENS * r).powi(2))).collect();
        let inner = 1.0 / (sep * unit * 0.8).max(biggest * 2.0).max(1.0);
        let (fade, far) = (0.6 / unit, 1.0 / reach);
        let haze = self.energy * 0.45 + self.flash * 3.0;
        let (haze_u, haze_r) = (0.12 + 0.5 * self.flash, 1.0 / (0.16 * unit).powi(2));
        self.body.fit(w, h);
        self.body.paint(|x, y| {
            let (mut px, mut py) = (x as f32, y as f32);
            for &(hx, hy, e2) in &lenses {
                let (dx, dy) = (x as f32 - hx, y as f32 - hy);
                let k = e2 / (dx * dx + dy * dy + 1.0);
                px -= dx * k;
                py -= dy * k;
            }
            let (height, amp, d) = waves.at(px, py);
            let near = (d * inner).min(1.0);
            let crest = 0.5 + 0.5 * height;
            let band = crest * crest * crest;
            let e = amp * near * near * (3.0 - 2.0 * near) * (0.22 + 1.7 * band) / (1.0 + d * fade);
            let (dx, dy) = (px - now.0, (py - now.1) * waves.stretch);
            let glow = haze / (1.0 + (dx * dx + dy * dy) * haze_r);
            let u = 0.18 + band * (0.32 + 0.55 * (d * far).min(1.0).sqrt());
            let total = e + glow;
            (total, if total > 1e-6 { (e * u + glow * haze_u) / total } else { 0.0 })
        });

        // The trails drift outward more slowly than the waves, so the waves smear behind
        // their motion. Mids bend the smear. The trails never turn: a turn would bring dark
        // light in from past the corners.
        let keep = per_frame(0.86, dt);
        self.trails.fit(w, h);
        self.trails.warp_flow(now, now, per_frame(1.003, dt), 0.0, keep, true,
            Flow { time: self.t as f32, amount: 0.006 + 0.03 * f.music.slow[1], dt });
        self.trails.add_scaled(&self.body, 1.0 - keep);
        self.trails.resolve(c, pal);

        // Threads of space ripple sideways as the waves pass, and small crosses stretch along
        // one diagonal and squeeze along the other. Treble makes them glint.
        let seed = self.controls.seed;
        let ink = pal.skin.peak;
        let show = self.energy.min(1.0);
        let treble = f.music.bands[2];
        let shift = 0.012 * unit;
        let middle = (w as f32 * 0.5, h as f32 * 0.5);
        let alpha = (0.1 + 0.16 * f.music.slow[2]) * show;
        if alpha > 0.004 {
            let step = (unit * 0.02).max(3.0);
            let count = (2.0 * reach / step).ceil() as usize;
            for i in 0..THREADS {
                let (sn, cs) = (variation(seed, 0x4C00 + i) * PI).sin_cos();
                let off = (variation(seed, 0x4C40 + i) - 0.5) * 1.5 * reach;
                let base = (middle.0 - sn * off, middle.1 + cs * off);
                let pts: Vec<(f32, f32)> = (0..=count)
                    .map(|k| {
                        let s = k as f32 * step - reach;
                        let (x, y) = (base.0 + cs * s, base.1 + sn * s);
                        let (height, amp, _) = waves.at(x, y);
                        let d = height * amp.min(2.5) * shift;
                        (x - sn * d, y + cs * d)
                    })
                    .collect();
                c.polyline(&pts, ink, alpha, 1, false);
            }
        }
        let t = self.t as f32;
        for i in 0..MARKERS {
            let (x, y) = (variation(seed, 0x5800 + i) * w as f32, variation(seed, 0x5840 + i) * h as f32);
            let size = unit * (0.005 + 0.012 * variation(seed, 0x5880 + i).powi(2));
            let (height, amp, _) = waves.at(x, y);
            let k = 0.45 * height * amp.min(2.0);
            let (a, b) = (size * (1.0 + k) * FRAC_1_SQRT_2, size * (1.0 - k) * FRAC_1_SQRT_2);
            let twinkle = 0.5 + 0.5 * (t * (1.3 + 2.7 * variation(seed, 0x58C0 + i)) + variation(seed, 0x5900 + i) * TAU).sin();
            let alpha = (0.2 + 0.35 * variation(seed, 0x5940 + i)) * (1.0 + treble * (1.4 * twinkle - 0.6)) * show;
            c.line(x - a, y - a, x + a, y + a, ink, alpha, 1, false);
            c.line(x - b, y + b, x + b, y - b, ink, alpha, 1, false);
        }
        for &(x, y, r) in &holes {
            disk(c, x, y, r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analyzer;

    fn music(peak: f32, bass: f32, pulse: f32) -> Frame {
        let mut a = Analyzer::new();
        a.frame.peak = peak;
        a.frame.music.bands = [bass, 0.4, 0.3];
        a.frame.music.slow = [bass, 0.4, 0.3];
        a.frame.music.pulse = pulse;
        a.frame
    }

    fn light(c: &Canvas) -> u64 {
        c.px.iter().map(|&v| unpack(v).iter().map(|&v| u64::from(v)).sum::<u64>()).sum()
    }

    #[test]
    fn silence_is_dark_and_music_fills_the_picture() {
        let mut mode = GravityWaves::default();
        let mut c = Canvas::new(200, 120);
        mode.draw(&mut c, &music(0.0, 0.0, 0.0), &Palette::new(3), 1.0 / 60.0);
        assert!(c.px.iter().all(|&v| unpack(v).iter().zip(SCREEN).all(|(a, b)| *a <= b)), "Silence starts dark");

        mode.reset(&mut c);
        let loud = music(0.3, 0.5, 0.0);
        for _ in 0..90 {
            mode.draw(&mut c, &loud, &Palette::new(3), 1.0 / 60.0);
        }
        for gy in 0..3 {
            for gx in 0..4 {
                let lit = (gy * 40..gy * 40 + 40).flat_map(|y| (gx * 50..gx * 50 + 50).map(move |x| y * 200 + x))
                    .filter(|&i| unpack(c.px[i]).iter().any(|&v| v > 40)).count();
                assert!(lit > 200, "Region {gx}, {gy} is dark");
            }
        }
        let active = light(&c);
        let silent = music(0.0, 0.0, 0.0);
        for _ in 0..600 {
            mode.draw(&mut c, &silent, &Palette::new(3), 1.0 / 60.0);
        }
        assert!(light(&c) < active / 4, "The waves fade and leave the picture in silence");
        for (w, h) in [(90, 120), (240, 100), (1, 1)] {
            c.resize(w, h);
            mode.draw(&mut c, &loud, &Palette::new(3), 1.0 / 30.0);
        }
    }

    #[test]
    fn bass_pulls_the_pair_together_and_winds_the_waves_tighter() {
        let settle = |bass: f32, pulse: f32| {
            let mut mode = GravityWaves::default();
            let mut c = Canvas::new(160, 90);
            for _ in 0..30 {
                mode.draw(&mut c, &music(0.3, bass, pulse), &Palette::new(0), 1.0 / 60.0);
            }
            let rate = mode.history.back().unwrap().rate.abs();
            (mode.pair().0, rate, mode.history.back().unwrap().amp)
        };
        let (wide, slow, weak) = settle(0.0, 0.0);
        let (close, fast, strong) = settle(0.9, 1.0);
        assert!(close < wide * 0.6, "Bass closes the pair: {close} against {wide}");
        assert!(fast > slow * 2.0, "A closer pair orbits faster: {fast} against {slow}");
        assert!(strong > weak * 1.5, "A closer pair sends stronger waves: {strong} against {weak}");
    }

    #[test]
    fn waves_carry_changes_outward_at_their_speed() {
        let mut mode = GravityWaves::default();
        let mut c = Canvas::new(200, 120);
        mode.draw(&mut c, &music(0.0, 0.0, 0.0), &Palette::new(1), 1.0 / 60.0);
        let loud = music(0.3, 0.3, 0.0);
        for _ in 0..30 {
            mode.draw(&mut c, &loud, &Palette::new(1), 1.0 / 60.0);
        }
        // Half a second at 0.2 shorter sides a second: the first 12 pixels of 120.
        assert!(mode.table[6].amp > 0.1, "Near the pair the new waves have arrived");
        assert_eq!(mode.table[20].amp, 0.0, "Farther out the waves from the silence remain");
        for _ in 0..240 {
            mode.draw(&mut c, &loud, &Palette::new(1), 1.0 / 60.0);
        }
        assert!(mode.table[100].amp > 0.1, "Four seconds later they have spread out");
    }

    #[test]
    fn the_pair_merges_on_a_beat_and_a_new_partner_falls_in() {
        let mut mode = GravityWaves::default();
        let mut c = Canvas::new(64, 40);
        let mut stages = Vec::new();
        for k in 0..30 * 60 {
            // A beat every half second.
            let pulse = (-((k % 15) as f32) / 30.0 / 0.12).exp();
            mode.draw(&mut c, &music(0.3, 0.5, pulse), &Palette::new(2), 1.0 / 30.0);
            let stage = std::mem::discriminant(&mode.stage);
            if stages.last().map(|s: &(_, f32)| s.0) != Some(stage) {
                stages.push((stage, pulse));
            }
        }
        let [inspiral, ringdown, capture] =
            [Stage::Inspiral(0.0), Stage::Ringdown(0.0, 0.0), Stage::Capture(0.0)].map(|s| std::mem::discriminant(&s));
        let order: Vec<_> = stages.iter().map(|s| s.0).collect();
        assert!(order.windows(4).any(|w| w == [inspiral, ringdown, capture, inspiral]), "{stages:?}");
        assert!(stages.iter().filter(|s| s.0 == ringdown).all(|s| s.1 > 0.2), "Mergers fall on beats: {stages:?}");
        assert!(mode.cycle >= 1);
        assert!(mode.history.len() < 30 * 60, "Old history is dropped");
    }
}
