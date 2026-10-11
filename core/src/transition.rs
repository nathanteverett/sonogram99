//! Crossfade independently rendered modes without feeding the blend back into either mode.
//!
//! A transition can fade the whole picture at once, or follow a pattern: each pixel starts
//! its own fade at a time set by a map, so the new mode grows from the center, closes in
//! from the edges, sweeps around, wipes across, or spreads through soft clouds.

use std::f32::consts::{PI, TAU};

use crate::canvas::{mix, pack, unpack, Canvas};

/// The length of a transition the viewer asks for.
pub const SECONDS: f32 = 2.4;
/// How much of the transition each pixel spends fading, as a share of the whole.
const SOFT: f32 = 0.35;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// Every pixel at once.
    Fade,
    /// Grows out from the center.
    Bloom,
    /// Closes in from the edges.
    Iris,
    /// Sweeps around the center like a clock hand.
    Sweep,
    /// Wipes across from left to right.
    Wipe,
    /// Spreads through soft clouds.
    Clouds,
}

pub const PATTERNS: [Pattern; 6] = [Pattern::Fade, Pattern::Bloom, Pattern::Iris, Pattern::Sweep, Pattern::Wipe, Pattern::Clouds];

pub struct Transition {
    pub from: Canvas,
    /// None holds the last displayed blend when a user interrupts a transition.
    pub mode: Option<usize>,
    pub pattern: Pattern,
    seconds: f32,
    elapsed: f32,
    /// Each pixel's start time, 0 (first) to 1 (last). Empty for a plain fade.
    map: Vec<f32>,
}

impl Transition {
    /// A plain fade of the standard length.
    pub fn new(from: Canvas, mode: Option<usize>) -> Self {
        Self::with(from, mode, Pattern::Fade, SECONDS, 0)
    }

    /// `seconds` of 0 is a hard cut. `seed` varies the cloud pattern.
    pub fn with(from: Canvas, mode: Option<usize>, pattern: Pattern, seconds: f32, seed: u32) -> Self {
        let map = start_map(pattern, from.w, from.h, seed);
        Self { from, mode, pattern, seconds: seconds.max(0.0), elapsed: 0.0, map }
    }

    /// Returns true after the incoming mode has become fully visible.
    pub fn blend(&mut self, to: &Canvas, out: &mut Canvas, dt: f32) -> bool {
        assert_eq!((self.from.w, self.from.h), (to.w, to.h));
        out.resize(to.w, to.h);
        let t = if self.seconds > 0.0 { (self.elapsed / self.seconds).clamp(0.0, 1.0) } else { 1.0 };
        let pixels = out.px.iter_mut().zip(&self.from.px).zip(&to.px);
        if self.map.is_empty() || t >= 1.0 {
            let k = smooth(t);
            for ((out, from), to) in pixels {
                *out = pack(mix(unpack(*from), unpack(*to), k));
            }
        } else {
            let reach = t * (1.0 + SOFT);
            for (((out, from), to), m) in pixels.zip(&self.map) {
                let k = (reach - m) / SOFT;
                *out = if k <= 0.0 {
                    *from
                } else if k >= 1.0 {
                    *to
                } else {
                    pack(mix(unpack(*from), unpack(*to), smooth(k)))
                };
            }
        }
        self.elapsed += dt.max(0.0);
        t >= 1.0
    }
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Each pixel's start time for a pattern, spread over the whole range 0 to 1.
fn start_map(pattern: Pattern, w: usize, h: usize, seed: u32) -> Vec<f32> {
    if pattern == Pattern::Fade || w == 0 || h == 0 {
        return Vec::new();
    }
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let reach = cx.hypot(cy).max(1.0);
    let noise = Clouds::new(seed);
    let scale = 4.0 / w.min(h).max(1) as f32;
    let mut map: Vec<f32> = (0..w * h).map(|i| {
        let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
        match pattern {
            Pattern::Fade => 0.0,
            Pattern::Bloom => (x - cx).hypot(y - cy) / reach,
            Pattern::Iris => 1.0 - (x - cx).hypot(y - cy) / reach,
            // Clockwise from twelve o'clock.
            Pattern::Sweep => ((x - cx).atan2(cy - y) / TAU).rem_euclid(1.0),
            // Slightly slanted, so the edge reads as a moving wipe and not a scrolling line.
            Pattern::Wipe => (x / w as f32) * 0.85 + (y / h as f32) * 0.15,
            Pattern::Clouds => noise.at(x * scale, y * scale),
        }
    }).collect();
    let (lo, hi) = map.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &m| (lo.min(m), hi.max(m)));
    let span = (hi - lo).max(1e-6);
    for m in &mut map {
        *m = (*m - lo) / span;
    }
    map
}

