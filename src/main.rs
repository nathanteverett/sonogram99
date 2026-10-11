//! Sonograph 99: a late-90s style music visualizer.
//!
//! It captures what the computer is playing (WASAPI loopback on Windows), so it works with
//! Spotify, YouTube, Tidal, or any other app, and shows twelve visual modes.

mod audio;
mod catalogue;
mod fullscreen;
mod media;
mod miniaturize;

use std::io::Write;
use std::time::Instant;

use sonograph_core::{analysis, canvas, font, interaction, modes, offline, palette, preset, show, testsignal};

use minifb::{Key, KeyRepeat, MouseButton, MouseMode, ScaleMode, Window, WindowOptions};

use analysis::{Analyzer, FFT};
use audio::{Capture, Ring, Source};
use canvas::{mix, Canvas};
use catalogue::{Action, Catalogue, Hit};
use media::{Command, Media, NowPlaying, PlayState};
use palette::{Palette, BEVEL_HI, BEVEL_LO, CHASSIS, GOLD, LABEL_DIM, SKINS};
use testsignal::TestSignal;
use preset::Preset;
use show::Show;
use interaction::{Button, BAR_H};

const TITLE: &str = "Sonograph 99";
const HUD_H: usize = 15;
const HELP_H: usize = 11;
const RATE_H: usize = 17;
const HELP_SECONDS: f32 = 12.0;
const TOAST_SECONDS: f32 = 3.5;
const HELP: &str = "M CATALOGUE  F11 FULLSCREEN  F12 MINI  ESC BACK / QUIT";

const USAGE: &str = "\
Sonograph 99 - a late-90s style music visualizer

Usage: sonograph99 [options]

By default it shows whatever your computer is playing, so it works with Spotify,
YouTube, Tidal, and any other app.

Options:
  --mic             Listen to the default microphone or line-in instead
  --test            Show the built-in test signal instead of real audio
  --mode <1-12>     Start in a mode: 1 bars, 2 scope, 3 sonogram, 4 halo, 5 vector,
                    6 tunnel, 7 orbs, 8 kaleidoscope, 9 wireframe, 10 mandala,
                    11 gravity waves, 12 synthwave
  --skin <name>     classic, amber, ice, orchid, aurora, sunset, lagoon, candy,
                    ember, citrine, dracula, mocha, ocean, or neon
  --pixel <1-4>     Pixel size. Larger is chunkier and faster (default 2)
  --auto            Change mode on a beat, about every 15 seconds
  --fullscreen      Start in borderless fullscreen (Windows)
  --preset <name|file> A recipe for modes 6/7: Liquid, Slow Drift, Electric, Echo Chamber,
                    Glass Tiles, Ripple Pool, Solar Flare, or a recipe file
  --save-preset <file> Save the selected recipe to a new file and exit
  --snapshot <file> Render every mode from the test signal to a BMP file and exit
  --render <wav> <folder>  Draw a WAV file as numbered BMP frames in an empty folder,
                    then exit. Uses --mode, --preset, --skin and --auto
  --fps <1-240>     Frame rate for --render (default 60)
  --size <WxH>      Picture size for --render (default 1280x720)
  --probe           Listen for 3 seconds without opening a window, then report the
                    device, the signal level, and the media app that is playing
  -h, --help        Show this help

Keys:
  M catalogue   0-9 assigned hotkeys (default: visuals)   G gravity waves   W synthwave
  Left/Right next/previous visual
  A auto   R next recipe (Tunnel/Orbs)   S skin   I input   P pixel size   H help
  Down/Up slower/faster visual rate (0.25x to 2x)
  T toggle pulse (beat and attack accents)
  F11 fullscreen   Esc leave fullscreen / quit
  F12 miniaturize (Windows): drag Halo center; double/right-click to restore
  Catalogue: arrows select, 0-9 assign, Enter rebind, Space run, Del clear, D defaults, Esc close
  Z previous   X play   C pause   V stop   B next  (sent to Spotify or the
  current media app through Windows media controls)";

struct Options {
    source: Source,
    mode: usize,
    skin: usize,
    scale: usize,
    auto: bool,
    fullscreen: bool,
    snapshot: Option<String>,
    probe: bool,
    preset: Preset,
    save_preset: Option<String>,
    /// The WAV file and the folder for its frames.
    render: Option<(String, String)>,
    fps: f32,
    size: (usize, usize),
}

impl Default for Options {
    fn default() -> Self {
        Options { source: Source::Loopback, mode: 0, skin: 0, scale: 2, auto: false, fullscreen: false, snapshot: None, probe: false,
            preset: Preset::default(), save_preset: None, render: None, fps: 60.0, size: (1280, 720) }
    }
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value."));
        match a.as_str() {
            "--mic" => o.source = Source::Mic,
            "--test" => o.source = Source::Test,
            "--auto" => o.auto = true,
            "--fullscreen" => o.fullscreen = true,
            "--mode" => {
                let v = value("--mode")?;
                o.mode = match v.parse::<usize>() {
                    Ok(n @ 1..=modes::COUNT) => n - 1,
                    _ => return Err(format!("--mode takes a number from 1 to {}, not \"{v}\".", modes::COUNT)),
                };
            }
            "--skin" => {
                let v = value("--skin")?;
                o.skin = SKINS
                    .iter()
                    .position(|s| s.name.eq_ignore_ascii_case(&v))
                    .ok_or_else(|| format!("Unknown skin \"{v}\". Use {}.", SKINS.iter().map(|s| s.name.to_lowercase()).collect::<Vec<_>>().join(", ")))?;
            }
            "--pixel" => {
                let v = value("--pixel")?;
                o.scale = match v.parse::<usize>() {
                    Ok(n @ 1..=4) => n,
                    _ => return Err(format!("--pixel takes a number from 1 to 4, not \"{v}\".")),
                };
            }
            "--snapshot" => o.snapshot = Some(value("--snapshot")?),
            "--probe" => o.probe = true,
            "--preset" => o.preset = Preset::load(&value("--preset")?)?,
            "--save-preset" => o.save_preset = Some(value("--save-preset")?),
            "--render" => {
                let wav = value("--render")?;
                o.render = Some((wav, value("--render")?));
            }
            "--fps" => {
                let v = value("--fps")?;
                o.fps = v.parse::<f32>().ok().filter(|f| (1.0..=240.0).contains(f))
                    .ok_or_else(|| format!("--fps takes a number from 1 to 240, not \"{v}\"."))?;
            }
            "--size" => {
                let v = value("--size")?;
                o.size = v.split_once(['x', 'X']).and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .filter(|&(w, h): &(usize, usize)| (16..=7680).contains(&w) && (16..=4320).contains(&h))
                    .ok_or_else(|| format!("--size takes a size such as 1280x720 (16 to 7680 by 16 to 4320), not \"{v}\"."))?;
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("Unknown option \"{other}\". Run with --help to see the options.")),
        }
    }
    Ok(o)
}

struct App {
    fullscreen: bool,
    miniaturized: bool,
    catalogue: Catalogue,
    dragging: bool,
    pointer: Option<(i32, i32)>,
    screen: Canvas,
    /// The visuals: modes, transitions, auto mode, recipes, and mouse controls.
    show: Show,
    skin: usize,
    pal: Palette,
    scale: usize,
    source: Source,
    capture: Option<Capture>,
    capture_error: Option<String>,
    retry_t: f32,
    check_t: f32,
    test: TestSignal,
    test_ring: Ring,
    analyzer: Analyzer,
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    media: Option<Media>,
    now: Option<NowPlaying>,
    toast: Option<(String, Instant)>,
    show_help: bool,
    help_t: Option<f32>,
    marquee_text: String,
    marquee_x: f32,
    peak_hold: f32,
    peak_text: String,
    readout_t: f32,
    /// The input device's own sample rate, shown on the LCD.
    input_rate: u32,
}

