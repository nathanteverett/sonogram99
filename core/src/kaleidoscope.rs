//! Two mirror kaleidoscopes over the same tumbling glass. Every pixel of the image folds back
//! through the mirrors into one cell of glass. The glass is random and never repeats.
//!
//! Kaleidoscope is a tube of three or four mirrors. Its reflections tile the whole picture,
//! drift across it, and change to a new tube every few bars. Mandala has two mirrors, so its
//! reflections make one rosette that stays fixed at the center and fills the picture.

use std::f32::consts::{FRAC_1_SQRT_2, PI, TAU};

use crate::analysis::Frame;
use crate::canvas::Canvas;
use crate::glow::{per_frame, Field, Flow};
use crate::interaction::{variation, Controls, Hand};
use crate::modes::Mode;
use crate::palette::{Palette, SCREEN};

type Point = (f32, f32);

/// Side of the square object cell that holds the glass, in cell pixels.
const CELL: usize = 224;
const MID: f32 = CELL as f32 * 0.5;
/// The glass bounces off a round wall inside the cell.
const WALL: f32 = CELL as f32 * 0.47;
/// Circumradius of the mirror window in the cell. It turns inside the wall.
const WINDOW: f32 = WALL * 0.94;
/// Seconds for one mirror tube to dissolve into the next.
const CHANGE: f32 = 2.2;

/// How the glass of one kaleidoscope looks and moves.
#[derive(Clone, Copy)]
struct Style {
    /// The share of its light the cell keeps each frame at 60 frames per second. Less light
    /// kept gives shorter trails and sharper glass.
    keep: f32,
    /// How much the light in the cell wobbles as it moves.
    flow: f32,
    /// How fast the glass drifts back toward the middle of the cell: the share of its distance
    /// from the middle it covers each second. Beats throw it out again.
    gather: f32,
    /// Brush radius of edges, rods and rings, against the standard brush.
    line: f32,
    /// How many shards and washes the cell holds, against the standard count.
    bulk: f32,
    /// The cell also holds tiny glass, and beads have hard edges.
    fine: bool,
    /// Upper limit on pieces of glass, sparks included.
    limit: usize,
}

impl Style {
    const KALEIDOSCOPE: Style = Style { keep: 0.86, flow: 1.0, gather: 0.0, line: 1.0, bulk: 1.0, fine: false, limit: 180 };
    /// The middle of the cell is the middle of the rosette, so the glass drifts back there
    /// between beats. Short trails, little wobble and thin lines keep the rosette sharp, and
    /// fewer large pieces leave room for the tiny glass.
    const MANDALA: Style = Style { keep: 0.7, flow: 0.25, gather: 0.25, line: 0.45, bulk: 0.6, fine: true, limit: 700 };
}

/// SplitMix64. It is small, fast, and the same on every platform, so one seed and the same
/// audio always give the same glass.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// 0 to 1, never 1.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / 16_777_216.0
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn sign(&mut self) -> f32 {
        if self.unit() < 0.5 { -1.0 } else { 1.0 }
    }
}

/// A triangle wave with period 2: the coordinate reflected between mirrors at each whole number.
#[inline]
fn bounce(t: f32) -> f32 {
    (t - 2.0 * ((t + 1.0) * 0.5).floor()).abs()
}

fn follow(value: &mut f32, target: f32, attack: f32, release: f32, dt: f32) {
    let rate = if target > *value { attack } else { release };
    *value += (target - *value) * (1.0 - (-rate * dt).exp());
}

/// The weight of a point on each of the three corner classes of an equilateral triangle grid
/// with side 1. Corners of one class never share an edge, so the weights of two neighboring
/// triangles are mirror images of each other.
#[inline]
fn triangle_weights(x: f32, y: f32) -> [f32; 3] {
    let b = y * (2.0 / 3.0f32.sqrt());
    let a = x - b * 0.5;
    let (i, j) = (a.floor(), b.floor());
    let (fa, fb) = (a - i, b - j);
    let n = ((i - j) as i32).rem_euclid(3) as usize;
    let w = if fa + fb < 1.0 { [1.0 - fa - fb, fa, fb] } else { [fa + fb - 1.0, 1.0 - fb, 1.0 - fa] };
    let mut out = [0.0; 3];
    for (k, w) in w.into_iter().enumerate() {
        out[(n + k) % 3] = w;
    }
    out
}

/// The real mirror tubes whose reflections tile the whole plane.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mirrors {
    /// Three mirrors at 60 degrees: six triangles meet at each corner.
    Equilateral,
    /// Three mirrors at 30, 60 and 90 degrees: twelve-point stars.
    HalfEquilateral,
    /// Three mirrors at 45, 45 and 90 degrees: eight-point stars on a square grid.
    RightIsosceles,
    /// Four mirrors in a square tube.
    Square,
}

const TUBES: [Mirrors; 4] = [Mirrors::Equilateral, Mirrors::HalfEquilateral, Mirrors::RightIsosceles, Mirrors::Square];

impl Mirrors {
    /// Circumradius of one mirror cell when the tiling's unit is 1.
    fn reach(self) -> f32 {
        match self {
            Mirrors::Equilateral => 1.0 / 3.0f32.sqrt(),
            Mirrors::HalfEquilateral => 0.5 / 3.0f32.sqrt(),
            Mirrors::RightIsosceles | Mirrors::Square => FRAC_1_SQRT_2,
        }
    }

