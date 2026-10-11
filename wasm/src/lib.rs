//! Sonograph 99 in a browser. The page writes the newest stereo samples into two buffers,
//! and the core renderer draws the same pictures as the desktop app into RGBA pixels for a
//! canvas.
//!
//! The interface is plain functions that take and return numbers and pointers, so the page
//! needs no generated glue code. All state lives in one `Web` value.

use std::cell::RefCell;

use sonograph_core::analysis::{Analyzer, FFT};
use sonograph_core::canvas::unpack;
use sonograph_core::modes;
use sonograph_core::palette::{Palette, SKINS};
use sonograph_core::preset::Preset;
use sonograph_core::show::Show;
use sonograph_core::testsignal::TestSignal;

/// The built-in test signal is 16 bars at 112 beats a minute.
const DEMO_SECONDS: f32 = 16.0 * 4.0 * 60.0 / 112.0;

struct Web {
    show: Show,
    analyzer: Analyzer,
    skin: usize,
    pal: Palette,
    /// The newest samples of each channel, oldest first. The page writes them.
    left: Vec<f32>,
    right: Vec<f32>,
    rgba: Vec<u8>,
    info: String,
    /// One loop of the test signal for the page to play: left channel, then right.
    demo: Vec<f32>,
    /// The test signal for a silent preview before any sound plays.
    preview: TestSignal,
}

impl Web {
    fn new(mode: usize, skin: usize, seed: u32) -> Self {
        Web {
            show: Show::new(mode, Preset::default(), false, seed),
            analyzer: Analyzer::new(),
            skin,
            pal: Palette::new(skin),
            left: vec![0.0; FFT],
            right: vec![0.0; FFT],
            rgba: Vec::new(),
            info: String::new(),
            demo: Vec::new(),
            preview: TestSignal::new(48_000.0),
        }
    }
}

thread_local! {
    static WEB: RefCell<Web> = RefCell::new(Web::new(5, 0, 1));
}

fn with<T>(f: impl FnOnce(&mut Web) -> T) -> T {
    WEB.with(|w| f(&mut w.borrow_mut()))
}

/// Starts again with a visual (0 to 10), a skin (0 to 13), and a seed for random choices.
#[no_mangle]
pub extern "C" fn start(mode: u32, skin: u32, seed: u32) {
    with(|w| *w = Web::new(mode as usize % modes::COUNT, skin as usize % SKINS.len(), seed));
}

/// Where the page writes the newest `buffer_len` samples of the left channel.
#[no_mangle]
pub extern "C" fn left_buffer() -> *mut f32 {
    with(|w| w.left.as_mut_ptr())
}

/// Where the page writes the newest `buffer_len` samples of the right channel.
#[no_mangle]
pub extern "C" fn right_buffer() -> *mut f32 {
    with(|w| w.right.as_mut_ptr())
}

#[no_mangle]
pub extern "C" fn buffer_len() -> u32 {
    FFT as u32
}

/// Fills the sample buffers from the test signal, for a silent preview.
#[no_mangle]
pub extern "C" fn preview(dt: f32) {
    with(|w| {
        let n = (dt.clamp(0.0, 0.1) * 48_000.0).round() as usize;
        let (left, right) = (&mut w.left, &mut w.right);
        let mut fresh = Vec::with_capacity(n);
        w.preview.render(n, |l, r| fresh.push((l, r)));
        let n = n.min(FFT);
        left.copy_within(n.., 0);
        right.copy_within(n.., 0);
        for (i, (l, r)) in fresh[fresh.len() - n..].iter().enumerate() {
            left[FFT - n + i] = *l;
            right[FFT - n + i] = *r;
        }
    });
}