/// Smooth value noise in three octaves.
struct Clouds {
    seed: u32,
    turn: f32,
}

impl Clouds {
    fn new(seed: u32) -> Self {
        Self { seed, turn: crate::interaction::variation(seed, 7) * PI }
    }

    fn lattice(&self, x: i32, y: i32, octave: u32) -> f32 {
        let key = (x as u32).wrapping_mul(0x8DA6_B343) ^ (y as u32).wrapping_mul(0xD816_3841) ^ octave.wrapping_mul(0xCB1A_B31F);
        crate::interaction::variation(self.seed ^ key, octave)
    }

    fn octave(&self, x: f32, y: f32, octave: u32) -> f32 {
        let (ix, iy) = (x.floor(), y.floor());
        let (fx, fy) = (smooth(x - ix), smooth(y - iy));
        let (ix, iy) = (ix as i32, iy as i32);
        let top = self.lattice(ix, iy, octave) * (1.0 - fx) + self.lattice(ix + 1, iy, octave) * fx;
        let bottom = self.lattice(ix, iy + 1, octave) * (1.0 - fx) + self.lattice(ix + 1, iy + 1, octave) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    fn at(&self, x: f32, y: f32) -> f32 {
        // Turning the lattice hides its grid in the coarsest octave.
        let (s, c) = self.turn.sin_cos();
        let (x, y) = (x * c - y * s, x * s + y * c);
        self.octave(x, y, 0) * 0.6 + self.octave(x * 2.1 + 17.0, y * 2.1, 1) * 0.28 + self.octave(x * 4.3, y * 4.3 + 9.0, 2) * 0.12
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blend_preserves_sources_and_reaches_both_endpoints() {
        let mut from = Canvas::new(2, 1);
        from.fill([240, 0, 0]);
        let mut to = Canvas::new(2, 1);
        to.fill([0, 0, 240]);
        let mut out = Canvas::new(1, 1);
        let mut transition = Transition::new(from, Some(0));
        assert!(!transition.blend(&to, &mut out, SECONDS / 2.0));
        assert_eq!(out.px[0], pack([240, 0, 0]));
        assert!(!transition.blend(&to, &mut out, SECONDS / 2.0));
        assert_eq!(out.px[0], pack([120, 0, 120]));
        assert!(transition.blend(&to, &mut out, 0.0));
        assert_eq!(out.px, to.px);
        assert_eq!(transition.from.px[0], pack([240, 0, 0]));
    }

    #[test]
    fn patterns_move_across_the_picture_and_finish_on_time() {
        let (a, b) = (pack([200, 10, 10]), pack([10, 10, 200]));
        for pattern in PATTERNS.into_iter().skip(1) {
            let mut from = Canvas::new(64, 40);
            from.px.fill(a);
            let mut to = Canvas::new(64, 40);
            to.px.fill(b);
            let mut out = Canvas::new(1, 1);
            let mut tr = Transition::with(from, Some(1), pattern, 2.0, 99);
            assert!(!tr.blend(&to, &mut out, 1.0));
            assert!(out.px.iter().all(|p| *p == a), "{pattern:?} starts from the old picture");
            assert!(!tr.blend(&to, &mut out, 1.0));
            let (old, new) = (out.px.iter().filter(|p| **p == a).count(), out.px.iter().filter(|p| **p == b).count());
            assert!(old > 100 && new > 100, "{pattern:?} halfway shows both pictures: {old} old, {new} new");
            assert!(tr.blend(&to, &mut out, 0.0));
            assert_eq!(out.px, to.px, "{pattern:?} ends on the new picture");
        }
        let mut bloom = Transition::with(Canvas::new(64, 40), None, Pattern::Bloom, 2.0, 0);
        let mut to = Canvas::new(64, 40);
        to.px.fill(b);
        let mut out = Canvas::new(1, 1);
        bloom.blend(&to, &mut out, 0.8);
        bloom.blend(&to, &mut out, 0.0);
        assert_eq!(out.px[20 * 64 + 32], b, "Bloom reaches the center first");
        assert_eq!(out.px[0], 0, "Bloom reaches the corners last");
    }

    #[test]
    fn hard_cuts_show_the_new_picture_at_once() {
        let mut to = Canvas::new(8, 8);
        to.px.fill(pack([1, 2, 3]));
        let mut out = Canvas::new(1, 1);
        let mut cut = Transition::with(Canvas::new(8, 8), Some(0), Pattern::Sweep, 0.0, 1);
        assert!(cut.blend(&to, &mut out, 1.0 / 60.0));
        assert_eq!(out.px, to.px);
    }
}