    /// The corners of the mirror window in the cell, turned by `turn` about the cell's center.
    /// The window has the same shape as a mirror cell on the screen, so the glass is not stretched.
    fn window(self, turn: f32) -> [Point; 4] {
        let r = WINDOW;
        let corners = match self {
            Mirrors::Equilateral => {
                let at = |k: f32| ((k * TAU / 3.0).cos() * r, (k * TAU / 3.0).sin() * r);
                [at(0.0), at(1.0), at(2.0), (0.0, 0.0)]
            }
            // The 30, 90 and 60 degree corners, on the circle around the hypotenuse.
            Mirrors::HalfEquilateral => [(-r, 0.0), (0.5 * r, 0.75f32.sqrt() * r), (r, 0.0), (0.0, 0.0)],
            // The 45, 90 and 45 degree corners.
            Mirrors::RightIsosceles => [(-r, 0.0), (0.0, r), (r, 0.0), (0.0, 0.0)],
            Mirrors::Square => {
                let s = r * FRAC_1_SQRT_2;
                [(-s, -s), (s, -s), (s, s), (-s, s)]
            }
        };
        let (sn, cs) = turn.sin_cos();
        corners.map(|(x, y)| (MID + x * cs - y * sn, MID + x * sn + y * cs))
    }

    /// Follows the reflections from a point on the tiled plane back to the window in the cell.
    #[inline]
    fn fold(self, x: f32, y: f32, v: &[Point; 4]) -> Point {
        let corners = |w: [f32; 3]| {
            (w[0] * v[0].0 + w[1] * v[1].0 + w[2] * v[2].0, w[0] * v[0].1 + w[1] * v[1].1 + w[2] * v[2].1)
        };
        match self {
            Mirrors::Equilateral => corners(triangle_weights(x, y)),
            Mirrors::HalfEquilateral => {
                // Each equilateral triangle holds six mirror cells. Sorting the weights finds the
                // cell, and these weights place the point in it.
                let [mut a, mut b, mut c] = triangle_weights(x, y);
                if a < b { std::mem::swap(&mut a, &mut b); }
                if b < c { std::mem::swap(&mut b, &mut c); }
                if a < b { std::mem::swap(&mut a, &mut b); }
                corners([a - b, 2.0 * (b - c), 3.0 * c])
            }
            Mirrors::RightIsosceles => {
                let (a, b) = (bounce(x), bounce(y));
                let (a, b) = if b > a { (b, a) } else { (a, b) };
                corners([1.0 - a, a - b, b])
            }
            Mirrors::Square => {
                let (a, b) = (bounce(x), bounce(y));
                (v[0].0 + a * (v[1].0 - v[0].0) + b * (v[3].0 - v[0].0),
                 v[0].1 + a * (v[1].1 - v[0].1) + b * (v[3].1 - v[0].1))
            }
        }
    }
}

/// One arrangement of the mirror tube and how it moves across the screen.
#[derive(Clone)]
struct Pattern {
    mirrors: Mirrors,
    /// Circumradius of one mirror cell, as a fraction of the shorter image side.
    size: f32,
    angle: f32,
    /// Radians per second.
    spin: f32,
    /// Position on the tiled plane, in tiling units.
    pan: Point,
    /// Tiling units per second.
    drift: Point,
    /// Extra turn of the window, so a new tube shows different glass.
    offset: f32,
}

impl Pattern {
    fn random(rng: &mut Rng, after: Option<Mirrors>) -> Self {
        let mut i = rng.below(TUBES.len());
        if after == Some(TUBES[i]) && rng.unit() < 0.8 {
            i = (i + 1 + rng.below(TUBES.len() - 1)) % TUBES.len();
        }
        let heading = rng.range(0.0, TAU);
        let speed = rng.range(0.02, 0.09);
        Pattern {
            mirrors: TUBES[i],
            size: rng.range(0.11, 0.27),
            angle: rng.range(0.0, TAU),
            spin: rng.sign() * rng.range(0.015, 0.08),
            pan: (rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)),
            drift: (heading.cos() * speed, heading.sin() * speed),
            offset: rng.range(0.0, TAU),
        }
    }

    fn advance(&mut self, dt: f32) {
        self.angle += self.spin * dt;
        self.pan.0 += self.drift.0 * dt;
        self.pan.1 += self.drift.1 * dt;
    }
}

/// A pattern fixed for one frame: screen pixels in, cell light out.
struct Lens {
    mirrors: Mirrors,
    cos: f32,
    sin: f32,
    /// Tiling units per screen pixel.
    inv: f32,
    pan: Point,
    window: [Point; 4],
}

impl Lens {
    fn new(p: &Pattern, side: f32, twist: f32, turn: f32) -> Self {
        let tile = (side * p.size / p.mirrors.reach()).max(1e-3);
        let (sin, cos) = (p.angle + twist).sin_cos();
        Lens { mirrors: p.mirrors, cos, sin, inv: 1.0 / tile, pan: p.pan, window: p.mirrors.window(turn + p.offset) }
    }

