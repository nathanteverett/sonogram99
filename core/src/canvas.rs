//! A small software framebuffer with the few drawing primitives the visualizer needs.

use crate::font;

pub type Rgb = [u8; 3];

#[inline]
pub fn pack(c: Rgb) -> u32 {
    (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
}

#[inline]
pub fn unpack(p: u32) -> Rgb {
    [(p >> 16) as u8, (p >> 8) as u8, p as u8]
}

#[inline]
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t) as u8,
    ]
}

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Self {
        Canvas { w, h, px: vec![0; w * h] }
    }

    /// Resizes the buffer. Returns true when the size changed (the contents are then cleared).
    pub fn resize(&mut self, w: usize, h: usize) -> bool {
        if w == self.w && h == self.h {
            return false;
        }
        self.w = w;
        self.h = h;
        self.px = vec![0; w * h];
        true
    }

    pub fn fill(&mut self, c: Rgb) {
        self.px.fill(pack(c));
    }

    /// Moves every pixel toward `bg`, keeping `keep` (0..1) of its distance from it.
    /// Integer division rounds toward zero, so pixels always settle exactly on `bg`.
    pub fn fade(&mut self, bg: Rgb, keep: f32) {
        let k = (keep.clamp(0.0, 1.0) * 256.0) as i32;
        let (br, bgc, bb) = (bg[0] as i32, bg[1] as i32, bg[2] as i32);
        for p in self.px.iter_mut() {
            let r = ((*p >> 16) & 255) as i32;
            let g = ((*p >> 8) & 255) as i32;
            let b = (*p & 255) as i32;
            let r = br + (r - br) * k / 256;
            let g = bgc + (g - bgc) * k / 256;
            let b = bb + (b - bb) * k / 256;
            *p = (r as u32) << 16 | (g as u32) << 8 | b as u32;
        }
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, c: Rgb) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.px[y as usize * self.w + x as usize] = pack(c);
        }
    }

    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let i = y as usize * self.w + x as usize;
        self.px[i] = pack(mix(unpack(self.px[i]), c, a));
    }

    #[inline]
    pub fn add(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let i = y as usize * self.w + x as usize;
        let d = unpack(self.px[i]);
        self.px[i] = pack([
            (d[0] as f32 + c[0] as f32 * a).min(255.0) as u8,
            (d[1] as f32 + c[1] as f32 * a).min(255.0) as u8,
            (d[2] as f32 + c[2] as f32 * a).min(255.0) as u8,
        ]);
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = ((x + w).max(0) as usize).min(self.w);
        let y1 = ((y + h).max(0) as usize).min(self.h);
        let p = pack(c);
        for row in y0..y1 {
            if x0 < x1 {
                self.px[row * self.w + x0..row * self.w + x1].fill(p);
            }
        }
    }

    pub fn rect_blend(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb, a: f32) {
        for yy in y.max(0)..(y + h).min(self.h as i32) {
            for xx in x.max(0)..(x + w).min(self.w as i32) {
                self.blend(xx, yy, c, a);
            }
        }
    }

    /// A straight line. `width` above 1 stamps a square brush. `additive` lights pixels up
    /// instead of covering them, which suits glowing traces.
    #[allow(clippy::too_many_arguments)]
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgb, a: f32, width: i32, additive: bool) {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as i32;
        if steps > 20_000 {
            return;
        }
        let (sx, sy) = (dx / steps as f32, dy / steps as f32);
        let half = (width - 1) / 2;
        let (mut x, mut y) = (x0, y0);
        for _ in 0..=steps {
            let (px, py) = (x.round() as i32, y.round() as i32);
            for oy in -half..width - half {
                for ox in -half..width - half {
                    if additive {
                        self.add(px + ox, py + oy, c, a);
                    } else {
                        self.blend(px + ox, py + oy, c, a);
                    }
                }
            }
            x += sx;
            y += sy;
        }
    }

    pub fn polyline(&mut self, pts: &[(f32, f32)], c: Rgb, a: f32, width: i32, additive: bool) {
        for w in pts.windows(2) {
            self.line(w[0].0, w[0].1, w[1].0, w[1].1, c, a, width, additive);
        }
    }

    /// Copies `src` with its top-left corner at (x, y).
    pub fn blit(&mut self, src: &Canvas, x: i32, y: i32) {
        for sy in 0..src.h as i32 {
            let dy = y + sy;
            if dy < 0 || dy >= self.h as i32 {
                continue;
            }
            let sx0 = (-x).max(0);
            let sx1 = (src.w as i32).min(self.w as i32 - x);
            if sx0 >= sx1 {
                continue;
            }
            let s = sy as usize * src.w;
            let d = dy as usize * self.w;
            self.px[d + (x + sx0) as usize..d + (x + sx1) as usize]
                .copy_from_slice(&src.px[s + sx0 as usize..s + sx1 as usize]);
        }
    }

    /// Draws `s` in the 5x7 font, clipped to the columns `clip.0..clip.1`. Returns the end x.
    pub fn text_clipped(&mut self, s: &str, x: i32, y: i32, c: Rgb, clip: (i32, i32)) -> i32 {
        let mut cx = x;
        for ch in s.chars() {
            if cx > clip.1 {
                break;
            }
            if cx + font::GLYPH_W >= clip.0 {
                let g = font::glyph(ch);
                for (col, bits) in g.iter().enumerate() {
                    let px = cx + col as i32;
                    if px < clip.0 || px >= clip.1 {
                        continue;
                    }
                    for row in 0..font::GLYPH_H {
                        if bits >> row & 1 == 1 {
                            self.set(px, y + row, c);
                        }
                    }
                }
            }
            cx += font::ADVANCE;
        }
        cx
    }

    pub fn text(&mut self, s: &str, x: i32, y: i32, c: Rgb) -> i32 {
        self.text_clipped(s, x, y, c, (0, self.w as i32))
    }

    pub fn text_right(&mut self, s: &str, right: i32, y: i32, c: Rgb) {
        self.text(s, right - font::text_width(s), y, c);
    }

    pub fn text_center(&mut self, s: &str, cx: i32, y: i32, c: Rgb) {
        self.text(s, cx - font::text_width(s) / 2, y, c);
    }

    /// Text on a solid strip, so it stays readable over moving pictures.
    pub fn label(&mut self, s: &str, x: i32, y: i32, c: Rgb, bg: Rgb) {
        self.rect(x - 2, y - 2, font::text_width(s) + 4, font::GLYPH_H + 4, bg);
        self.text(s, x, y, c);
    }

    /// Writes the buffer as a 24-bit BMP file.
    pub fn save_bmp(&self, path: &str) -> std::io::Result<()> {
        std::fs::write(path, self.bmp())
    }

    /// The buffer as the bytes of a 24-bit BMP file.
    pub fn bmp(&self) -> Vec<u8> {
        let row = (self.w * 3 + 3) & !3;
        let size = 54 + row * self.h;
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(size as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(self.w as i32).to_le_bytes());
        out.extend_from_slice(&(self.h as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&[0u8; 24]);
        for y in (0..self.h).rev() {
            let start = out.len();
            for x in 0..self.w {
                let [r, g, b] = unpack(self.px[y * self.w + x]);
                out.extend_from_slice(&[b, g, r]);
            }
            out.resize(start + row, 0);
        }
        out
    }
}
