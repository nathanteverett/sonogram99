//! Tunnel: a feedback visual drawn entirely from the recipe. On each frame it moves the last
//! frame's light through a coarse grid (the motion), draws the border, the shapes and the
//! waves on top, shows the light in the skin's colors, and applies the effects. A new
//! recipe file gives a new look without new code.

use std::f32::consts::{FRAC_1_SQRT_2, TAU};

use crate::analysis::{Frame, TD};
use crate::canvas::{mix, Canvas};
use crate::glow::{per_frame, Field, Fill, Grid};
use crate::interaction::{variation, Controls, Hand};
use crate::modes::{band_max, shape as level_shape, wave_peak, Mode, F_MAX, F_MIN};
use crate::palette::{Palette, SCREEN};
use crate::post::{self, Look};
use crate::preset::{Inputs, Param, Preset, ShapeKind, Wave, WaveKind};

#[derive(Default)]
pub struct Tunnel {
    field: Field,
    t: f32,
    preset: Preset,
    energy: f32,
    controls: Controls,
    hand: Hand,
    /// Scratch space for the echo effect.
    copy: Vec<u32>,
}

/// One trace point: x, y, and its place on the skin's gradient.
type Point = (f32, f32, f32);

impl Mode for Tunnel {
    fn name(&self) -> &'static str {
        "TUNNEL"
    }

    fn configure(&mut self, preset: &Preset) {
        self.preset = preset.clone();
        self.controls = Controls::new(preset.seed);
    }

    fn interact(&mut self, controls: Controls) { self.controls = controls; }

    fn reset(&mut self, c: &mut Canvas) {
        c.fill(SCREEN);
        self.field.clear();
        self.t = 0.0;
        self.energy = 0.0;
        self.hand = Hand::default();
    }

    fn draw(&mut self, c: &mut Canvas, f: &Frame, pal: &Palette, dt: f32) {
        let dt = dt.clamp(0.0, 0.2);
        self.field.fit(c.w, c.h);
        self.hand.update(&self.controls, dt);
        let p = &self.preset;
        self.energy += ((f.peak * 7.0).clamp(0.0, 1.0) - self.energy) * (1.0 - (-7.0 * dt).exp());
        self.t += dt * p.speed;
        let t = self.t + variation(self.controls.seed, 0) * 12.0;
        let inputs = Inputs::new(f, p, t);
        let ev = |q: &Param| q.eval(&inputs);
        let (w, h) = (c.w as f32, c.h as f32);
        let m = w.min(h);
        // Motion is set per frame at 60 frames a second, and the recipe's speed scales it.
        let frames = dt * 60.0 * p.speed;
        // Light and opacity are set per frame at 60 frames a second.
        let ticks = dt * 60.0;

        // Each frame shows the last one moved: zoomed, turned and bent, then dimmed.
        let mo = &p.motion;
        let (home_x, home_y) = (ev(&mo.cx), ev(&mo.cy));
        let pull = self.hand.force * 0.7;
        let (cx, cy) = (w * (home_x + (self.hand.x - home_x) * pull), h * (home_y + (self.hand.y - home_y) * pull));
        // How far the mouse has pulled the center; the layers move with it.
        let shift = (cx - w * home_x, cy - h * home_y);
        let (ox, oy) = (cx + ev(&mo.dx) * w * frames, cy + ev(&mo.dy) * h * frames);
        let (zoom, zoom_edge, rot, swirl, ripple) = (ev(&mo.zoom), ev(&mo.zoom_edge), ev(&mo.rot), ev(&mo.swirl), ev(&mo.ripple));
        let flow = p.warp * (ev(&mo.flow) + self.hand.force * 0.12) * m * dt;
        let keep = per_frame(p.persistence, dt);
        let dark = ev(&mo.center_dark);
        let grid = Grid::build(c.w, c.h, |x, y| {
            let (dx, dy) = (x - ox, y - oy);
            // 1 at the middle of the nearest long edge.
            let r = dx.hypot(dy) / (m * 0.5);
            let z = (zoom + zoom_edge * r + ripple * 0.02 * (r * 9.0 - t * 4.0).sin()).max(0.5).powf(frames);
            let (sn, cs) = ((rot + swirl / (1.0 + 4.0 * r * r)) * frames).sin_cos();
            let (u, v) = (dy / m, dx / m);
            let sx = cx + (cs * dx + sn * dy) / z + ((u * 8.0 + t * 0.7).sin() + 0.4 * (u * 15.0 - t * 0.4).sin()) * flow;
            let sy = cy + (cs * dy - sn * dx) / z + ((v * 7.0 - t * 0.6).sin() + 0.4 * (v * 13.0 + t * 0.5).sin()) * flow;
            let shade = (1.0 - 0.12 * dark * (-r * r * 6.0).exp()).max(0.0).powf(frames);
            [sx, sy, keep * shade]
        });
        self.field.warp_grid(&grid, false);

        let border = ev(&mo.border);
        if border > 0.0 {
            self.field.border((m * ev(&mo.border_size)).max(1.0), ev(&mo.border_color), border * ticks);
        }

        let scale = self.controls.scale;
        for shape in p.shapes.iter().flatten() {
            let s = &shape.set;
            let at = (w * ev(&s.x) + shift.0, h * ev(&s.y) + shift.1);
            let size = ev(&s.size) * scale;
            let radius = m * size;
            let sides = ev(&s.sides).round() as usize;
            let angle = ev(&s.angle) + ev(&s.spin) * t;
            let color = ev(&s.color);
            let alpha = 1.0 - (1.0 - ev(&s.opacity)).powf(ticks);
            match shape.kind {
                ShapeKind::Outline => {
                    let corners: Vec<Point> = (0..=sides).map(|k| {
                        let a = angle + k as f32 * TAU / sides as f32;
                        (at.0 + a.cos() * radius, at.1 + a.sin() * radius, color)
                    }).collect();
                    self.field.polyline(&corners, (m * 0.004).max(1.0), ev(&s.light) * ticks);
                }
                ShapeKind::Filled => self.field.polygon(at, radius, sides, angle, Fill::Light(ev(&s.light), color), alpha),
                ShapeKind::Textured => {
                    // At zoom 1 the picture's shorter side fits across the shape.
                    let scale = 1.0 / (2.0 * size.max(0.005) * ev(&s.zoom));
                    self.field.polygon(at, radius, sides, angle, Fill::Copy { scale, twist: ev(&s.twist) }, alpha);
                }
            }
        }

        let mut traces = Vec::new();
        for wave in p.waves.iter().flatten() {
            let pts = trace(wave, f, &inputs, t, (w, h), shift, scale, p.detail);
            let brush = (m * ev(&wave.set.thick)).max(1.5);
            self.field.polyline(&pts, brush, ev(&wave.set.light) * self.energy * ticks);
            traces.push(pts);
        }
        self.field.resolve_at(c, pal, p.background);
        // The current waves stay readable against the slower, painted feedback.
        for pts in &traces {
            for s in pts.windows(2) {
                c.line(s[0].0, s[0].1, s[1].0, s[1].1, mix(pal.glow(s[1].2), pal.skin.peak, 0.12), self.energy * p.body, 2, false);
            }
        }
        post::apply(c, &Look::new(&p.effects, &inputs), p.mirror, &mut self.copy);
    }
}