    /// `x` and `y` are measured from the center of the image.
    #[inline]
    fn look(&self, cell: &Field, x: f32, y: f32) -> (f32, f32) {
        let qx = (self.cos * x - self.sin * y) * self.inv + self.pan.0;
        let qy = (self.sin * x + self.cos * y) * self.inv + self.pan.1;
        let (u, v) = self.mirrors.fold(qx, qy, &self.window);
        cell.sample(u, v)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// A flat piece of colored glass with bright edges.
    Shard,
    Bead,
    Rod,
    Ring,
    /// A broad, faint cloud of color behind the glass.
    Wash,
    /// A short flash on the treble.
    Spark,
    /// A tiny hard dot.
    Grain,
    /// A small shard.
    Flake,
    /// A tiny star of thin spokes.
    Star,
    /// A tiny ring of dots around a dot.
    Lace,
}

impl Kind {
    /// Tiny glass, which only a fine style holds.
    fn fine(self) -> bool {
        matches!(self, Kind::Grain | Kind::Flake | Kind::Star | Kind::Lace)
    }
}

/// How fast tiny glass returns to its own distance from the middle of the cell: the share of
/// the gap it closes each second.
const HOLD: f32 = 1.2;

/// The kinds the recipe counts. Sparks come from the treble instead.
const KINDS: [Kind; 9] = [Kind::Shard, Kind::Bead, Kind::Rod, Kind::Ring, Kind::Wash,
                          Kind::Grain, Kind::Flake, Kind::Star, Kind::Lace];

struct Chip {
    kind: Kind,
    pos: Point,
    vel: Point,
    angle: f32,
    spin: f32,
    size: f32,
    /// Brush radius of rods.
    thick: f32,
    hue: f32,
    phase: f32,
    /// A shard's corners as (angle, fraction of size).
    corners: [Point; 4],
    /// A shard's corners, a star's spokes, or the dots of lace.
    sides: usize,
    age: f32,
    life: f32,
    /// The distance from the middle of the cell that tiny glass returns to. Other glass
    /// returns to the middle.
    home: f32,
}

impl Chip {
    /// `scale` is the size of the cell against the standard `CELL`.
    fn new(rng: &mut Rng, kind: Kind, mood: f32, burst: bool, scale: f32) -> Self {
        let (mid, wall) = (MID * scale, WALL * scale);
        let (dist, heading) = (if burst { rng.range(0.0, wall * 0.15) } else { wall * 0.9 * rng.unit().powf(0.65) }, rng.range(0.0, TAU));
        let pos = (mid + heading.cos() * dist, mid + heading.sin() * dist);
        let push = scale * if burst { rng.range(40.0, 110.0) } else { rng.range(0.0, 12.0) };
        let aim = if burst { heading } else { rng.range(0.0, TAU) };
        let (size, life) = match kind {
            Kind::Shard => (rng.range(9.0, 27.0), rng.range(5.0, 14.0)),
            Kind::Bead => (rng.range(2.5, 6.5), rng.range(4.0, 11.0)),
            Kind::Rod => (rng.range(8.0, 22.0), rng.range(5.0, 12.0)),
            Kind::Ring => (rng.range(4.0, 10.0), rng.range(4.0, 10.0)),
            Kind::Wash => (rng.range(30.0, 70.0), rng.range(6.0, 15.0)),
            Kind::Spark => (rng.range(1.0, 2.2), rng.range(0.2, 0.6)),
            Kind::Grain => (rng.range(0.35, 0.8), rng.range(3.0, 9.0)),
            Kind::Flake => (rng.range(2.5, 7.0), rng.range(4.0, 10.0)),
            Kind::Star => (rng.range(1.6, 4.0), rng.range(4.0, 10.0)),
            Kind::Lace => (rng.range(1.8, 4.5), rng.range(5.0, 12.0)),
        };
        let size = size * scale;
        let hue = match kind {
            Kind::Spark => rng.range(0.6, 1.0),
            _ if rng.unit() < 0.2 => rng.unit(),
            _ => bounce(mood + rng.range(-0.3, 0.3)),
        };
        let sides = match kind {
            Kind::Star | Kind::Lace => 5 + rng.below(4),
            _ => 3 + rng.below(2),
        };
        let mut corners = [(0.0, 0.0); 4];
        for (k, c) in corners.iter_mut().enumerate().take(sides) {
            let step = TAU / sides as f32;
            *c = (k as f32 * step + rng.range(-0.35, 0.35) * step, rng.range(0.55, 1.0));
        }
        Chip {
            kind, pos, vel: (aim.cos() * push, aim.sin() * push),
            angle: rng.range(0.0, TAU), spin: rng.range(-1.2, 1.2),
            size, thick: rng.range(1.0, 2.2) * scale, hue, phase: rng.range(0.0, TAU),
            corners, sides, age: 0.0, life,
            // Burst grains fly out from the middle to a distance of their own.
            home: match (kind.fine(), burst) {
                (false, _) => 0.0,
                (true, false) => dist,
                (true, true) => wall * 0.9 * rng.unit().powf(0.65),
            },
        }
    }

    /// Fades in and out, so glass never pops into or out of view.
    fn presence(&self) -> f32 {
        let rise = (self.age / (self.life * 0.2).min(0.5)).min(1.0);
        let fall = ((self.life - self.age) / (self.life * 0.4).min(0.9)).clamp(0.0, 1.0);
        rise * fall
    }

