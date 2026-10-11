//! Orbs as a continuous luminous membrane: lobes merge and separate in a warped field.

use crate::analysis::Frame;
use crate::canvas::Canvas;
use crate::glow::{per_frame, Field, Flow};
use crate::modes::Mode;
use crate::palette::{Palette, SCREEN};
use crate::post::{self, Look};
use crate::preset::{Inputs, Preset};
use crate::interaction::{Controls, Hand, variation};

#[derive(Default)]
pub struct Orbs {
    trails: Field,
    body: Field,
    t: f32,
    energy: f32,
    preset: Preset,
    controls: Controls,
    hand: Hand,
    /// Scratch space for the echo effect.
    copy: Vec<u32>,
}

impl Mode for Orbs {
    fn name(&self) -> &'static str { "ORBS" }

    fn configure(&mut self, preset: &Preset) {
        self.preset = preset.clone();
        self.controls = Controls::new(preset.seed);
    }

    fn interact(&mut self, controls: Controls) { self.controls = controls; }

    fn reset(&mut self, c: &mut Canvas) {
        c.fill(SCREEN);
        self.trails.clear();
        self.body.clear();
        self.t = 0.0;
        self.energy = 0.0;
        self.hand = Hand::default();
    }

    fn draw(&mut self, c: &mut Canvas, f: &Frame, pal: &Palette, dt: f32) {
        let dt = dt.clamp(0.0, 0.2);
        self.hand.update(&self.controls, dt);
        let p = &self.preset;
        let [bass, mids, treble] = [f.music.bands[0] * p.bass, f.music.bands[1] * p.mids, f.music.bands[2] * p.treble];
        self.t += dt * p.speed * (0.45 + 0.38 * f.music.slow[1] + 0.22 * bass);
        let t = self.t;
        let target = (f.peak * 7.0).clamp(0.0, 1.0);
        let rate = if target > self.energy { 12.0 } else { 3.0 };
        self.energy += (target - self.energy) * (1.0 - (-rate * dt).exp());
        let energy = self.energy;
        let (w, h) = (c.w, c.h);
        let m = w.min(h).max(1) as f32;
        let center = (w as f32 * 0.5, h as f32 * 0.5);
        let hand_x = (self.hand.x - 0.5) * w as f32 / m;
        let hand_y = (self.hand.y - 0.5) * h as f32 / m;
        let force = self.hand.force;
        self.trails.fit(w, h);
        self.body.fit(w, h);
        self.trails.warp_flow(center, center, per_frame(1.002 + bass * 0.002, dt), dt * 0.035,
            per_frame(p.persistence, dt), true,
            Flow { time: t, amount: p.warp * (0.024 + f.music.slow[1] * 0.045 + force * 0.07), dt });

        // Domain warping bends the entire membrane; broad and fine ripples have separate controls.
        let bend = (0.016 + mids * 0.046) * p.warp;
        let fine = treble * 0.007 * p.detail;
        let xs: Vec<_> = (0..w).map(|x| {
            let u = (x as f32 - center.0) / m;
            (u, (u * 8.0 + t * 0.8).sin() * bend + (u * 31.0 - t * 1.3).sin() * fine)
        }).collect();
        let ys: Vec<_> = (0..h).map(|y| {
            let v = (y as f32 - center.1) / m;
            (v, (v * 9.0 - t * 0.7).sin() * bend + (v * 27.0 + t * 1.1).sin() * fine)
        }).collect();

        // Unequal lobes continuously fold into one another, rather than orbiting as rigid balls.
        let lobes: Vec<_> = (0..5).map(|i| {
            let phase = i as f32 * 2.39996 + variation(self.controls.seed, i) * 1.8;
            let a = t * (0.48 + i as f32 * 0.07) + phase;
            let separation = 0.17 + 0.055 * (t * 0.63 + phase).sin() + f.music.width * 0.045;
            let mut x = a.cos() * separation * 1.35;
            let mut y = (a * 0.83 + phase * 0.3).sin() * separation;
            let pull = force * (0.3 + i as f32 * 0.09);
            x += (hand_x - x) * pull;
            y += (hand_y - y) * pull;
            let radius = (0.090 + 0.016 * (t * 0.8 + phase).sin()) * (1.0 + bass * 0.25 + f.music.onset * p.bass * 0.09) * self.controls.scale;
            (x, y, radius * radius, 0.75 + 0.3 * (t * 0.4 + phase).sin())
        }).collect();
        let hue = 0.5 + 0.16 * (t * 0.3).sin();
        let accelerated = crate::acceleration::run(crate::acceleration::Operation::Orbs {
            dst: self.body.pixels_mut(), w, h, xs: &xs, ys: &ys, lobes: &lobes,
            hand: [hand_x, hand_y, force], hue, energy, detail: p.detail,
        });
        if !accelerated {
            self.body.paint(|x, y| {
                if energy < 1e-5 { return (0.0, 0.0); }
                let mut u = xs[x].0 + ys[y].1;
                let mut v = ys[y].0 + xs[x].1;
                let dx = u - hand_x;
                let dy = v - hand_y;
                let stir = force * 0.12 / (1.0 + (dx * dx + dy * dy) * 18.0);
                u += dy * stir;
                v -= dx * stir;
                let mut density = 0.0;
                for &(cx, cy, r2, stretch) in &lobes {
                    let dx = (u - cx) * stretch;
                    let dy = (v - cy) / stretch;
                    density += r2 / (dx * dx + dy * dy + r2 * 0.16);
                }
                // Soft luminous contour, translucent interior, and smaller nested folds.
                let edge = 1.0 / (1.0 + (density - 1.0).powi(2) * 220.0);
                let inner = ((density - 1.0) * 2.0).clamp(0.0, 1.0);
                let fold = 1.0 / (1.0 + (density - 1.75).powi(2) * 80.0);
                let core = 1.0 / (1.0 + (density - 2.65).powi(2) * 45.0);
                let e = (edge * 2.8 + inner * 0.16 + p.detail * (fold * 0.8 + core * 0.4)) * energy;
                let color = hue + u * 0.8 - v * 0.55 + (density.min(3.0) - 1.0) * 0.12;
                (e, color)
            });
            crate::acceleration::finish_software();
        }
        self.trails.add_scaled(&self.body, dt * 2.4);
        self.trails.resolve_at(c, pal, p.background);
        self.body.composite(c, pal, p.body);
        post::apply(c, &Look::new(&p.effects, &Inputs::new(f, p, t)), p.mirror, &mut self.copy);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analyzer;
    use crate::canvas::{pack, unpack};

    #[test]
    fn fluid_responds_to_each_band_and_fades_in_silence() {
        let render = |band: Option<usize>| {
            let mut mode = Orbs::default();
            let mut a = Analyzer::new();
            a.frame.peak = 0.3;
            if let Some(i) = band { a.frame.music.bands[i] = 0.9; }
            let mut c = Canvas::new(120, 90);
            for _ in 0..40 { mode.draw(&mut c, &a.frame, &Palette::new(2), 1.0 / 60.0); }
            (mode, c)
        };
        let (_, base) = render(None);
        for i in 0..3 { assert_ne!(base.px, render(Some(i)).1.px, "Band {i} must affect shape"); }
        let (mut mode, mut c) = render(Some(0));
        let light = |c: &Canvas| -> u64 { c.px.iter().map(|p| unpack(*p).iter().map(|v| *v as u64).sum::<u64>()).sum() };
        let active = light(&c);
        let silence = Analyzer::new();
        for _ in 0..480 { mode.draw(&mut c, &silence.frame, &Palette::new(2), 1.0 / 60.0); }
        assert!(light(&c) < active / 3);
        for (w, h) in [(90, 120), (240, 100), (1, 1)] {
            c.resize(w, h);
            mode.draw(&mut c, &silence.frame, &Palette::new(2), 1.0 / 30.0);
        }
        mode.reset(&mut c);
        assert!(c.px.iter().all(|p| *p == pack(SCREEN)));
    }
}
