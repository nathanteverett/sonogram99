//! Effects on a finished picture: an echo copy, gamma, solarize and invert. They change
//! only what the viewer sees, never a mode's trails.

use crate::canvas::{pack, unpack, Canvas};
use crate::preset::{Effects, Inputs, Mirror};

/// The effects' values on one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub echo: f32,
    pub echo_zoom: f32,
    pub gamma: f32,
    pub solarize: f32,
    pub invert: f32,
}

impl Look {
    pub fn new(e: &Effects, inputs: &Inputs) -> Self {
        Look {
            echo: e.echo.eval(inputs),
            echo_zoom: e.echo_zoom.eval(inputs),
            gamma: e.gamma.eval(inputs),
            solarize: e.solarize.eval(inputs),
            invert: e.invert.eval(inputs),
        }
    }

    fn tones(&self) -> bool {
        (self.gamma - 1.0).abs() > 1e-3 || self.solarize > 1e-3 || self.invert > 1e-3
    }
}

/// Applies the effects to `c`. `copy` is scratch space that the caller keeps between frames.
pub fn apply(c: &mut Canvas, look: &Look, mirror: Mirror, copy: &mut Vec<u32>) {
    if (look.echo > 1e-3 || look.tones()) && crate::acceleration::run(crate::acceleration::Operation::Post {
        pixels: &mut c.px, w: c.w, h: c.h, look, mirror,
    }) { return; }
    if look.echo > 1e-3 {
        echo(c, look.echo, look.echo_zoom, mirror, copy);
    }
    if look.tones() {
        // Every effect here changes each channel the same way, so one table covers them.
        let table: Vec<u8> = (0..256).map(|v| {
            let mut v = (v as f32 / 255.0).powf(1.0 / look.gamma);
            v += (4.0 * v * (1.0 - v) - v) * look.solarize;
            v += (1.0 - 2.0 * v) * look.invert;
            (v.clamp(0.0, 1.0) * 255.0).round() as u8
        }).collect();
        for p in &mut c.px {
            let [r, g, b] = unpack(*p);
            *p = pack([table[r as usize], table[g as usize], table[b as usize]]);
        }
    }
    crate::acceleration::finish_software();
}

/// Blends `amount` of a copy of the picture, zoomed about the center and maybe flipped.
fn echo(c: &mut Canvas, amount: f32, zoom: f32, mirror: Mirror, copy: &mut Vec<u32>) {
    let (w, h) = (c.w, c.h);
    if w < 2 || h < 2 {
        return;
    }
    copy.clear();
    copy.extend_from_slice(&c.px);
    let (mid_x, mid_y) = (w as f32 * 0.5 - 0.5, h as f32 * 0.5 - 0.5);
    let flip_x = if matches!(mirror, Mirror::X | Mirror::Both) { -1.0 } else { 1.0 };
    let flip_y = if matches!(mirror, Mirror::Y | Mirror::Both) { -1.0 } else { 1.0 };
    let (kx, ky) = (flip_x / zoom, flip_y / zoom);
    let (wmax, hmax) = (w as f32 - 1.001, h as f32 - 1.001);
    let k = amount.clamp(0.0, 1.0);
    for y in 0..h {
        let sy = (mid_y + (y as f32 - mid_y) * ky).clamp(0.0, hmax);
        let (iy, fy) = (sy as usize, sy.fract());
        for x in 0..w {
            let sx = (mid_x + (x as f32 - mid_x) * kx).clamp(0.0, wmax);
            let (ix, fx) = (sx as usize, sx.fract());
            let i = iy * w + ix;
            let corners = [copy[i], copy[i + 1], copy[i + w], copy[i + w + 1]].map(unpack);
            let weights = [(1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy];
            let own = unpack(copy[y * w + x]);
            let mut out = [0u8; 3];
            for ch in 0..3 {
                let seen: f32 = corners.iter().zip(&weights).map(|(p, wt)| p[ch] as f32 * wt).sum();
                out[ch] = (own[ch] as f32 + (seen - own[ch] as f32) * k).round() as u8;
            }
            c.px[y * w + x] = pack(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look() -> Look {
        Look { echo: 0.0, echo_zoom: 1.0, gamma: 1.0, solarize: 0.0, invert: 0.0 }
    }

    #[test]
    fn plain_look_changes_nothing() {
        let mut c = Canvas::new(5, 4);
        for (i, p) in c.px.iter_mut().enumerate() {
            *p = pack([i as u8 * 10, 200 - i as u8 * 5, 77]);
        }
        let before = c.px.clone();
        apply(&mut c, &look(), Mirror::X, &mut Vec::new());
        assert_eq!(c.px, before);
    }

    #[test]
    fn tones_invert_brighten_and_solarize() {
        let mut c = Canvas::new(3, 1);
        c.px = vec![pack([0, 128, 255]), pack([64, 64, 64]), pack([255, 255, 255])];
        apply(&mut c, &Look { invert: 1.0, ..look() }, Mirror::None, &mut Vec::new());
        assert_eq!(unpack(c.px[0]), [255, 127, 0]);
        let mut g = Canvas::new(1, 1);
        g.px[0] = pack([64, 64, 64]);
        apply(&mut g, &Look { gamma: 2.0, ..look() }, Mirror::None, &mut Vec::new());
        assert!(unpack(g.px[0])[0] > 120, "Gamma above 1 brightens middle tones");
        let mut s = Canvas::new(1, 1);
        s.px[0] = pack([255, 255, 255]);
        apply(&mut s, &Look { solarize: 1.0, ..look() }, Mirror::None, &mut Vec::new());
        assert_eq!(unpack(s.px[0]), [0, 0, 0], "Full solarize turns white dark");
    }

    #[test]
    fn mirrored_echo_makes_the_picture_symmetric() {
        let mut c = Canvas::new(8, 2);
        c.px[0] = pack([200, 200, 200]);
        apply(&mut c, &Look { echo: 0.5, ..look() }, Mirror::X, &mut Vec::new());
        assert_eq!(unpack(c.px[0]), [100, 100, 100]);
        assert_eq!(unpack(c.px[7]), [100, 100, 100], "The flipped copy lands on the other side");
    }
}