    fn draw(&self, cell: &mut Field, light: f32, s: &Scene) {
        let a = light * self.presence();
        if a <= 0.0 {
            return;
        }
        let (x, y) = self.pos;
        let brush = s.scale * s.style.line;
        match self.kind {
            Kind::Shard | Kind::Flake => {
                // Bass swells the glass; treble makes its edges glint.
                let grow = self.size * (1.0 + 0.22 * s.bass + s.kick);
                let corner = |k: usize| {
                    let (turn, reach) = self.corners[k];
                    let t = self.angle + turn;
                    (x + t.cos() * grow * reach, y + t.sin() * grow * reach)
                };
                for k in 1..self.sides - 1 {
                    cell.triangle([corner(0), corner(k), corner(k + 1)], self.hue, a * 0.7);
                }
                let glint = 0.7 + s.treble * 1.8 * (0.5 + 0.5 * (s.t * 7.0 + self.phase).sin());
                let edge = bounce(self.hue + 0.08);
                for k in 0..self.sides {
                    let (p, q) = (corner(k), corner((k + 1) % self.sides));
                    cell.line(p.0, p.1, q.0, q.1, brush, edge, edge, a * glint);
                }
            }
            Kind::Bead => {
                let r = self.size * (1.0 + 0.2 * s.bass);
                if s.style.fine {
                    cell.disc(x, y, r * 0.5, self.hue, a * 1.3);
                } else {
                    cell.dab(x, y, r, self.hue, a * 1.6);
                }
            }
            Kind::Rod => {
                let (sn, cs) = self.angle.sin_cos();
                let l = self.size;
                let thick = self.thick * s.style.line;
                cell.line(x - cs * l, y - sn * l, x + cs * l, y + sn * l, thick, self.hue, bounce(self.hue + 0.12), a * 1.2);
            }
            Kind::Ring => {
                let pts: Vec<_> = (0..=20).map(|k| {
                    let t = k as f32 / 20.0 * TAU;
                    let r = self.size * (1.0 + 0.25 * s.treble * (t * 3.0 + s.t * 5.0 + self.phase).sin());
                    (x + t.cos() * r, y + t.sin() * r, self.hue)
                }).collect();
                cell.polyline(&pts, brush, a);
            }
            Kind::Wash => cell.dab(x, y, self.size, self.hue, a * 0.32),
            Kind::Spark => cell.dab(x, y, self.size, self.hue, a * 3.0),
            Kind::Grain => {
                let glint = 1.0 + s.treble * 1.5 * (0.5 + 0.5 * (s.t * 9.0 + self.phase).sin());
                cell.disc(x, y, self.size, self.hue, a * 2.0 * glint);
            }
            Kind::Star => {
                // An even star alternates long and short spokes.
                let tip = bounce(self.hue + 0.1);
                let grow = self.size * (1.0 + 0.3 * s.treble);
                for k in 0..self.sides {
                    let t = self.angle + k as f32 * TAU / self.sides as f32;
                    let l = if self.sides.is_multiple_of(2) && k % 2 == 1 { grow * 0.55 } else { grow };
                    cell.line(x, y, x + t.cos() * l, y + t.sin() * l, brush, self.hue, tip, a * 1.4);
                }
            }
            Kind::Lace => {
                let r = self.size * (1.0 + 0.25 * s.treble * (s.t * 5.0 + self.phase).sin());
                let (dot, rim) = (0.4 * s.scale, bounce(self.hue + 0.1));
                for k in 0..self.sides {
                    let t = self.angle + k as f32 * TAU / self.sides as f32;
                    cell.disc(x + t.cos() * r, y + t.sin() * r, dot, rim, a * 1.8);
                }
                cell.disc(x, y, dot * 1.3, self.hue, a * 1.8);
            }
        }
    }
}

/// Everything that changes as the kaleidoscope plays. A new seed builds a new scene.
struct Scene {
    rng: Rng,
    style: Style,
    /// Size of the cell against the standard `CELL`. Positions, sizes and speeds of the glass
    /// all scale with it, so a larger cell holds the same glass in finer detail.
    scale: f32,
    t: f32,
    energy: f32,
    bass: f32,
    mids: f32,
    treble: f32,
    /// How far the tube has turned against the glass.
    turn: f32,
    turn_dir: f32,
    /// Speed of the glass around the cell, in radians per second.
    swirl: f32,
    swirl_dir: f32,
    /// Wobble of the image, as a fraction of the shorter image side.
    ripple: f32,
    /// Extra size from the last beat. It decays between beats.
    kick: f32,
    beat: bool,
    /// Center of the colors that new glass takes from the skin's gradient.
    mood: f32,
    sparks: f32,
    /// How many pieces of each kind in `KINDS` the cell holds.
    recipe: [usize; 9],
    glass: Vec<Chip>,
    pattern: Pattern,
    /// The tube that is dissolving, and how far it has gone (0 to 1).
    previous: Option<(Pattern, f32)>,
    until_change: f32,
}

fn recipe(rng: &mut Rng, style: &Style) -> [usize; 9] {
    let mut out = [(rng.range(22.0, 40.0) * style.bulk) as usize, rng.range(4.0, 18.0) as usize,
                   rng.range(0.0, 10.0) as usize, rng.range(0.0, 6.0) as usize,
                   (rng.range(5.0, 10.0) * style.bulk) as usize, 0, 0, 0, 0];
    // The mirrors show only one wedge of the cell, so it needs hundreds of tiny pieces for a
    // few dozen to show at a time.
    if style.fine {
        for (n, (lo, hi)) in out[5..].iter_mut().zip([(120.0, 260.0), (30.0, 70.0), (30.0, 70.0), (25.0, 50.0)]) {
            *n = rng.range(lo, hi) as usize;
        }
    }
    out
}

impl Scene {
    fn new(seed: u32, scale: f32, style: Style) -> Self {
        let mut rng = Rng(u64::from(seed).wrapping_mul(0xD1B5_4A32_D192_ED03) ^ 0x4B41_4C45);
        let pattern = Pattern::random(&mut rng, None);
        let mood = rng.unit();
        let recipe = recipe(&mut rng, &style);
        // Start with a full cell, at mixed ages, so the first beat already shows glass.
        let mut glass = Vec::with_capacity(style.limit);
        for (kind, &count) in KINDS.iter().zip(&recipe) {
            for _ in 0..count {
                let mut chip = Chip::new(&mut rng, *kind, mood, false, scale);
                chip.age = rng.range(0.5, chip.life * 0.5);
                glass.push(chip);
            }
        }
        Scene {
            style, scale, t: 0.0, energy: 0.0, bass: 0.0, mids: 0.0, treble: 0.0,
            turn: rng.range(0.0, TAU), turn_dir: rng.sign(), swirl: 0.0, swirl_dir: rng.sign(),
            ripple: 0.0, kick: 0.0, beat: false, mood, sparks: 0.0, recipe, glass, pattern,
            previous: None, until_change: rng.range(8.0, 18.0), rng,
        }
    }

    fn update(&mut self, f: &Frame, dt: f32, hand: &Hand) {
        let music = &f.music;
        let [bass, mids, treble] = music.bands;
        (self.bass, self.mids, self.treble) = (bass, mids, treble);
        follow(&mut self.energy, (f.peak * 7.0).clamp(0.0, 1.0), 12.0, 3.0, dt);
        self.t += dt;
        // Mids turn the tube and stir the glass. The slow envelope keeps the turning smooth.
        self.turn += dt * self.turn_dir * (0.05 + 0.32 * music.slow[1] + 0.1 * bass);
        self.swirl = self.swirl_dir * (0.1 + 0.7 * mids + 1.2 * hand.force);
        self.ripple = 0.002 + 0.012 * mids + 0.004 * treble;
        self.kick *= (-5.0 * dt).exp();
        let hit = music.onset > 0.45 && !self.beat;
        if hit {
            self.beat = true;
            self.strike(music.onset);
        } else if music.onset < 0.2 {
            self.beat = false;
        }

        let pace = 0.4 + 0.9 * music.slow[1] + 0.4 * bass + 1.5 * music.width;
        self.pattern.advance(dt * pace);
        if let Some((old, fade)) = &mut self.previous {
            old.advance(dt * pace);
            *fade += dt / CHANGE;
            if *fade >= 1.0 {
                self.previous = None;
            }
        }
        // Loud music changes the tube sooner, and a beat near the end of a pattern triggers it.
        self.until_change -= dt * (0.4 + self.energy);
        if self.previous.is_none() && (self.until_change <= 0.0 || (hit && self.until_change < 3.0)) {
            self.change();
        }
        self.mood = bounce(self.mood + dt * 0.012 * (self.t * 0.07 + self.turn_dir).sin());
        self.tend_glass(dt);
    }