/// The points of one wave. `shift` is the mouse's pull on the center.
#[allow(clippy::too_many_arguments)]
fn trace(wave: &Wave, f: &Frame, inputs: &Inputs, t: f32, (w, h): (f32, f32), shift: (f32, f32), scale: f32, detail: f32) -> Vec<Point> {
    let ev = |q: &Param| q.eval(inputs);
    let s = &wave.set;
    let m = w.min(h);
    let (x, y) = (w * ev(&s.x) + shift.0, h * ev(&s.y) + shift.1);
    let size = m * ev(&s.size) * scale;
    let amp = m * ev(&s.amp) * detail;
    let turn = ev(&s.angle) + ev(&s.spin) * t;
    let (color, spread) = (ev(&s.color), ev(&s.spread));
    let (wobble, fringe) = (ev(&s.wobble), ev(&s.fringe) * detail);
    // Quiet music still draws a readable trace.
    let gain = (0.7 / wave_peak(&f.mid, 4)).min(4.0);
    match wave.kind {
        WaveKind::Ring => {
            const PTS: usize = 192;
            let stride = TD / PTS;
            let mut pts: Vec<Point> = (0..PTS).map(|i| {
                let a = i as f32 / PTS as f32 * TAU + turn;
                let rad = size * (1.0 + wobble * (a * 3.0 - t).sin() + fringe * (a * 11.0 + t * 0.7).sin())
                    + f.mid[i * stride] * amp * gain;
                (x + a.cos() * rad, y + a.sin() * rad, color + spread * (3.0 * a + t * 1.7).sin())
            }).collect();
            pts.push(pts[0]);
            pts
        }
        WaveKind::Line | WaveKind::Spectrum => {
            let n = if wave.kind == WaveKind::Line { 192 } else { 128 };
            let (sn, cs) = turn.sin_cos();
            let ratio = F_MAX / F_MIN;
            (0..n).map(|i| {
                let along = i as f32 / (n - 1) as f32;
                let bend = m * (wobble * (along * 4.7 - t).sin() + fringe * (along * 34.0 + t * 0.7).sin());
                let off = if wave.kind == WaveKind::Line {
                    f.mid[i * (TD / n)] * amp * gain
                } else {
                    let (f0, f1) = (F_MIN * ratio.powf(i as f32 / n as f32), F_MIN * ratio.powf((i + 1) as f32 / n as f32));
                    -level_shape(band_max(f, f0, f1), along) * amp
                } + bend;
                let d = (along - 0.5) * 2.0 * size;
                (x + cs * d - sn * off, y + sn * d + cs * off, color + spread * (along - 0.5) * 2.0)
            }).collect()
        }
        WaveKind::Xy => {
            const PTS: usize = 512;
            let stride = TD / PTS;
            let peak = (0..TD).step_by(4).fold(1e-4f32, |p, i| p.max((f.left[i] + f.right[i]).abs()).max((f.left[i] - f.right[i]).abs()));
            let k = size * (0.85 / (peak * FRAC_1_SQRT_2)).clamp(1.0, 10.0) * FRAC_1_SQRT_2;
            let (sn, cs) = turn.sin_cos();
            (0..PTS).map(|i| {
                let (l, r) = (f.left[i * stride], f.right[i * stride]);
                // Mono stands upright; left leans left.
                let (u, v) = ((r - l) * k, -(l + r) * k);
                let reach = u.hypot(v) / size.max(1.0);
                (x + cs * u - sn * v, y + sn * u + cs * v, color + spread * (reach - 0.5) * 2.0)
            }).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analyzer;
    use crate::canvas::unpack;

    fn light(c: &Canvas) -> u64 {
        c.px.iter().map(|p| unpack(*p).iter().map(|v| *v as u64).sum::<u64>()).sum()
    }

    fn music() -> Frame {
        let mut a = Analyzer::new();
        a.frame.peak = 0.3;
        a.frame.music.bands = [0.6, 0.4, 0.3];
        a.frame.music.pulse = 0.5;
        a.frame.spec.fill(0.5);
        for i in 0..TD {
            let s = (i as f32 * 0.05).sin() * 0.3;
            (a.frame.left[i], a.frame.right[i], a.frame.mid[i]) = (s, s * 0.6, s * 0.8);
        }
        a.frame
    }

    fn render(preset: &Preset, frames: usize, f: &Frame) -> Canvas {
        let mut mode = Tunnel::default();
        mode.configure(preset);
        let mut c = Canvas::new(96, 64);
        mode.reset(&mut c);
        for _ in 0..frames {
            mode.draw(&mut c, f, &Palette::new(1), 1.0 / 60.0);
        }
        c
    }

    #[test]
    fn every_built_in_recipe_draws_light_and_differs() {
        let f = music();
        let pictures: Vec<Canvas> = (0..Preset::builtin_count()).map(|i| render(&Preset::builtin(i), 90, &f)).collect();
        let dark = render(&Preset::default(), 0, &f);
        for (i, c) in pictures.iter().enumerate() {
            assert!(light(c) > light(&dark) * 2, "{} draws light", Preset::builtin(i).name);
            for d in &pictures[..i] {
                assert_ne!(c.px, d.px, "{} has its own look", Preset::builtin(i).name);
            }
        }
    }

    #[test]
    fn each_layer_kind_and_effect_changes_the_picture() {
        let f = music();
        let base = render(&Preset::default(), 40, &f);
        for extra in [
            "wave1 = line", "wave1 = spectrum", "wave1 = xy", "wave2 = ring\nwave2.size = 0.3",
            "shape1 = outline", "shape1 = filled", "shape1 = textured", "zoom_edge = 0.05", "swirl = 0.05",
            "ripple = 0.5", "center_dark = 1", "border = 1", "echo = 0.5\necho_mirror = xy", "invert = 1",
            "gamma = 2", "solarize = 1", "cx = 0.3",
        ] {
            let p = Preset::parse(extra).unwrap();
            assert_ne!(render(&p, 40, &f).px, base.px, "{extra}");
        }
    }

    /// A contact sheet of every built-in recipe in Tunnel, `seconds` into the test signal.
    fn recipe_sheet(seconds: usize) -> Canvas {
        use crate::analysis::FFT;
        use crate::testsignal::TestSignal;
        let (tw, th) = (480, 300);
        let n = Preset::builtin_count();
        let mut sheet = Canvas::new(tw * 2 + 18, (th + 6) * n.div_ceil(2) + 6);
        sheet.fill(crate::palette::CHASSIS);
        for i in 0..n {
            let mut mode = Tunnel::default();
            mode.configure(&Preset::builtin(i));
            let mut c = Canvas::new(tw, th);
            mode.reset(&mut c);
            let mut test = TestSignal::new(48_000.0);
            let mut a = Analyzer::new();
            let (mut l, mut r) = (vec![0.0; FFT], vec![0.0; FFT]);
            for _ in 0..60 * seconds {
                test.render(800, |x, y| {
                    l.push(x);
                    r.push(y);
                });
                l.drain(..800);
                r.drain(..800);
                a.process(&l, &r, 48_000.0, 1.0 / 60.0);
                mode.draw(&mut c, &a.frame, &Palette::new(i % 4), 1.0 / 60.0);
            }
            c.text(&Preset::builtin(i).name.to_uppercase(), 4, 4, [230, 198, 126]);
            sheet.blit(&c, (6 + (i % 2) * (tw + 6)) as i32, (6 + (i / 2) * (th + 6)) as i32);
        }
        sheet
    }

    /// Writes contact sheets for visual review; opt in explicitly.
    #[test]
    #[ignore]
    fn render_recipe_review() {
        std::fs::create_dir_all("../target/recipe-review").unwrap();
        for seconds in [11, 14] {
            recipe_sheet(seconds).save_bmp(&format!("../target/recipe-review/recipes-{seconds}s.bmp")).unwrap();
        }
    }

    #[test]
    fn layers_survive_any_size_and_silence() {
        let p = Preset::parse("shape1 = textured\nshape2 = filled\nshape2.size = 2\nwave2 = xy\nwave3 = spectrum\nborder = 2\necho = 1\necho_zoom = 0.25").unwrap();
        let mut mode = Tunnel::default();
        mode.configure(&p);
        let silent = Analyzer::new();
        for (w, h) in [(1, 1), (2, 3), (240, 20), (30, 200)] {
            let mut c = Canvas::new(w, h);
            mode.reset(&mut c);
            for _ in 0..3 {
                mode.draw(&mut c, &music(), &Palette::new(0), 1.0 / 60.0);
                mode.draw(&mut c, &silent.frame, &Palette::new(0), 0.0);
            }
        }
    }
}