/// Analyzes the sample buffers and draws one frame. Returns `w * h * 4` bytes of RGBA.
#[no_mangle]
pub extern "C" fn frame(w: u32, h: u32, sample_rate: f32, dt: f32) -> *const u8 {
    with(|web| {
        let (w, h) = ((w as usize).clamp(16, 4096), (h as usize).clamp(16, 4096));
        let dt = dt.clamp(0.0, 0.1);
        let rate = if sample_rate.is_finite() && sample_rate > 1000.0 { sample_rate } else { 48_000.0 };
        web.analyzer.process(&web.left, &web.right, rate, dt);
        web.show.resize(w, h);
        web.show.frame(&web.analyzer.frame, &web.pal, dt);
        web.rgba.clear();
        web.rgba.extend(web.show.composite.px.iter().flat_map(|p| {
            let [r, g, b] = unpack(*p);
            [r, g, b, 255]
        }));
        web.rgba.as_ptr()
    })
}

/// Changes to a visual, 0 to 10, with a transition.
#[no_mangle]
pub extern "C" fn set_mode(i: u32) {
    with(|w| w.show.set_mode(i as usize));
}

/// Select 0.25x, 0.5x, 1x, 1.5x, or 2x by index (0 to 4).
#[no_mangle]
pub extern "C" fn set_rate(index: u32) { with(|w| w.show.set_rate(index as usize)); }

#[no_mangle]
pub extern "C" fn step_rate(delta: i32) { with(|w| w.show.step_rate(delta)); }

#[no_mangle]
pub extern "C" fn rate_index() -> u32 { with(|w| w.show.rate_index() as u32) }

/// Moves forward (1) or back (-1) through the visuals.
#[no_mangle]
pub extern "C" fn step_mode(delta: i32) {
    with(|w| {
        let n = w.show.modes.len() as i32;
        let next = (w.show.mode as i32 + delta).rem_euclid(n) as usize;
        w.show.set_mode(next);
    });
}

/// Turns auto mode on or off. Returns 1 when it is on.
#[no_mangle]
pub extern "C" fn toggle_auto() -> u32 {
    with(|w| {
        w.show.auto.on = !w.show.auto.on;
        w.show.auto.restart();
        w.show.auto.on as u32
    })
}

/// The next recipe, in Tunnel and Orbs. Returns 0 in other visuals.
#[no_mangle]
pub extern "C" fn next_recipe() -> u32 {
    with(|w| {
        if !w.show.uses_recipe() {
            return 0;
        }
        w.show.next_recipe();
        1
    })
}

#[no_mangle]
pub extern "C" fn next_skin() {
    with(|w| {
        w.skin = (w.skin + 1) % SKINS.len();
        w.pal = Palette::new(w.skin);
    });
}

/// Shuffle (1) picks a new seed; reset (0) restores the recipe's seed. Returns 0 in
/// visuals without a seed.
#[no_mangle]
pub extern "C" fn reset_visual(shuffle: u32) -> u32 {
    with(|w| w.show.reset_visual(shuffle != 0) as u32)
}

/// The mouse over the picture, from 0 to 1 across and down. It pulls the visual only while
/// `down` is not 0, and only in Tunnel, Orbs, Kaleidoscope, Mandala and Gravity Waves.
#[no_mangle]
pub extern "C" fn pointer(x: f32, y: f32, down: u32) {
    with(|w| {
        let held = down != 0 && w.show.interactive() && x.is_finite() && y.is_finite();
        let mode = w.show.mode;
        w.show.controls[mode].pointer = held.then_some((x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)));
    });
}

/// Scroll over the picture: positive values make the visual larger.
#[no_mangle]
pub extern "C" fn wheel(amount: f32) {
    with(|w| {
        if w.show.interactive() {
            let mode = w.show.mode;
            w.show.controls[mode].scroll(amount);
        }
    });
}