    /// A beat shakes the cell, throws new glass out from the middle, and swells the image.
    fn strike(&mut self, onset: f32) {
        self.kick += 0.04 + 0.06 * onset;
        let (rng, mid, scale) = (&mut self.rng, MID * self.scale, self.scale);
        for chip in &mut self.glass {
            let (rx, ry) = (chip.pos.0 - mid, chip.pos.1 - mid);
            let r = rx.hypot(ry).max(1.0);
            let aim = rng.range(0.0, TAU);
            // Tiny glass only shivers, so the fine pattern holds together.
            let shake = if chip.kind.fine() { 0.35 } else { 1.0 };
            let push = (30.0 + 70.0 * onset) * rng.range(0.4, 1.0) * scale * shake;
            chip.vel.0 += push * (rx / r * 0.6 + aim.cos() * 0.5);
            chip.vel.1 += push * (ry / r * 0.6 + aim.sin() * 0.5);
        }
        for _ in 0..1 + self.rng.below(3) {
            if self.glass.len() < self.style.limit {
                let chip = Chip::new(&mut self.rng, Kind::Shard, self.mood, true, self.scale);
                self.glass.push(chip);
            }
        }
        // In the mirrors, a burst of grains makes a ring of dots that spreads from the middle.
        if self.style.fine {
            for _ in 0..6 + self.rng.below(10) {
                if self.glass.len() < self.style.limit {
                    let chip = Chip::new(&mut self.rng, Kind::Grain, self.mood, true, self.scale);
                    self.glass.push(chip);
                }
            }
        }
        self.sparks += 6.0 * onset;
    }

    fn change(&mut self) {
        let next = Pattern::random(&mut self.rng, Some(self.pattern.mirrors));
        self.previous = Some((std::mem::replace(&mut self.pattern, next), 0.0));
        self.until_change = self.rng.range(9.0, 22.0);
        self.recipe = recipe(&mut self.rng, &self.style);
        self.mood = bounce(self.mood + self.rng.range(-0.45, 0.45));
        if self.rng.unit() < 0.35 { self.turn_dir = -self.turn_dir; }
        if self.rng.unit() < 0.35 { self.swirl_dir = -self.swirl_dir; }
    }

    fn tend_glass(&mut self, dt: f32) {
        self.glass.retain(|c| c.age < c.life);
        let mut counts = [0; 9];
        for chip in &self.glass {
            if let Some(k) = KINDS.iter().position(|&k| k == chip.kind) { counts[k] += 1; }
        }
        for (k, kind) in KINDS.iter().enumerate() {
            let missing = self.recipe[k].saturating_sub(counts[k]);
            if kind.fine() {
                // Tiny glass is plentiful, so it comes back in proportion to what is missing.
                if missing > 0 {
                    let due = missing as f32 * dt * 1.5;
                    for _ in 0..due as usize + usize::from(self.rng.unit() < due.fract()) {
                        if self.glass.len() < self.style.limit {
                            let chip = Chip::new(&mut self.rng, *kind, self.mood, false, self.scale);
                            self.glass.push(chip);
                        }
                    }
                }
            } else if missing > 0 && self.glass.len() < self.style.limit && self.rng.unit() < dt * 4.0 {
                let chip = Chip::new(&mut self.rng, *kind, self.mood, false, self.scale);
                self.glass.push(chip);
            }
        }
        self.sparks += dt * self.treble * 40.0;
        while self.sparks >= 1.0 {
            self.sparks -= 1.0;
            if self.glass.len() < self.style.limit {
                let chip = Chip::new(&mut self.rng, Kind::Spark, self.mood, false, self.scale);
                self.glass.push(chip);
            }
        }
        // The glass circles the cell with its own small wander, and bounces off the wall.
        let settle = 1.0 - (-1.6 * dt).exp();
        let turn = 0.6 + 1.6 * self.mids;
        let (mid, wall, roam) = (MID * self.scale, WALL * self.scale, 7.0 * self.scale);
        for chip in &mut self.glass {
            let (rx, ry) = (chip.pos.0 - mid, chip.pos.1 - mid);
            let wander = (roam * (self.t * 0.7 + chip.phase).sin(), roam * (self.t * 0.6 + chip.phase * 1.3).cos());
            // Tiny glass returns to its own distance from the middle, so it spreads over the
            // whole rosette instead of crowding its center or its rim.
            let (rate, home) = if chip.kind.fine() { (HOLD, chip.home) } else { (self.style.gather, 0.0) };
            let pull = rate * (1.0 - home / rx.hypot(ry).max(1e-3));
            chip.vel.0 += (-ry * self.swirl - rx * pull + wander.0 - chip.vel.0) * settle;
            chip.vel.1 += (rx * self.swirl - ry * pull + wander.1 - chip.vel.1) * settle;
            chip.pos.0 += chip.vel.0 * dt;
            chip.pos.1 += chip.vel.1 * dt;
            chip.angle += chip.spin * turn * dt;
            chip.age += dt;
            let (rx, ry) = (chip.pos.0 - mid, chip.pos.1 - mid);
            let r = rx.hypot(ry);
            let limit = (wall - chip.size * 0.3).max(wall * 0.3);
            if r > limit {
                let (nx, ny) = (rx / r, ry / r);
                chip.pos = (mid + nx * limit, mid + ny * limit);
                let out = chip.vel.0 * nx + chip.vel.1 * ny;
                if out > 0.0 {
                    chip.vel.0 -= nx * out * 1.7;
                    chip.vel.1 -= ny * out * 1.7;
                }
            }
        }
    }

