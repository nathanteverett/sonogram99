//! Repeatable procedural variation and per-visual mouse adjustments.

pub const DEFAULT_SEED: u32 = 0x534F_4E4F;
pub const BAR_H: usize = 17;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    pub seed: u32,
    pub original_seed: u32,
    /// Position in the visualization, normalized to 0..1; only present during a drag.
    pub pointer: Option<(f32, f32)>,
    pub scale: f32,
}

impl Default for Controls {
    fn default() -> Self { Self::new(DEFAULT_SEED) }
}

impl Controls {
    pub fn new(seed: u32) -> Self {
        Self { seed, original_seed: seed, pointer: None, scale: 1.0 }
    }

    pub fn reset(&mut self) { *self = Self::new(self.original_seed); }

    pub fn shuffle(&mut self) {
        self.seed = self.seed.wrapping_add(0x9E37_79B9);
        self.pointer = None;
    }

    pub fn scroll(&mut self, amount: f32) {
        if amount.is_finite() {
            self.scale = (self.scale * (amount.clamp(-10.0, 10.0) * 0.12).exp()).clamp(0.5, 1.8);
        }
    }
}

/// Stateless hash: the same seed and channel always give the same procedural parameter.
pub fn variation(seed: u32, channel: u32) -> f32 {
    let mut x = seed.wrapping_add(channel.wrapping_mul(0x9E37_79B9));
    x = (x ^ (x >> 16)).wrapping_mul(0x7FEB_352D);
    x = (x ^ (x >> 15)).wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x >> 8) as f32 / 16_777_216.0
}

/// A small seeded random number generator (xorshift32) for choices such as the next
/// transition pattern. The same seed always gives the same choices.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        // xorshift never leaves zero, so the state must not start there.
        Rng(seed.wrapping_mul(0x9E37_79B9) | 1)
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// A number from 0 up to (not including) 1.
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 16_777_216.0
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.unit() * n as f32) as usize % n.max(1)
    }
}

/// Smoothly engage and release the hand without jumping when a drag starts elsewhere.
#[derive(Default)]
pub struct Hand {
    pub x: f32,
    pub y: f32,
    pub force: f32,
}

impl Hand {
    pub fn update(&mut self, controls: &Controls, dt: f32) {
        let weight = 1.0 - (-12.0 * dt.clamp(0.0, 0.2)).exp();
        if let Some((x, y)) = controls.pointer {
            if self.force < 0.001 { self.x = x; self.y = y; }
            self.x += (x - self.x) * weight;
            self.y += (y - self.y) * weight;
            self.force += (1.0 - self.force) * weight;
        } else {
            self.force *= 1.0 - weight;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Button { Shuffle, Reset }

pub fn buttons(w: usize) -> [(Button, i32, i32, &'static str); 2] {
    [(Button::Shuffle, w as i32 - 105, 57, "SHUFFLE"),
     (Button::Reset, w as i32 - 45, 42, "RESET")]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reset_restores_seed_and_mouse_adjustments() {
        let mut c = Controls::new(42);
        let original = c;
        c.shuffle();
        assert_ne!(c.seed, original.seed);
        c.pointer = Some((0.2, 0.8));
        c.scroll(100.0);
        assert!(c.scale <= 1.8);
        c.reset();
        assert_eq!(c, original);
        assert_eq!(variation(c.seed, 2), variation(original.seed, 2));
        assert_ne!(variation(c.seed, 2), variation(c.seed.wrapping_add(1), 2));
    }
}