/// Builds the status line, `|` between fields: visual, recipe (empty outside Tunnel and
/// Orbs), auto (0 or 1), skin, seed in hex, and whether the visual takes the mouse (0 or 1).
/// Returns its length in bytes; `info_ptr` gives its address.
#[no_mangle]
pub extern "C" fn info() -> u32 {
    with(|w| {
        let s = &w.show;
        let recipe = if s.uses_recipe() { s.preset.name.as_str() } else { "" };
        w.info = format!("{}|{}|{}|{}|{:08X}|{}", s.name(), recipe, s.auto.on as u8, SKINS[w.skin].name,
            s.controls[s.mode].seed, s.interactive() as u8);
        w.info.len() as u32
    })
}

#[no_mangle]
pub extern "C" fn info_ptr() -> *const u8 {
    with(|w| w.info.as_ptr())
}

/// Renders one loop of the test signal at `rate` for the page to play. Returns the left
/// channel followed by the right; `demo_len` gives the length of each.
#[no_mangle]
pub extern "C" fn demo(rate: f32) -> *const f32 {
    with(|w| {
        let rate = if rate.is_finite() && rate >= 8000.0 { rate.min(192_000.0) } else { 48_000.0 };
        let n = (DEMO_SECONDS * rate).round() as usize;
        let mut synth = TestSignal::new(rate);
        let mut right = Vec::with_capacity(n);
        w.demo = Vec::with_capacity(n * 2);
        let demo = &mut w.demo;
        synth.render(n, |l, r| {
            demo.push(l);
            right.push(r);
        });
        w.demo.extend(right);
        w.demo.as_ptr()
    })
}

#[no_mangle]
pub extern "C" fn demo_len() -> u32 {
    with(|w| (w.demo.len() / 2) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> String {
        let n = info() as usize;
        let bytes = unsafe { std::slice::from_raw_parts(info_ptr(), n) };
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[test]
    fn the_page_interface_draws_and_controls_the_show() {
        start(5, 2, 9);
        assert_eq!(buffer_len() as usize, FFT);
        assert_eq!(rate_index(), 2);
        set_rate(4);
        assert_eq!(rate_index(), 4);
        step_rate(-1);
        assert_eq!(rate_index(), 3);
        set_rate(u32::MAX);
        assert_eq!(rate_index(), 3);
        set_rate(2);
        let mut last = Vec::new();
        for _ in 0..30 {
            preview(1.0 / 60.0);
            let px = frame(80, 50, 48_000.0, 1.0 / 60.0);
            last = unsafe { std::slice::from_raw_parts(px, 80 * 50 * 4) }.to_vec();
        }
        assert!(last.chunks(4).all(|p| p[3] == 255));
        assert!(last.chunks(4).any(|p| p[0] > 30 || p[1] > 30 || p[2] > 30), "The preview draws light");
        assert_eq!(status(), "TUNNEL|Liquid|0|Ice|534F4E4F|1");
        assert_eq!(next_recipe(), 1);
        assert_eq!(toggle_auto(), 1);
        next_skin();
        assert!(status().starts_with("TUNNEL|Slow Drift|1|Orchid|"));
        pointer(0.2, 0.3, 1);
        wheel(2.0);
        assert_eq!(reset_visual(1), 1);
        step_mode(-1);
        assert!(status().starts_with("VECTOR||1|"));
        assert_eq!(next_recipe(), 0, "Vector has no recipe");
        set_mode(8);
        assert!(status().starts_with("WIREFRAME|"));
        step_mode(1);
        assert!(status().starts_with("MANDALA|") && status().ends_with("|1"), "Mandala takes the mouse");
        step_mode(1);
        assert!(status().starts_with("GRAVITY WAVES|") && status().ends_with("|1"), "Gravity Waves takes the mouse");
        set_mode((modes::COUNT - 1) as u32);
        step_mode(1);
        assert!(status().starts_with("BARS|"));
        let samples = demo(44_100.0);
        let n = demo_len() as usize;
        assert!((n as f32 - DEMO_SECONDS * 44_100.0).abs() < 2.0);
        let both = unsafe { std::slice::from_raw_parts(samples, n * 2) };
        assert!(both.iter().any(|v| v.abs() > 0.1) && both.iter().all(|v| v.abs() <= 1.0));
    }
}