    /// Moves the old light with the glass, then lights the glass where it is now.
    fn paint(&self, cell: &mut Field, dt: f32) {
        let keep = per_frame(self.style.keep, dt);
        let mid = MID * self.scale;
        cell.warp_flow((mid, mid), (mid, mid), 1.0, self.swirl * dt, keep, false,
            Flow { time: self.t, amount: (0.015 + 0.05 * self.mids) * self.style.flow, dt });
        // Each frame adds what decays, so a still piece holds a steady brightness at any frame rate.
        let light = self.energy * (1.0 - keep);
        if light > 0.0 {
            for chip in &self.glass {
                chip.draw(cell, light, self);
            }
        }
    }
}

#[derive(Default)]
pub struct Kaleidoscope {
    /// The object cell: the glass at the far end of the mirror tube.
    cell: Field,
    /// The reflected image, before the skin colors it.
    view: Field,
    /// Built from the seed on the first frame after a reset.
    scene: Option<Scene>,
    controls: Controls,
    hand: Hand,
}

impl Mode for Kaleidoscope {
    fn name(&self) -> &'static str { "KALEIDOSCOPE" }

    fn configure(&mut self, preset: &crate::preset::Preset) { self.controls = Controls::new(preset.seed); }

    fn interact(&mut self, controls: Controls) { self.controls = controls; }

    fn reset(&mut self, c: &mut Canvas) {
        c.fill(SCREEN);
        self.cell.clear();
        self.view.clear();
        self.scene = None;
        self.hand = Hand::default();
    }

    fn draw(&mut self, c: &mut Canvas, f: &Frame, pal: &Palette, dt: f32) {
        let dt = dt.clamp(0.0, 0.2);
        self.hand.update(&self.controls, dt);
        let seed = self.controls.seed;
        let scene = self.scene.get_or_insert_with(|| Scene::new(seed, 1.0, Style::KALEIDOSCOPE));
        scene.update(f, dt, &self.hand);
        self.cell.fit(CELL, CELL);
        scene.paint(&mut self.cell, dt);

        let (w, h) = (c.w, c.h);
        self.view.fit(w, h);
        let side = w.min(h).max(1) as f32;
        // Dragging moves the center of the mirrors and twists them.
        let hand = &self.hand;
        let center = (w as f32 * (0.5 + (hand.x - 0.5) * hand.force * 0.45),
                      h as f32 * (0.5 + (hand.y - 0.5) * hand.force * 0.45));
        let twist = (hand.x - 0.5) * hand.force * 1.5;
        let zoom = side * self.controls.scale * (1.0 + 0.06 * scene.bass + scene.kick);
        let current = Lens::new(&scene.pattern, zoom, twist, scene.turn);
        let previous = scene.previous.as_ref().map(|(p, fade)| {
            (Lens::new(p, zoom, twist, scene.turn), fade * fade * (3.0 - 2.0 * fade))
        });
        // A gentle wobble, as if the tube were filled with water. Mids make it stronger.
        let wobble = side * scene.ripple;
        let shift_x: Vec<_> = (0..h).map(|y| (y as f32 / side * 7.0 + scene.t * 0.9).sin() * wobble).collect();
        let shift_y: Vec<_> = (0..w).map(|x| (x as f32 / side * 6.0 - scene.t * 0.8).sin() * wobble).collect();
        let cell = &self.cell;
        self.view.paint(|x, y| {
            let px = x as f32 + 0.5 - center.0 + shift_x[y];
            let py = y as f32 + 0.5 - center.1 + shift_y[x];
            let (mut e, mut eu) = current.look(cell, px, py);
            if let Some((lens, mix)) = &previous {
                let (e0, eu0) = lens.look(cell, px, py);
                e = e0 + (e - e0) * mix;
                eu = eu0 + (eu - eu0) * mix;
            }
            (e, if e > 1e-6 { eu / e } else { 0.0 })
        });
        self.view.resolve(c, pal);
    }
}

/// Mirror pairs in the rosette, chosen by the seed. Four pairs make 8 wedges; eight make 16.
const FOLDS: [usize; 5] = [4, 5, 6, 7, 8];
/// Image pixels per cell pixel that the rosette aims for. The cell grows with the image, up
/// to `MAX_SCALE` times `CELL`, so one cell pixel covers one image pixel and the tiny glass
/// stays sharp when one rosette fills a large image.
const MAGNIFY: f32 = 1.0;
const MAX_SCALE: f32 = 4.0;

/// A two-mirror kaleidoscope: one rosette, fixed at the center of the image and large enough
/// to reach the corners. The mirrors never move. Only the glass behind them tumbles and turns.
#[derive(Default)]
pub struct Mandala {
    cell: Field,
    view: Field,
    scene: Option<Scene>,
    /// For each image pixel: its distance from the center, and the direction it looks in once
    /// the mirrors fold it into the first wedge (cosine and sine).
    rays: Vec<[f32; 3]>,
    /// The width, height and mirror pairs that `rays` was built for.
    rays_for: (usize, usize, usize),
    controls: Controls,
    hand: Hand,
}

/// Where an image point looks once two mirrors at `wedge` radians fold it into the first
/// wedge: its distance from the center, and the cosine and sine of its direction. `x` and `y`
/// are measured from the center. Angles count from straight up, so one mirror stands upright.
#[inline]
fn ray(x: f32, y: f32, wedge: f32) -> [f32; 3] {
    let (sn, cs) = (bounce(x.atan2(-y) / wedge) * wedge).sin_cos();
    [x.hypot(y), cs, sn]
}

impl Mandala {
    fn folds(&self) -> usize {
        FOLDS[(variation(self.controls.seed, 0x4D41) * FOLDS.len() as f32) as usize % FOLDS.len()]
    }