impl App {
    fn new(o: &Options, with_media: bool) -> Self {
        let mut app = App {
            fullscreen: o.fullscreen,
            miniaturized: false,
            catalogue: Catalogue::default(),
            dragging: false,
            pointer: None,
            screen: Canvas::new(1, 1),
            // A fixed seed keeps snapshots and tests repeatable; the window reseeds it.
            show: Show::new(o.mode, o.preset.clone(), o.auto, 0x5EED_0099),
            skin: o.skin,
            pal: Palette::new(o.skin),
            scale: o.scale,
            source: o.source,
            capture: None,
            capture_error: None,
            retry_t: 0.0,
            check_t: 2.0,
            test: TestSignal::new(48_000.0),
            test_ring: Ring::new(),
            analyzer: Analyzer::new(),
            buf_l: vec![0.0; FFT],
            buf_r: vec![0.0; FFT],
            media: if with_media { Some(Media::start()) } else { None },
            now: None,
            toast: None,
            show_help: true,
            help_t: Some(HELP_SECONDS),
            marquee_text: String::new(),
            marquee_x: 0.0,
            peak_hold: 0.0,
            peak_text: "--".into(),
            readout_t: 1.0,
            input_rate: 48_000,
        };
        if app.source != Source::Test {
            app.open_capture();
        }
        app
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn open_capture(&mut self) {
        match Capture::start(self.source) {
            Ok(cap) => {
                let what = if self.source == Source::Loopback { "Listening to" } else { "Microphone:" };
                self.toast(format!("{what} {}", cap.device_name));
                self.capture = Some(cap);
                self.capture_error = None;
            }
            Err(e) => {
                if self.capture_error.as_deref() != Some(e.as_str()) {
                    self.toast(format!("{e} Showing the test signal."));
                }
                self.capture_error = Some(e);
                self.retry_t = 5.0;
            }
        }
    }

    fn set_mode(&mut self, i: usize) {
        if i % self.show.modes.len() == self.show.mode { return; }
        self.dragging = false;
        self.show.set_mode(i);
    }

    fn key(&mut self, k: Key) {
        if self.catalogue.open && self.catalogue.rebinding {
            if k == Key::Escape {
                self.catalogue.rebinding = false;
                self.catalogue.status = "Reassignment cancelled.".into();
            } else {
                self.catalogue.bind_key(k);
            }
            return;
        }
        if k == Key::M {
            self.catalogue.open = !self.catalogue.open;
            return;
        }
        // Keep window-mode shortcuts available while browsing, but let key capture
        // above consume it when assigning a shortcut.
        if k == Key::F11 && self.catalogue.bindings.entry(k) == catalogue::index("fullscreen") {
            self.execute_function(Key::F11);
            return;
        }
        if k == Key::F12 && self.catalogue.bindings.entry(k) == catalogue::index("miniaturize") {
            self.execute_function(Key::F12);
            return;
        }
        if self.catalogue.open {
            if let Some(slot) = catalogue::number_key(k) {
                self.catalogue.bind(slot);
            } else {
                match k {
                    Key::Escape => self.catalogue.open = false,
                    Key::Up => self.catalogue.navigate(-1),
                    Key::Down => self.catalogue.navigate(1),
                    Key::PageUp => self.catalogue.navigate(-5),
                    Key::PageDown => self.catalogue.navigate(5),
                    Key::Left => self.catalogue.tab(false),
                    Key::Right => self.catalogue.tab(true),
                    Key::Tab => self.catalogue.tab(self.catalogue.selected < catalogue::VISUALS),
                    Key::Enter => self.catalogue.start_rebind(),
                    Key::Space => self.catalogue_hit(Hit::Run),
                    Key::Delete => self.catalogue.clear(),
                    Key::D => self.catalogue.defaults(),
                    // Assign ordinary keys directly, just like the number slots.
                    // Enter capture remains available for keys used by this menu.
                    _ => self.catalogue.bind_key(k),
                }
            }
            return;
        }
        if k == Key::Escape && self.miniaturized {
            self.set_miniaturized(false);
            return;
        }
        if k == Key::Escape && self.fullscreen {
            self.set_fullscreen(false);
            return;
        }
        if let Some(entry) = self.catalogue.bindings.entry(k) {
            self.execute(catalogue::ENTRIES[entry].action);
        } else if let Some(slot) = catalogue::number_key(k) {
            self.toast(format!("Key {} is unassigned. Open M to assign it.", catalogue::slot_name(slot)));
        }
    }

    // Execute actions directly: re-entering key() here would follow reassigned
    // letters instead of the selected function, and could recurse indefinitely.
    fn execute_function(&mut self, k: Key) {
        let mode_key = match k {
            Key::Right | Key::Tab => Some(self.show.mode + 1),
            Key::Left => Some(self.show.mode + self.show.modes.len() - 1),
            _ => None,
        };
        if let Some(m) = mode_key {
            self.set_mode(m);
            return;
        }
        let cmd = match k {
            Key::Z => Some(Command::Prev),
            Key::X => Some(Command::Play),
            Key::C => Some(Command::PauseToggle),
            Key::V => Some(Command::Stop),
            Key::B => Some(Command::Next),
            _ => None,
        };
        if let Some(cmd) = cmd {
            let sent = self.media.as_ref().is_some_and(|m| m.send(cmd));
            if !sent {
                self.toast("Media keys need Windows media controls (Windows 10 or later).");
            } else if let Some(now) = &self.now {
                let app = if now.app.is_empty() { "Player".to_string() } else { now.app.clone() };
                self.toast(format!("{app}: {}", cmd.label()));
            } else {
                self.toast("No app is playing media right now.");
            }
            return;
        }
        match k {
            Key::F12 => self.set_miniaturized(!self.miniaturized),
            Key::F11 => self.set_fullscreen(!self.fullscreen),
            Key::A => {
                self.show.auto.on = !self.show.auto.on;
                self.show.auto.restart();
                self.toast(if self.show.auto.on { "Auto on: beat-aligned changes, paced by Rate (about 15 seconds at 1x)." } else { "Auto off." });
            }
            Key::T => {
                self.show.pulse = !self.show.pulse;
                self.toast(if self.show.pulse { "Pulse on." } else { "Pulse off: beat and attack accents disabled." });
            }
            Key::R => {
                if self.show.uses_recipe() {
                    self.show.next_recipe();
                    self.dragging = false;
                    self.toast(format!("Recipe: {}", self.show.preset.name));
                } else {
                    self.toast("Recipes are available in Tunnel and Orbs.");
                }
            }
            Key::S => {
                self.skin = (self.skin + 1) % SKINS.len();
                self.pal = Palette::new(self.skin);
                self.toast(format!("Skin: {}", SKINS[self.skin].name));
            }
            Key::I => {
                self.source = self.source.next();
                self.capture = None;
                self.capture_error = None;
                self.retry_t = 0.0;
                if self.source == Source::Test {
                    self.toast("Input: test signal");
                } else {
                    self.open_capture();
                }
            }
            Key::P => {
                self.scale = self.scale % 4 + 1;
                self.toast(format!("Pixel size {}", self.scale));
            }
            Key::H => {
                self.show_help = !self.show_help;
                self.help_t = None;
            }
            Key::Backspace => self.reset_visual(false),
            Key::N => self.reset_visual(true),
            Key::Down => self.show.step_rate(-1),
            Key::Up => self.show.step_rate(1),
            _ => {}
        }
    }

    fn execute(&mut self, action: Action) {
        match action {
            Action::Visual(mode) => self.set_mode(mode),
            Action::Function(key) => self.execute_function(key),
        }
    }

    fn catalogue_hit(&mut self, hit: Hit) {
        match hit {
            Hit::Close => {
                self.catalogue.open = false;
                self.catalogue.rebinding = false;
            }
            Hit::Tab(functions) => self.catalogue.tab(functions),
            Hit::Select(entry) => {
                self.catalogue.selected = entry;
                self.catalogue.status.clear();
                self.catalogue.rebinding = false;
            }
            Hit::Bind(slot) => self.catalogue.bind(slot),
            Hit::Rebind => self.catalogue.start_rebind(),
            Hit::Run => {
                let action = catalogue::ENTRIES[self.catalogue.selected].action;
                self.catalogue.open = false;
                self.catalogue.rebinding = false;
                self.execute(action);
            }
            Hit::Clear => self.catalogue.clear(),
            Hit::Defaults => self.catalogue.defaults(),
        }
    }

    fn set_fullscreen(&mut self, enabled: bool) {
        self.fullscreen = enabled;
        self.dragging = false;
        self.show.controls[self.show.mode].pointer = None;
    }

    fn set_miniaturized(&mut self, enabled: bool) {
        self.miniaturized = enabled;
        self.catalogue.open = false;
        self.dragging = false;
        self.pointer = None;
        self.show.controls[self.show.mode].pointer = None;
    }

    fn click(&mut self, w: usize, h: usize, x: i32, y: i32) {
        if self.miniaturized { return; }
        if self.catalogue.open {
            if let Some(hit) = self.catalogue.hit(w, h, x, y) { self.catalogue_hit(hit); }
        } else if self.fullscreen {
            // The hidden header and footer must not retain clickable controls.
            return;
        } else if (14..104).contains(&x) && (0..HUD_H as i32).contains(&y) {
            self.catalogue.open = true;
        } else if (104..191).contains(&x) && (0..HUD_H as i32).contains(&y) {
            self.execute_function(Key::F11);
        } else if (194..194 + Self::mini_button_width(w)).contains(&x) && (0..HUD_H as i32).contains(&y) {
            self.execute_function(Key::F12);
        } else if (self.rate_y(h)..self.bar_y(h)).contains(&y) {
            if (w as i32 - 64..w as i32 - 3).contains(&x) {
                self.execute_function(Key::T);
                return;
            }
            for i in 0..show::RATES.len() {
                let (bx, bw) = Self::rate_button(w, i);
                if (bx..bx + bw).contains(&x) { self.show.set_rate(i); }
            }
        } else if self.interactive() && (self.bar_y(h)..self.bar_y(h) + BAR_H as i32).contains(&y) {
            for (button, bx, bw, _) in interaction::buttons(w) {
                if (bx..bx + bw).contains(&x) { self.reset_visual(button == Button::Shuffle); }
            }
        }
    }

    fn interactive(&self) -> bool { self.show.interactive() }

    fn mini_button_width(w: usize) -> i32 { if w >= 374 { 87 } else { 43 } }

    fn visual_top(&self) -> i32 { if self.fullscreen { 0 } else { HUD_H as i32 } }

    fn bar_y(&self, h: usize) -> i32 {
        if self.fullscreen { h as i32 }
        else { h.saturating_sub(BAR_H + if self.show_help { HELP_H } else { 0 }) as i32 }
    }

    fn rate_y(&self, h: usize) -> i32 {
        if self.fullscreen { h as i32 } else { self.bar_y(h) - RATE_H as i32 }
    }

    fn reset_visual(&mut self, shuffle: bool) {
        if !self.show.reset_visual(shuffle) { return; }
        self.dragging = false;
        self.toast(if shuffle { "New seed. Reset returns to the recipe's original seed." }
            else { "Original seed restored. Mouse adjustments and trails cleared." });
    }

    fn mouse(&mut self, w: usize, h: usize, pointer: Option<(i32, i32)>, down: bool, pressed: bool, wheel: f32) {
        self.pointer = pointer;
        let top = self.visual_top();
        let inside = pointer.is_some_and(|(x, y)| x >= 0 && x < w as i32 && y >= top && y < self.rate_y(h));
        if !down || !inside || self.catalogue.open || !self.interactive() { self.dragging = false; }
        else if pressed { self.dragging = true; }
        let mode = self.show.mode;
        self.show.controls[mode].pointer = if self.dragging {
            pointer.map(|(x, y)| (x as f32 / w.max(1) as f32,
                (y - top) as f32 / (self.rate_y(h) - top).max(1) as f32))
        } else { None };
        if inside && self.interactive() && !self.catalogue.open { self.show.controls[mode].scroll(wheel); }
    }

    fn rate_button(w: usize, index: usize) -> (i32, i32) {
        if w < 320 { (3 + index as i32 * 34, 32) }
        else { (34 + index as i32 * 39, 37) }
    }

    fn draw_controls(&mut self) {
        let (w, h) = (self.screen.w, self.screen.h);
        let ry = self.rate_y(h);
        self.screen.rect(0, ry, w as i32, RATE_H as i32, CHASSIS);
        self.screen.rect(0, ry, w as i32, 1, BEVEL_HI);
        if w >= 320 { self.screen.text("RATE", 5, ry + 5, LABEL_DIM); }
        for (i, &(_, label)) in show::RATES.iter().enumerate() {
            let (x, bw) = Self::rate_button(w, i);
            let selected = self.show.rate_index() == i;
            let hover = self.pointer.is_some_and(|(mx, my)| (x..x + bw).contains(&mx) && (ry..ry + RATE_H as i32).contains(&my));
            self.screen.rect(x, ry + 2, bw, 13, if hover { BEVEL_HI } else { BEVEL_LO });
            self.screen.text_center(label, x + bw / 2, ry + 5, if selected { GOLD } else { self.pal.skin.ink });
            if selected { self.screen.rect(x + 2, ry + 13, bw - 4, 1, GOLD); }
        }
        if w >= 460 { self.screen.text("DOWN SLOWER  UP FASTER", 237, ry + 5, LABEL_DIM); }
        let px = w as i32 - 64;
        let hover = self.pointer.is_some_and(|(mx, my)| (px..px + 61).contains(&mx) && (ry..ry + RATE_H as i32).contains(&my));
        self.screen.rect(px, ry + 2, 61, 13, if hover { BEVEL_HI } else { BEVEL_LO });
        self.screen.text_center(if self.show.pulse { "PULSE ON" } else { "PULSE OFF" }, px + 30, ry + 5,
            if self.show.pulse { GOLD } else { LABEL_DIM });
        let y = self.bar_y(h);
        self.screen.rect(0, y, w as i32, BAR_H as i32, CHASSIS);
        self.screen.rect(0, y, w as i32, 1, BEVEL_HI);
        if !self.interactive() {
            self.screen.text("M CATALOGUE", 5, y + 5, LABEL_DIM);
            return;
        }
        let hint = if w >= 460 { "DRAG TO STIR  WHEEL SIZE" } else { "DRAG / WHEEL" };
        self.screen.text(hint, 5, y + 5, LABEL_DIM);
        if w >= 460 {
            self.screen.text(&format!("SEED {:08X}", self.show.controls[self.show.mode].seed), 152, y + 5, GOLD);
        }
        for (_, x, bw, label) in interaction::buttons(w) {
            let hover = self.pointer.is_some_and(|(mx, my)| (x..x + bw).contains(&mx) && (y..y + BAR_H as i32).contains(&my));
            self.screen.rect(x, y + 2, bw, 13, if hover { BEVEL_HI } else { BEVEL_LO });
            self.screen.text_center(label, x + bw / 2, y + 5, self.pal.skin.ink);
        }
    }

    fn pump_audio(&mut self, dt: f32) {
        if self.source != Source::Test {
            self.check_t -= dt;
            if self.check_t <= 0.0 {
                self.check_t = 2.0;
                if self.capture.as_ref().is_some_and(|c| c.needs_restart(self.source)) {
                    self.capture = None;
                    self.retry_t = 0.0;
                }
            }
            if self.capture.is_none() {
                self.retry_t -= dt;
                if self.retry_t <= 0.0 {
                    self.open_capture();
                }
            }
        }
        let sample_rate = match (&self.capture, self.source) {
            (Some(cap), s) if s != Source::Test => {
                if let Ok(ring) = cap.ring.lock() {
                    ring.latest(&mut self.buf_l, &mut self.buf_r);
                }
                self.input_rate = cap.device_rate;
                cap.sample_rate as f32
            }
            _ => {
                let n = (dt * self.test.sample_rate()).round() as usize;
                let ring = &mut self.test_ring;
                self.test.render(n, |l, r| ring.push(l, r));
                ring.touch();
                ring.latest(&mut self.buf_l, &mut self.buf_r);
                self.input_rate = self.test.sample_rate() as u32;
                self.test.sample_rate()
            }
        };
        self.analyzer.process(&self.buf_l, &self.buf_r, sample_rate, dt);
    }

    fn using_test(&self) -> bool {
        self.source == Source::Test || self.capture.is_none()
    }

    fn frame(&mut self, w: usize, h: usize, dt: f32) {
        if let Some(t) = self.help_t.as_mut().filter(|_| !self.fullscreen) {
            *t -= dt;
            if *t <= 0.0 {
                self.show_help = false;
                self.help_t = None;
            }
        }
        self.screen.resize(w, h);
        let top = self.visual_top();
        let vh = (self.rate_y(h) - top).max(1) as usize;
        self.show.resize(w, vh);
        self.pump_audio(dt);
        if let Some(m) = &self.media {
            self.now = m.now();
        }
        if self.show.frame(&self.analyzer.frame, &self.pal, dt) {
            self.dragging = false;
        }
        self.screen.fill(CHASSIS);
        self.screen.blit(&self.show.composite, 0, top);
        if !self.fullscreen {
            self.draw_hud(dt);
            self.draw_controls();
        }
        if self.show_help && !self.fullscreen {
            let y = h.saturating_sub(HELP_H) as i32;
            self.screen.rect(0, y, w as i32, 1, BEVEL_HI);
            self.screen.text(HELP, 4, y + 3, LABEL_DIM);
        }
        if self.catalogue.open {
            self.catalogue.draw(&mut self.screen, self.pal.skin.ink);
        }
    }

    fn marquee_source(&self) -> String {
        if let Some((msg, at)) = &self.toast {
            if at.elapsed().as_secs_f32() < TOAST_SECONDS {
                return msg.clone();
            }
        }
        if let Some(now) = self.now.as_ref().filter(|n| !n.title.is_empty()) {
            let song = if now.artist.is_empty() { now.title.clone() } else { format!("{} - {}", now.artist, now.title) };
            return if now.app.is_empty() { song } else { format!("{song}  [{}]", now.app) };
        }
        match (&self.capture, self.source) {
            (Some(cap), Source::Loopback) => format!("Listening to {}. Play something in Spotify, a browser, or any app.", cap.device_name),
            (Some(cap), Source::Mic) => format!("Listening to {}", cap.device_name),
            _ => "Test signal - Am F C G at 112 BPM".to_string(),
        }
    }

    fn draw_hud(&mut self, dt: f32) {
        let w = self.screen.w as i32;
        let ink = self.pal.skin.ink;
        let background = self.pal.skin.screen;
        let f = &self.analyzer.frame;

        // Readouts refresh a few times a second, like a real LCD.
        self.peak_hold = f.peak.max(self.peak_hold * 0.25f32.powf(dt));
        self.readout_t += dt;
        if self.readout_t > 0.15 {
            self.readout_t = 0.0;
            self.peak_text = if self.peak_hold < 1e-5 { "-INF".into() } else { format!("{:.1}", 20.0 * self.peak_hold.log10()) };
        }
        let sr = self.input_rate;
        let khz = if sr.is_multiple_of(1000) { format!("{} KHZ", sr / 1000) } else { format!("{:.1} KHZ", sr as f32 / 1000.0) };
        let state = match self.now.as_ref().map(|n| n.state) {
            Some(PlayState::Playing) => PlayState::Playing,
            Some(PlayState::Paused) => PlayState::Paused,
            _ if f.peak > 1e-3 => PlayState::Playing,
            _ => PlayState::Stopped,
        };
        let src = if self.using_test() { "TEST" } else { self.source.label() };
        let mut readouts = vec![
            self.show.name().to_string(),
            src.to_string(),
            format!("PEAK {} DB", self.peak_text),
            khz,
        ];
        if self.show.uses_recipe() {
            readouts.insert(1, self.show.preset.name.to_uppercase());
        }
        if self.show.auto.on {
            readouts.insert(0, "AUTO".into());
        }
        let text = font::to_lcd(&self.marquee_source());
        if text != self.marquee_text {
            self.marquee_text = text;
            self.marquee_x = 0.0;
        }

        let s = &mut self.screen;
        s.rect(0, 0, w, 1, BEVEL_HI);
        s.rect(0, HUD_H as i32 - 1, w, 1, BEVEL_LO);
        status_glyph(s, 5, 4, state, ink, background);
        lcd_box(s, 14, 2, 87, 11, background);
        s.text("M Catalogue", 18, 4, GOLD);
        lcd_box(s, 104, 2, 87, 11, background);
        s.text(if self.fullscreen { "Exit full" } else { "Fullscreen" }, 108, 4, GOLD);
        let mini_width = Self::mini_button_width(w as usize);
        let title_x = 197 + mini_width;
        lcd_box(s, 194, 2, mini_width, 11, background);
        s.text(if mini_width == 87 { "Miniaturize" } else { "Mini" }, 198, 4, GOLD);

        // Readouts fill from the right while the marquee keeps at least 90 pixels.
        let mut right = w - 3;
        for r in &readouts {
            let bw = font::text_width(r) + 8;
            if right - bw < title_x + 90 {
                break;
            }
            lcd_box(s, right - bw, 2, bw, 11, background);
            s.text(r, right - bw + 4, 4, ink);
            right -= bw + 3;
        }

        // Scrolling title, as on the old players.
        let (bx, bw) = (title_x, right - title_x);
        if bw < 8 { return; }
        lcd_box(s, bx, 2, bw, 11, background);
        let clip = (bx + 3, bx + bw - 3);
        let tw = font::text_width(&self.marquee_text);
        if tw <= clip.1 - clip.0 {
            s.text_clipped(&self.marquee_text, clip.0, 4, ink, clip);
        } else {
            let looped = format!("{}  ***  ", self.marquee_text);
            let period = font::text_width(&looped) + 1;
            self.marquee_x = (self.marquee_x + dt * 24.0) % period as f32;
            let x = clip.0 - self.marquee_x as i32;
            s.text_clipped(&looped, x, 4, ink, clip);
            s.text_clipped(&looped, x + period, 4, ink, clip);
        }
    }
}

fn lcd_box(s: &mut Canvas, x: i32, y: i32, w: i32, h: i32, background: canvas::Rgb) {
    s.rect(x, y, w, h, background);
    s.rect(x, y, w, 1, BEVEL_LO);
    s.rect(x, y, 1, h, BEVEL_LO);
    s.rect(x, y + h - 1, w, 1, BEVEL_HI);
    s.rect(x + w - 1, y, 1, h, BEVEL_HI);
}

fn status_glyph(s: &mut Canvas, x: i32, y: i32, state: PlayState, ink: canvas::Rgb, background: canvas::Rgb) {
    match state {
        PlayState::Playing => {
            for r in 0..7 {
                s.rect(x, y + r, 2 + r.min(6 - r), 1, ink);
            }
        }
        PlayState::Paused => {
            s.rect(x, y, 2, 7, ink);
            s.rect(x + 4, y, 2, 7, ink);
        }
        _ => s.rect(x, y + 1, 6, 6, mix(background, ink, 0.6)),
    }
}

/// Renders every mode from the test signal and writes a two-column contact sheet.
fn snapshot(path: &str, o: &Options) -> Result<(), String> {
    let (tw, th, gap) = (480usize, 300usize, 6usize);
    let count = modes::all().len();
    let rows = count.div_ceil(2);
    let mut sheet = Canvas::new(tw * 2 + gap * 3, th * rows + gap * (rows + 1) + 14);
    sheet.fill(CHASSIS);
    sheet.text("SONOGRAPH 99", gap as i32, 5, GOLD);
    for m in 0..count {
        let opts = Options { source: Source::Test, mode: m, skin: o.skin, scale: 1, auto: false, snapshot: None, probe: false, preset: o.preset.clone(), save_preset: None, ..Options::default() };
        let mut app = App::new(&opts, false);
        // Six seconds in: past the intro, with the drums and bass playing.
        for _ in 0..360 {
            app.frame(tw, th, 1.0 / 60.0);
        }
        let x = gap + (m % 2) * (tw + gap);
        let y = 14 + gap + (m / 2) * (th + gap);
        sheet.blit(&app.screen, x as i32, y as i32);
    }
    sheet.save_bmp(path).map_err(|e| format!("Could not write {path}: {e}"))
}

/// Draws a WAV file frame by frame into a folder of BMP files.
fn render(o: &Options, wav: &str, dir: &str) -> Result<(), String> {
    let bytes = std::fs::read(wav).map_err(|e| format!("Could not read {wav}: {e}"))?;
    let audio = offline::read_wav(&bytes).map_err(|e| format!("{wav}: {e}"))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not create the folder {dir}: {e}"))?;
    let job = offline::Job { fps: o.fps, width: o.size.0, height: o.size.1, mode: o.mode,
        preset: o.preset.clone(), skin: o.skin, auto: o.auto };
    let total = job.frames(&audio);
    println!("Drawing {total} frames ({:.1} seconds at {} fps, {}x{}) into {dir}", audio.seconds(), o.fps, o.size.0, o.size.1);
    let start = Instant::now();
    let every = (o.fps as usize * 10).max(1);
    let n = offline::render(&audio, &job, |k, c| {
        let path = std::path::Path::new(dir).join(format!("frame-{k:06}.bmp"));
        // Never overwrite an earlier render.
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
            .map_err(|e| format!("Could not create {}: {e}. Use an empty folder.", path.display()))?;
        file.write_all(&c.bmp()).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
        if (k + 1) % every == 0 {
            println!("  {} of {total} frames", k + 1);
        }
        Ok(())
    })?;
    println!("Wrote {n} frames in {:.1} seconds. To make a video with ffmpeg:", start.elapsed().as_secs_f32());
    println!("  ffmpeg -framerate {} -i \"{dir}/frame-%06d.bmp\" -i \"{wav}\" -c:v libx264 -pix_fmt yuv420p -shortest video.mp4", o.fps);
    Ok(())
}

/// Checks capture and the media session from the command line, without a window.
fn probe(o: &Options) -> Result<(), String> {
    let source = if o.source == Source::Test { Source::Loopback } else { o.source };
    let cap = Capture::start(source)?;
    println!(
        "Input:   {} ({}, {} Hz, analyzed at {} Hz)",
        cap.device_name,
        source.label().to_lowercase(),
        cap.device_rate,
        cap.sample_rate
    );
    let media = Media::start();
    std::thread::sleep(std::time::Duration::from_secs(3));
    let (frames, peak) = cap.ring.lock().map(|r| (r.total, r.peak)).unwrap_or((0, 0.0));
    let level = if peak < 1e-6 { "silent".to_string() } else { format!("peak {:.1} dBFS", 20.0 * peak.log10()) };
    println!("Signal:  {frames} frames in 3 seconds, {level}");
    if frames == 0 && source == Source::Loopback {
        println!("         Windows sends loopback audio only while something is playing.");
    }
    match media.now() {
        Some(n) => println!("Media:   {} - {} - {} ({:?})", n.app, n.artist, n.title, n.state),
        None => println!("Media:   no app has an active media session"),
    }
    Ok(())
}

fn run(o: &Options) -> Result<(), String> {
    let mut window = Window::new(
        TITLE,
        1280,
        720,
        WindowOptions { resize: true, scale_mode: ScaleMode::Stretch, ..WindowOptions::default() },
    )
    .map_err(|e| format!("Could not open a window: {e}"))?;
    window.set_target_fps(60);
    let mut app = App::new(o, true);
    let mut fullscreen = fullscreen::Fullscreen::default();
    fullscreen.set(&window, app.fullscreen)?;
    let mut mini = miniaturize::Miniaturize::default();
    let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    app.show.reseed(clock);
    app.catalogue.load();
    let mut title = String::new();
    let mut last = Instant::now();
    let mut mouse_down = false;
    'running: while window.is_open() {
        let now = Instant::now();
        let dt = (now - last).as_secs_f32().clamp(0.001, 0.1);
        last = now;
        for k in window.get_keys_pressed(KeyRepeat::No) {
            if k == Key::Escape && !app.catalogue.open && !app.fullscreen && !app.miniaturized { break 'running; }
            app.key(k);
        }
        if app.fullscreen != fullscreen.active() {
            if let Err(error) = fullscreen.set(&window, app.fullscreen) {
                app.fullscreen = fullscreen.active();
                app.toast(error);
            }
        }
        if mini.restore_requested() { app.set_miniaturized(false); }
        if app.miniaturized != mini.active() {
            if let Err(error) = mini.set(&window, app.miniaturized) {
                app.set_miniaturized(mini.active());
                app.toast(error);
            }
            mouse_down = false;
        }
        if mini.active() {
            app.pump_audio(dt);
            if let Some(m) = &app.media { app.now = m.now(); }
            if let Err(error) = mini.frame(&app.analyzer.frame, &app.pal, dt, app.show.rate_index(), app.show.pulse) {
                let _ = mini.set(&window, false);
                app.set_miniaturized(false);
                app.toast(error);
            }
            // Keep dispatching native overlay input and audio without drawing the hidden player.
            window.update();
            continue;
        }
        let (ww, wh) = window.get_size();
        let w = (ww / app.scale).max(240);
        let h = (wh / app.scale).max(120);
        let down = window.get_mouse_down(MouseButton::Left);
        let pointer = if ww > 0 && wh > 0 {
            window.get_unscaled_mouse_pos(MouseMode::Discard).map(|(x, y)|
                ((x * w as f32 / ww as f32) as i32, (y * h as f32 / wh as f32) as i32))
        } else { None };
        let wheel = window.get_scroll_wheel().map_or(0.0, |(_, y)| y);
        if down && !mouse_down {
            if let Some((x, y)) = pointer { app.click(w, h, x, y); }
        }
        app.mouse(w, h, pointer, down, down && !mouse_down, wheel);
        mouse_down = down;
        if app.catalogue.open && wheel != 0.0 {
            app.catalogue.navigate(if wheel > 0.0 { -1 } else { 1 });
        }
        app.frame(w, h, dt);
        window
            .update_with_buffer(&app.screen.px, w, h)
            .map_err(|e| format!("Could not draw the window: {e}"))?;
        // Put the current song in the taskbar too.
        let want = match app.now.as_ref().filter(|n| !n.title.is_empty()) {
            Some(n) if !n.artist.is_empty() => format!("{} - {} - {TITLE}", n.artist, n.title),
            Some(n) => format!("{} - {TITLE}", n.title),
            None => TITLE.to_string(),
        };
        if want != title {
            window.set_title(&want);
            title = want;
        }
    }
    Ok(())
}

fn main() {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if let Some(path) = &opts.save_preset {
        match opts.preset.save(path) {
            Ok(()) => println!("Wrote preset {path}"),
            Err(e) => { eprintln!("{e}"); std::process::exit(1); }
        }
        return;
    }
    let result = match &opts.snapshot {
        _ if opts.render.is_some() => {
            let (wav, dir) = opts.render.as_ref().unwrap();
            render(&opts, wav, dir)
        }
        Some(path) => snapshot(path, &opts).map(|_| println!("Wrote {path}")),
        None if opts.probe => probe(&opts),
        None => run(&opts),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonograph_core::interaction::Controls;

    #[test]
    fn pulse_toggle_routes_clicks_catalogue_and_saved_reassignments() {
        let mut app = App::new(&test_options(7), false);
        app.key(Key::T);
        assert!(!app.show.pulse);
        for (w, h) in [(240, 120), (640, 360)] {
            let rate = app.show.rate_index();
            app.click(w, h, w as i32 - 30, app.rate_y(h) + 5);
            assert!(app.show.pulse);
            assert_eq!(app.show.rate_index(), rate);
            app.key(Key::T);
        }
        app.key(Key::M);
        app.catalogue.selected = catalogue::index("pulse").unwrap();
        let path = std::env::temp_dir().join(format!("sonograph-pulse-hotkeys-{}.conf", std::process::id()));
        app.catalogue.path = Some(path.clone());
        app.key(Key::Enter);
        app.key(Key::F2);
        assert!(!app.show.pulse, "Binding a key must not run the toggle");
        app.catalogue.bindings = catalogue::Bindings::load(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        app.key(Key::Space);
        assert!(app.show.pulse && !app.catalogue.open);
        app.key(Key::T);
        assert!(app.show.pulse, "Old shortcut must stop working");
        app.key(Key::F2);
        assert!(!app.show.pulse);
        app.set_fullscreen(true);
        app.click(640, 360, 610, 331);
        assert!(!app.show.pulse, "Hidden controls must not receive clicks");
        app.key(Key::F2);
        assert!(app.show.pulse, "Assigned hotkey works in fullscreen");
    }

    #[test]
    fn rate_selector_routes_clicks_keys_and_leaves_audio_at_normal_speed() {
        let mut app = App::new(&test_options(7), false);
        let mut reference = App::new(&test_options(7), false);
        app.click(240, 120, 150, app.rate_y(120) + 5);
        assert_eq!(app.show.rate_index(), 4);
        app.mouse(240, 120, Some((150, app.rate_y(120) + 5)), true, true, 2.0);
        assert!(!app.dragging);
        assert_eq!(app.show.controls[7].scale, 1.0);
        for _ in 0..5 {
            app.frame(240, 120, 1.0 / 60.0);
            reference.frame(240, 120, 1.0 / 60.0);
            assert_eq!(app.buf_l, reference.buf_l);
            assert_eq!(app.buf_r, reference.buf_r);
        }
        app.key(Key::Down);
        assert_eq!(app.show.rate_index(), 3);
        app.key(Key::M);
        app.key(Key::Up);
        assert_eq!(app.show.rate_index(), 3, "Catalogue navigation does not change rate");
        app.key(Key::Escape);
        app.set_fullscreen(true);
        app.click(240, 120, 205, 80);
        assert_eq!(app.show.rate_index(), 3, "Hidden rate controls do not receive clicks");
        app.key(Key::Up);
        assert_eq!(app.show.rate_index(), 4);
    }

    #[test]
    #[ignore]
    fn render_rate_review() {
        std::fs::create_dir_all("target/rate-review").unwrap();
        for (w, h) in [(240, 120), (640, 360)] {
            let mut app = App::new(&test_options(8), false);
            app.show.set_rate(1);
            for _ in 0..60 { app.frame(w, h, 1.0 / 60.0); }
            app.screen.save_bmp(&format!("target/rate-review/{w}.bmp")).unwrap();
        }
    }

    #[test]
    fn interactive_modes_restore_exact_seeded_render_after_reset() {
        let mut audio = Analyzer::new();
        audio.frame.peak = 0.3;
        audio.frame.bass = 0.6;
        audio.frame.spec.fill(0.5);
        audio.frame.music.bands = [0.6, 0.4, 0.3];
        for index in [5, 6, 7, 9, 10] {
            let mut modes = modes::all();
            let mode = &mut modes[index];
            let mut c = Canvas::new(120, 90);
            let pal = Palette::new(2);
            let mut controls = Controls::new(12345);
            let draw = |mode: &mut Box<dyn modes::Mode>, c: &mut Canvas, controls| {
                mode.interact(controls);
                for _ in 0..40 { mode.draw(c, &audio.frame, &pal, 1.0 / 60.0); }
            };
            mode.reset(&mut c);
            draw(mode, &mut c, controls);
            let baseline = c.px.clone();
            mode.reset(&mut c);
            controls.pointer = Some((0.85, 0.25));
            draw(mode, &mut c, controls);
            assert_ne!(baseline, c.px, "Drag must change mode {index}");
            mode.reset(&mut c);
            controls.pointer = None;
            controls.scroll(3.0);
            draw(mode, &mut c, controls);
            assert_ne!(baseline, c.px, "Wheel must change mode {index}");
            controls.reset();
            controls.shuffle();
            mode.reset(&mut c);
            draw(mode, &mut c, controls);
            assert_ne!(baseline, c.px, "Seed must change mode {index}");
            controls.reset();
            mode.reset(&mut c);
            draw(mode, &mut c, controls);
            assert_eq!(baseline, c.px, "Reset must reproduce mode {index} with the same audio");
        }
    }

    #[test]
    fn pointer_routes_visuals_and_buttons_without_leaking_through_catalogue() {
        let mut app = App::new(&test_options(7), false);
        app.frame(240, 120, 1.0 / 60.0);
        app.mouse(240, 120, Some((190, 45)), true, true, 2.0);
        assert!(app.show.controls[7].pointer.is_some());
        assert!(app.show.controls[7].scale > 1.0);
        app.catalogue.open = true;
        let scale = app.show.controls[7].scale;
        app.mouse(240, 120, Some((190, 45)), true, false, 2.0);
        assert!(app.show.controls[7].pointer.is_none());
        assert_eq!(app.show.controls[7].scale, scale);
        app.catalogue.open = false;
        let seed = app.show.controls[7].seed;
        let y = app.bar_y(120) + 5;
        app.click(240, 120, 140, y); // Shuffle
        assert_ne!(app.show.controls[7].seed, seed);
        app.click(240, 120, 220, y); // Reset
        app.mouse(240, 120, Some((220, y)), true, true, 3.0);
        assert_eq!(app.show.controls[7], Controls::new(seed));
        assert!(!app.dragging);
        // A press originating in the toolbar cannot become a drag by moving into the image.
        app.mouse(240, 120, Some((190, 45)), true, false, 0.0);
        assert!(app.show.controls[7].pointer.is_none());
        app.mouse(240, 120, Some((190, 45)), true, true, 0.0);
        app.mouse(240, 120, None, true, false, 0.0);
        assert!(app.show.controls[7].pointer.is_none());
        app.key(Key::N);
        app.key(Key::Backspace);
        assert_eq!(app.show.controls[7], Controls::new(seed));
    }

    #[test]
    #[ignore]
    fn render_interaction_review() {
        std::fs::create_dir_all("target/interaction-review").unwrap();
        for mode in [5, 6, 7, 9, 10] {
            let mut app = App::new(&test_options(mode), false);
            app.show_help = false;
            app.help_t = None;
            for _ in 0..360 { app.frame(480, 300, 1.0 / 60.0); }
            for step in 0..180 {
                if step < 120 {
                    let a = step as f32 / 120.0 * std::f32::consts::TAU;
                    let pos = ((240.0 + a.cos() * 120.0) as i32, (140.0 + a.sin() * 70.0) as i32);
                    app.mouse(480, 300, Some(pos), true, step == 0, if step == 60 { 2.0 } else { 0.0 });
                } else if step == 120 { app.reset_visual(false); }
                app.frame(480, 300, 1.0 / 60.0);
                if step % 4 == 0 {
                    app.screen.save_bmp(&format!("target/interaction-review/mode-{mode}-{:03}.bmp", step / 4)).unwrap();
                }
            }
            app.frame(240, 120, 1.0 / 60.0);
            app.screen.save_bmp(&format!("target/interaction-review/compact-{mode}.bmp")).unwrap();
        }
    }

    #[test]
    fn function_rebinding_replaces_letters_and_dispatches_actions_directly() {
        let opts = Options { source: Source::Test, mode: 5, skin: 0, scale: 1,
            auto: false, snapshot: None, probe: false, preset: Preset::default(), save_preset: None, ..Options::default() };
        let mut app = App::new(&opts, false);
        let (skin, auto) = (catalogue::index("skin").unwrap(), catalogue::index("auto").unwrap());
        app.key(Key::M);
        app.catalogue.selected = skin; // Next skin, initially S
        assert_eq!(app.catalogue.bindings.labels(skin), "S");
        app.key(Key::Enter);
        app.key(Key::A); // Move skin onto Auto's former shortcut.
        assert!(!app.catalogue.rebinding);
        assert_eq!(app.catalogue.bindings.entry(Key::S), None);
        app.key(Key::Escape);
        app.key(Key::A);
        assert_eq!(app.skin, 1);
        assert!(!app.show.auto.on, "The old A action must not run");
        app.key(Key::S);
        assert_eq!(app.skin, 1, "The previous shortcut must be inactive");
        app.key(Key::M);
        app.catalogue.selected = auto; // Auto now on S; no recursive A/S routing.
        app.key(Key::Enter);
        app.key(Key::S);
        app.key(Key::Escape);
        app.key(Key::S);
        assert!(app.show.auto.on);
        assert_eq!(app.skin, 1);
        app.key(Key::M);
        app.key(Key::Key9); // Numeric assignment replaces S.
        assert_eq!(app.catalogue.bindings.entry(Key::S), None);
        assert_eq!(app.catalogue.bindings.labels(auto), "9");
        app.key(Key::Escape);
        app.key(Key::NumPad9);
        assert!(!app.show.auto.on);
        assert_eq!(app.show.mode, 5, "9 must no longer switch to Wireframe");
    }

    #[test]
    fn selected_rate_functions_accept_letters_without_enter_and_survive_reload() {
        let mut app = App::new(&test_options(0), false);
        let initial_rate = app.show.rate_index();
        app.key(Key::M);
        app.key(Key::Right);
        let slower = catalogue::index("slower").unwrap();
        let faster = catalogue::index("faster").unwrap();
        for _ in catalogue::VISUALS..slower { app.key(Key::Down); }
        assert_eq!(app.catalogue.selected, slower);
        app.key(Key::Q);
        assert_eq!(app.catalogue.bindings.entry(Key::Q), Some(slower));
        assert_eq!(app.catalogue.bindings.entry(Key::Down), None);
        app.key(Key::Down);
        assert_eq!(app.catalogue.selected, faster, "Menu navigation still works after rebinding Down");
        app.key(Key::E);
        assert_eq!(app.catalogue.bindings.entry(Key::E), Some(faster));
        assert_eq!(app.catalogue.bindings.entry(Key::Up), None);
        assert_eq!(app.show.rate_index(), initial_rate, "Assignment must not execute the function");
        assert!(app.catalogue.open && !app.catalogue.rebinding);

        let path = std::env::temp_dir().join(format!("sonograph-rate-hotkeys-{}.conf", std::process::id()));
        app.catalogue.bindings.save(&path).unwrap();
        app.catalogue.bindings = catalogue::Bindings::load(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        app.key(Key::Escape);
        app.key(Key::Q);
        assert_eq!(app.show.rate_index(), initial_rate - 1);
        app.key(Key::E);
        assert_eq!(app.show.rate_index(), initial_rate);
        app.key(Key::E);
        assert_eq!(app.show.rate_index(), initial_rate + 1);
        app.key(Key::Down);
        app.key(Key::Up);
        assert_eq!(app.show.rate_index(), initial_rate + 1, "Previous shortcuts stay unassigned");
    }

    #[test]
    fn cancelled_rebinding_preserves_shortcuts_and_reserved_menu_key() {
        let opts = Options { source: Source::Test, mode: 0, skin: 0, scale: 1,
            auto: false, snapshot: None, probe: false, preset: Preset::default(), save_preset: None, ..Options::default() };
        let mut app = App::new(&opts, false);
        app.key(Key::M);
        app.key(Key::Right);
        let before = app.catalogue.bindings.clone();
        app.key(Key::Enter);
        app.key(Key::M);
        assert!(app.catalogue.open && app.catalogue.rebinding);
        app.key(Key::Escape);
        assert!(app.catalogue.open && !app.catalogue.rebinding);
        assert_eq!(app.catalogue.bindings, before);
        app.key(Key::Escape);
        assert!(!app.catalogue.open);
    }

    #[test]
    fn catalogue_routes_assignments_without_running_them() {
        let opts = Options { source: Source::Test, mode: 0, skin: 0, scale: 1,
            auto: false, snapshot: None, probe: false, preset: Preset::default(), save_preset: None, ..Options::default() };
        let mut app = App::new(&opts, false);
        app.click(640, 360, 35, 5);
        assert!(app.catalogue.open);
        app.key(Key::Right); // Functions tab
        app.key(Key::Down);
        app.key(Key::Down); // Toggle auto
        app.key(Key::NumPad1);
        assert!(!app.show.auto.on, "Assigning a function must not execute it");
        assert_eq!(app.show.mode, 0, "Menu navigation must not change the visual");
        app.key(Key::Escape);
        assert!(!app.catalogue.open);
        app.key(Key::Key1);
        assert!(app.show.auto.on);
        app.key(Key::NumPad1);
        assert!(!app.show.auto.on);
        app.key(Key::M);
        app.key(Key::Left);
        for _ in 0..6 { app.key(Key::Down); }
        app.key(Key::Key2); // Map 2 to Orbs
        app.key(Key::M);
        app.key(Key::NumPad2);
        assert_eq!(app.show.mode, 6);
        app.key(Key::M);
        app.key(Key::Delete);
        app.key(Key::M);
        app.set_mode(0);
        app.key(Key::Key2);
        assert_eq!(app.show.mode, 0, "Cleared shortcuts must not execute their old binding");
        app.key(Key::M);
        app.key(Key::Space); // Run selected Orbs directly
        assert!(!app.catalogue.open);
        assert_eq!(app.show.mode, 6);
        app.key(Key::M);
        app.key(Key::D);
        app.key(Key::Escape);
        app.key(Key::Key2);
        assert_eq!(app.show.mode, 1);
    }

    /// Render the actual menu without requiring audio devices or a desktop session.
    #[test]
    #[ignore]
    fn render_catalogue_review() {
        let opts = Options { source: Source::Test, mode: 0, skin: 2, scale: 1,
            auto: false, snapshot: None, probe: false, preset: Preset::default(), save_preset: None, ..Options::default() };
        let mut app = App::new(&opts, false);
        std::fs::create_dir_all("target/catalogue-review").unwrap();
        app.key(Key::M);
        for (w, h, name) in [(640, 360, "visuals"), (240, 120, "compact")] {
            app.frame(w, h, 1.0 / 60.0);
            app.screen.save_bmp(&format!("target/catalogue-review/{name}.bmp")).unwrap();
        }
        app.key(Key::Right);
        app.key(Key::Down);
        app.key(Key::Down);
        app.key(Key::Key1);
        app.frame(640, 360, 1.0 / 60.0);
        app.screen.save_bmp("target/catalogue-review/functions.bmp").unwrap();
    }

    #[test]
    fn fullscreen_fills_the_canvas_and_routes_pointer_over_hidden_controls() {
        let mut app = App::new(&test_options(6), false);
        app.catalogue.bindings = catalogue::Bindings::parse("F11 = none").unwrap();
        app.key(Key::F11);
        assert!(app.fullscreen, "Legacy saved settings must not disable F11");
        app.frame(640, 360, 1.0 / 60.0);
        assert_eq!((app.show.composite.w, app.show.composite.h), (640, 360));
        assert_eq!(app.screen.px, app.show.composite.px, "Fullscreen must contain only the visual");
        let help_t = app.help_t;
        app.frame(640, 360, 0.1);
        assert_eq!(app.help_t, help_t, "Preserve help visibility for the return to windowed mode");

        let seed = app.show.controls[6].seed;
        for (x, y) in [(20, 4), (120, 6), (630, 355)] {
            app.click(640, 360, x, y);
            app.mouse(640, 360, Some((x, y)), true, true, 0.0);
            assert!(app.fullscreen && !app.catalogue.open);
            assert_eq!(app.show.controls[6].seed, seed);
            assert!(app.dragging);
            let pointer = app.show.controls[6].pointer.unwrap();
            assert_eq!(pointer, (x as f32 / 640.0, y as f32 / 360.0));
        }
        app.key(Key::M);
        app.mouse(640, 360, Some((20, 4)), true, true, 0.0);
        assert!(!app.dragging);
        app.key(Key::F11);
        assert!(!app.fullscreen, "F11 must work while browsing the catalogue");
        assert!(app.catalogue.open);
        app.key(Key::Escape);
        app.frame(640, 360, 1.0 / 60.0);
        assert_eq!(app.show.composite.h, 360 - HUD_H - BAR_H - HELP_H - RATE_H);
        assert_ne!(app.screen.px, app.show.composite.px);
    }

    #[test]
    fn fullscreen_controls_respect_catalogue_and_reassigned_shortcuts() {
        let mut app = App::new(&test_options(0), false);
        app.key(Key::F11);
        assert!(app.fullscreen);
        app.key(Key::M);
        app.key(Key::Escape);
        assert!(!app.catalogue.open);
        assert!(app.fullscreen, "Closing the catalogue must not leave fullscreen");
        app.key(Key::Escape);
        assert!(!app.fullscreen);

        app.click(640, 360, 120, 6);
        assert!(app.fullscreen);
        app.click(640, 360, 120, 6);
        assert!(app.fullscreen, "The hidden button must not respond to clicks");
        app.key(Key::F11);
        assert!(!app.fullscreen);

        app.catalogue.selected = catalogue::ENTRIES.iter().position(|e| e.id == "fullscreen").unwrap();
        app.key(Key::M);
        app.key(Key::Enter);
        app.key(Key::F2);
        app.key(Key::Escape);
        app.key(Key::F11);
        assert!(!app.fullscreen, "The old shortcut must stop working after reassignment");
        app.key(Key::F2);
        assert!(app.fullscreen);
        app.key(Key::M);
        app.key(Key::Space);
        assert!(!app.fullscreen, "The catalogue must dispatch fullscreen directly");
    }

    #[test]
    fn miniaturize_routes_controls_and_preserves_player_state() {
        let mut app = App::new(&test_options(6), false);
        app.show.auto.on = true;
        app.show.set_rate(4);
        app.show.pulse = false;
        let seed = app.show.controls[6].seed;
        app.dragging = true;
        app.show.controls[6].pointer = Some((0.5, 0.5));
        app.key(Key::F12);
        assert!(app.miniaturized);
        assert!(!app.dragging);
        assert_eq!(app.show.controls[6].pointer, None);
        app.key(Key::Escape);
        assert!(!app.miniaturized);
        assert_eq!(app.show.mode, 6);
        assert_eq!(app.show.controls[6].seed, seed);
        assert!(app.show.auto.on);
        assert_eq!(app.show.rate_index(), 4);
        assert!(!app.show.pulse);
        for width in [240, 320, 640] {
            app.frame(width, 180, 1.0 / 60.0);
            app.click(width, 180, 215, 5);
            assert!(app.miniaturized, "The top-bar button works at every supported width");
            app.click(width, 180, 120, 5);
            assert!(!app.fullscreen, "Hidden controls must not accept clicks");
            app.set_miniaturized(false);
        }
        app.set_fullscreen(true);
        app.key(Key::M);
        app.key(Key::F12);
        assert!(app.miniaturized && app.fullscreen && !app.catalogue.open);
        app.key(Key::Escape);
        assert!(!app.miniaturized && app.fullscreen, "Restoring the player preserves fullscreen");
        app.catalogue.bindings = catalogue::Bindings::parse("F12 = none\nF2 = miniaturize").unwrap();
        app.key(Key::F12);
        assert!(!app.miniaturized);
        app.key(Key::F2);
        assert!(app.miniaturized);
        app.set_miniaturized(false);
        app.key(Key::M);
        app.catalogue.selected = catalogue::index("miniaturize").unwrap();
        app.key(Key::Space);
        assert!(app.miniaturized && !app.catalogue.open);
    }

    fn test_options(mode: usize) -> Options {
        Options { source: Source::Test, mode, skin: 2, scale: 1,
            auto: false, snapshot: None, probe: false, preset: Preset::default(), save_preset: None, ..Options::default() }
    }

    /// Kaleidoscope and Mandala frames from the test signal at the default render size, with
    /// the frame time.
    #[test]
    #[ignore]
    fn render_kaleidoscope_review() {
        let dir = std::path::Path::new("target/kaleidoscope-review");
        std::fs::create_dir_all(dir).unwrap();
        for (mode, name) in [(7, "kaleidoscope"), (9, "mandala")] {
            let mut app = App::new(&test_options(mode), false);
            app.show_help = false;
            app.help_t = None;
            let start = Instant::now();
            for i in 0..2400 {
                app.pal = Palette::new(i / 600);
                app.frame(640, 360, 1.0 / 60.0);
                if i % 150 == 149 {
                    app.screen.save_bmp(dir.join(format!("{name}-{:02}.bmp", i / 150)).to_str().unwrap()).unwrap();
                }
            }
            println!("{name} frames: {:.2} ms/frame", start.elapsed().as_secs_f64() * 1000.0 / 2400.0);
        }
    }

    /// Wireframe frames from the test signal, one every second, as the solids change shape
    /// and swap places.
    #[test]
    #[ignore]
    fn render_wireframe_review() {
        let dir = std::path::Path::new("target/wireframe-review");
        std::fs::create_dir_all(dir).unwrap();
        let mut app = App::new(&test_options(8), false);
        app.show_help = false;
        app.help_t = None;
        let start = Instant::now();
        for i in 0..3600 {
            app.frame(640, 360, 1.0 / 60.0);
            if i % 60 == 59 {
                app.screen.save_bmp(dir.join(format!("wireframe-{:02}.bmp", i / 60)).to_str().unwrap()).unwrap();
            }
        }
        println!("Wireframe frames: {:.2} ms/frame", start.elapsed().as_secs_f64() * 1000.0 / 3600.0);
    }

    /// Gravity Waves frames from the test signal in the Orchid skin, two a second, through an
    /// inspiral, a merger, and the next capture.
    #[test]
    #[ignore]
    fn render_gravity_review() {
        let dir = std::path::Path::new("target/gravity-review");
        std::fs::create_dir_all(dir).unwrap();
        let mut app = App::new(&Options { skin: 3, ..test_options(10) }, false);
        app.show_help = false;
        app.help_t = None;
        for i in 0..2700 {
            app.frame(640, 360, 1.0 / 60.0);
            if i % 30 == 29 {
                app.screen.save_bmp(dir.join(format!("gravity-{:02}.bmp", i / 30)).to_str().unwrap()).unwrap();
            }
        }
        // The default 1280x720 / pixel-size-2 workload, beside Orbs, without disk writes.
        for mode in [10, 6] {
            let mut app = App::new(&test_options(mode), false);
            let start = Instant::now();
            for _ in 0..600 { app.frame(640, 360, 1.0 / 60.0); }
            println!("{}: {:.2} ms/frame", app.show.name(), start.elapsed().as_secs_f64() * 1000.0 / 600.0);
        }
    }

    #[test]
    fn interrupted_crossfades_preserve_display_and_resize_safely() {
        let mut app = App::new(&test_options(6), false);
        for _ in 0..20 { app.frame(160, 120, 1.0 / 60.0); }
        app.set_mode(5);
        assert_eq!(app.show.transition.as_ref().unwrap().mode, Some(6));
        for _ in 0..20 { app.frame(160, 120, 1.0 / 60.0); }
        let displayed = app.show.composite.px.clone();
        app.set_mode(3);
        assert_eq!(app.show.transition.as_ref().unwrap().mode, None);
        app.frame(160, 120, 1.0 / 60.0);
        assert_eq!(app.show.composite.px, displayed, "Interrupted fade starts from exactly what was displayed");
        for _ in 0..160 { app.frame(160, 120, 1.0 / 60.0); }
        assert!(app.show.transition.is_none());
        app.set_mode(6);
        app.frame(120, 160, 1.0 / 60.0);
        assert!(app.show.transition.is_none());
        assert_eq!(app.show.composite.px.len(), 120 * (160 - HUD_H - HELP_H - BAR_H - RATE_H));
        app.key(Key::R);
        assert_eq!(app.show.preset.name, "Slow Drift");
    }

    /// Deterministic frames from the actual renderer for visual review; opt in explicitly.
    #[test]
    #[ignore]
    fn render_visual_review() {
        let dir = std::path::Path::new("target/visual-review");
        std::fs::create_dir_all(dir).unwrap();
        let mut app = App::new(&test_options(6), false);
        app.show_help = false;
        app.help_t = None;
        for i in 0..720 {
            app.frame(480, 300, 1.0 / 60.0);
            if i >= 480 && i % 4 == 0 {
                app.screen.save_bmp(dir.join(format!("orbs-{:03}.bmp", (i - 480) / 4)).to_str().unwrap()).unwrap();
            }
        }
        for skin in 0..SKINS.len() {
            app.pal = Palette::new(skin);
            app.frame(480, 300, 1.0 / 60.0);
            app.screen.save_bmp(dir.join(format!("skin-{skin}.bmp")).to_str().unwrap()).unwrap();
        }
        app.pal = Palette::new(2);
        app.set_mode(5);
        for i in 0..240 {
            app.frame(480, 300, 1.0 / 60.0);
            if i % 4 == 0 {
                app.screen.save_bmp(dir.join(format!("transition-{:03}.bmp", i / 4)).to_str().unwrap()).unwrap();
            }
        }
        for recipe in 0..3 {
            app.show.set_preset(Preset::builtin(recipe));
            for _ in 0..180 { app.frame(480, 300, 1.0 / 60.0); }
            app.screen.save_bmp(dir.join(format!("tunnel-{recipe}.bmp")).to_str().unwrap()).unwrap();
        }
        // Measure the default 1280x720 / pixel-size-2 render workload, excluding disk writes.
        app.set_mode(6);
        let start = Instant::now();
        for _ in 0..180 { app.frame(640, 360, 1.0 / 60.0); }
        println!("Default-size fluid frames: {:.2} ms/frame", start.elapsed().as_secs_f64() * 1000.0 / 180.0);
    }

    #[test]
    fn effect_hotkeys_and_mode_cycle() {
        let opts = Options { source: Source::Test, mode: 0, skin: 0, scale: 1,
            auto: false, snapshot: None, probe: false, preset: Preset::default(), save_preset: None, ..Options::default() };
        let mut app = App::new(&opts, false);
        for (key, name) in [
            (Key::Key7, "ORBS"), (Key::NumPad7, "ORBS"),
            (Key::Key8, "KALEIDOSCOPE"), (Key::NumPad8, "KALEIDOSCOPE"),
            (Key::Key9, "WIREFRAME"), (Key::NumPad9, "WIREFRAME"),
            (Key::Key0, "MANDALA"), (Key::NumPad0, "MANDALA"),
            (Key::G, "GRAVITY WAVES"),
            (Key::W, "SYNTHWAVE"),
        ] {
            app.key(key);
            assert_eq!(app.show.name(), name);
        }
        app.key(Key::Right);
        assert_eq!(app.show.mode, 0);
        app.key(Key::Left);
        assert_eq!(app.show.name(), "SYNTHWAVE");
        app.key(Key::A);
        app.show.auto.t = app.show.auto.due + show::AUTO_GRACE;
        app.frame(240, 120, 1.0 / 60.0);
        assert_eq!(app.show.mode, 0, "Auto cycling must wrap after the last mode");
    }
}