    fn aim(&mut self, w: usize, h: usize, folds: usize) {
        if self.rays_for == (w, h, folds) && self.rays.len() == w * h {
            return;
        }
        let wedge = PI / folds as f32;
        let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
        self.rays = (0..h).flat_map(|y| (0..w).map(move |x| ray(x as f32 + 0.5 - cx, y as f32 + 0.5 - cy, wedge)))
            .collect();
        self.rays_for = (w, h, folds);
    }
}

impl Mode for Mandala {
    fn name(&self) -> &'static str { "MANDALA" }

    fn configure(&mut self, preset: &crate::preset::Preset) { self.controls = Controls::new(preset.seed); }

    fn interact(&mut self, controls: Controls) { self.controls = controls; }

    fn reset(&mut self, c: &mut Canvas) {
        c.fill(SCREEN);
        self.cell.clear();
        self.view.clear();
        self.scene = None;
        self.hand = Hand::default();
    }

    fn draw(&mut self, c: &mut Canvas, f: &Frame, pal: &Palette, dt: f32) {
        let dt = dt.clamp(0.0, 0.2);
        self.hand.update(&self.controls, dt);
        let (w, h) = (c.w, c.h);
        // The distance from the center to a corner. The rosette reaches this far.
        let reach = (w as f32).hypot(h as f32).max(1.0) * 0.5;
        let seed = self.controls.seed;
        // Dragging stirs the glass. The scene reads the hand for that.
        let scene = self.scene.get_or_insert_with(|| {
            Scene::new(seed, (reach / (WINDOW * MAGNIFY)).clamp(1.0, MAX_SCALE), Style::MANDALA)
        });
        scene.update(f, dt, &self.hand);
        let side = (CELL as f32 * scene.scale).round() as usize;
        self.cell.fit(side, side);
        scene.paint(&mut self.cell, dt);
        let (mid, window, turn) = (MID * scene.scale, WINDOW * scene.scale, scene.turn);

        let folds = self.folds();
        self.aim(w, h, folds);
        self.view.fit(w, h);
        // Cell pixels per image pixel. At the normal size the corners see the rim of the glass;
        // the wheel makes the glass larger or smaller, but the center never moves.
        let k = window / (reach * self.controls.scale);
        let (sn, cs) = turn.sin_cos();
        let (cell, rays) = (&self.cell, &self.rays);
        self.view.paint(|x, y| {
            let [d, dx, dy] = rays[y * w + x];
            let mut r = d * k;
            // Made smaller, the glass ends before the corners. Its rim then acts as a round
            // mirror, so the reflections still fill the image.
            if r > window {
                r = (2.0 * window - r).max(0.0);
            }
            let (e, eu) = cell.sample(mid + r * (dx * cs - dy * sn), mid + r * (dx * sn + dy * cs));
            (e, if e > 1e-6 { eu / e } else { 0.0 })
        });
        self.view.resolve(c, pal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analyzer;
    use crate::canvas::{pack, unpack};

    #[test]
    fn mirrors_tile_the_plane_without_seams() {
        let step = 0.001;
        for mirrors in TUBES {
            let window = mirrors.window(0.7);
            let scale = WINDOW / mirrors.reach();
            for i in 0..300 {
                for j in 0..300 {
                    let (x, y) = (i as f32 * 0.0217 - 3.1, j as f32 * 0.0193 - 2.9);
                    let p = mirrors.fold(x, y, &window);
                    assert!((p.0 - MID).hypot(p.1 - MID) <= WINDOW + 0.01, "{mirrors:?} leaves the window at {x}, {y}");
                    // Reflections keep distances, so a small step on the plane is a small step in the cell.
                    for q in [mirrors.fold(x + step, y, &window), mirrors.fold(x, y + step, &window)] {
                        let moved = (q.0 - p.0).hypot(q.1 - p.1);
                        assert!(moved <= scale * step * 1.01 + 0.002, "{mirrors:?} has a seam at {x}, {y}");
                    }
                }
            }
        }
    }

    fn light(c: &Canvas) -> u64 {
        c.px.iter().map(|&v| unpack(v).iter().map(|&v| u64::from(v)).sum::<u64>()).sum()
    }

    fn play(mode: &mut dyn Mode, c: &mut Canvas, bands: [f32; 3], frames: usize) {
        let mut a = Analyzer::new();
        a.frame.peak = 0.3;
        a.frame.music.bands = bands;
        a.frame.music.slow = bands;
        for _ in 0..frames { mode.draw(c, &a.frame, &Palette::new(3), 1.0 / 60.0); }
    }

    /// Every part of a 200 by 120 image holds reflected glass, not only the middle.
    fn assert_filled(c: &Canvas) {
        for gy in 0..3 {
            for gx in 0..4 {
                let lit = (gy * 40..gy * 40 + 40).flat_map(|y| (gx * 50..gx * 50 + 50).map(move |x| y * 200 + x))
                    .filter(|&i| c.px[i] != pack(SCREEN)).count();
                assert!(lit > 200, "Region {gx}, {gy} is dark");
            }
        }
    }

    #[test]
    fn glass_fills_the_whole_image_and_follows_the_music() {
        let mut mode = Kaleidoscope::default();
        let mut c = Canvas::new(200, 120);
        mode.draw(&mut c, &Analyzer::new().frame, &Palette::new(3), 1.0 / 60.0);
        assert!(c.px.iter().all(|&v| v == pack(SCREEN)), "Silence starts dark");

        mode.reset(&mut c);
        play(&mut mode, &mut c, [0.5; 3], 90);
        assert_filled(&c);

        let render = |bands| {
            let mut mode = Kaleidoscope::default();
            let mut c = Canvas::new(200, 120);
            play(&mut mode, &mut c, bands, 60);
            c.px
        };
        let base = render([0.0; 3]);
        for band in 0..3 {
            let mut bands = [0.0; 3];
            bands[band] = 0.9;
            assert_ne!(base, render(bands), "Band {band} must change the image");
        }

        let first = c.px.clone();
        play(&mut mode, &mut c, [0.5; 3], 30);
        assert_ne!(first, c.px, "The image must keep moving");
        let active = light(&c);
        let silence = Analyzer::new();
        for _ in 0..480 { mode.draw(&mut c, &silence.frame, &Palette::new(3), 1.0 / 60.0); }
        let dark = light(&Canvas { w: c.w, h: c.h, px: vec![pack(SCREEN); c.px.len()] });
        assert!(light(&c) - dark < (active - dark) / 20, "The image must fade after playback ends");
    }

    #[test]
    fn tubes_change_over_time_and_survive_any_size() {
        let mut mode = Kaleidoscope::default();
        let mut c = Canvas::new(120, 90);
        let mut tubes = Vec::new();
        for _ in 0..40 {
            play(&mut mode, &mut c, [0.6, 0.5, 0.4], 60);
            let mirrors = mode.scene.as_ref().unwrap().pattern.mirrors;
            if !tubes.contains(&mirrors) { tubes.push(mirrors); }
        }
        assert!(tubes.len() >= 3, "Only {tubes:?} in 40 seconds");
        for (w, h) in [(90, 120), (240, 100), (1, 1)] {
            c.resize(w, h);
            play(&mut mode, &mut c, [0.9; 3], 3);
        }
        mode.reset(&mut c);
        assert!(c.px.iter().all(|&v| v == pack(SCREEN)));
    }

    #[test]
    fn mandala_folds_every_direction_into_one_wedge() {
        for folds in FOLDS {
            let wedge = PI / folds as f32;
            for i in 0..500 {
                let (a, d) = (i as f32 * 0.137 - 30.0, 1.0 + i as f32 * 0.61);
                let at = |a: f32| ray(a.sin() * d, -a.cos() * d, wedge);
                let p = at(a);
                assert!((p[0] - d).abs() < 1e-3 * d);
                assert!(p[2] >= -1e-6 && p[2].atan2(p[1]) <= wedge + 1e-5, "{folds} mirror pairs: {a} leaves the wedge");
                // A turn by two wedges, or a reflection in the upright mirror, shows the same glass.
                for q in [at(a + 2.0 * wedge), at(-a)] {
                    assert!((p[1] - q[1]).abs() < 1e-3 && (p[2] - q[2]).abs() < 1e-3, "{folds} mirror pairs: seam at {a}");
                }
            }
        }
    }

    #[test]
    fn mandala_stays_centered_fills_the_image_and_follows_the_music() {
        let mut mode = Mandala::default();
        let mut c = Canvas::new(200, 120);
        mode.draw(&mut c, &Analyzer::new().frame, &Palette::new(3), 1.0 / 60.0);
        assert!(c.px.iter().all(|&v| v == pack(SCREEN)), "Silence starts dark");

        mode.reset(&mut c);
        // At the normal size, zoomed out, and zoomed in while the mouse stirs the glass.
        for (scale, pointer) in [(1.0, None), (0.5, None), (1.8, Some((0.9, 0.2)))] {
            mode.interact(Controls { scale, pointer, ..Controls::default() });
            for _ in 0..3 {
                play(&mut mode, &mut c, [0.5; 3], 30);
                assert_filled(&c);
                // The upright mirror stays on the center line in every frame.
                for y in 0..120 {
                    for x in 0..100 {
                        assert_eq!(c.px[y * 200 + x], c.px[y * 200 + 199 - x], "Scale {scale}: not mirrored at {x}, {y}");
                    }
                }
            }
        }

        mode.interact(Controls::default());
        let render = |bands| {
            let mut mode = Mandala::default();
            let mut c = Canvas::new(200, 120);
            play(&mut mode, &mut c, bands, 60);
            c.px
        };
        let base = render([0.0; 3]);
        for band in 0..3 {
            let mut bands = [0.0; 3];
            bands[band] = 0.9;
            assert_ne!(base, render(bands), "Band {band} must change the image");
        }

        let first = c.px.clone();
        play(&mut mode, &mut c, [0.5; 3], 30);
        assert_ne!(first, c.px, "The glass must keep moving");
        let active = light(&c);
        let silence = Analyzer::new();
        for _ in 0..480 { mode.draw(&mut c, &silence.frame, &Palette::new(3), 1.0 / 60.0); }
        let dark = light(&Canvas { w: c.w, h: c.h, px: vec![pack(SCREEN); c.px.len()] });
        assert!(light(&c) - dark < (active - dark) / 20, "The image must fade after playback ends");
    }

    #[test]
    fn mandala_cell_grows_with_the_image_and_survives_any_size() {
        let mut mode = Mandala::default();
        let mut sides = Vec::new();
        for (w, h) in [(120, 90), (1280, 640), (1, 1), (90, 400)] {
            let mut c = Canvas::new(w, h);
            mode.reset(&mut c);
            play(&mut mode, &mut c, [0.9; 3], 3);
            sides.push(mode.cell.w);
            assert_eq!((mode.view.w, mode.view.h), (w, h));
        }
        assert_eq!(sides[..3], [CELL, CELL * 4, CELL], "Small images keep the standard cell; large ones stop at four times it");
        assert!((CELL + 1..CELL * 4).contains(&sides[3]), "A tall image needs a cell between: {sides:?}");
        let mut seeds = Vec::new();
        for seed in 0..40 {
            mode.interact(Controls::new(seed));
            if !seeds.contains(&mode.folds()) { seeds.push(mode.folds()); }
        }
        assert_eq!(seeds.len(), FOLDS.len(), "Seeds pick every mirror count: {seeds:?}");
    }
}
