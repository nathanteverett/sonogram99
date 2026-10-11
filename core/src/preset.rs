//! Editable visual recipes. A recipe is plain `key = value` text: numbers, names, and links
//! from the music to a setting. It never contains code, and every value is checked.
//!
//! Tunnel and Orbs share the basic settings and the effects. Tunnel also reads the motion
//! and the wave and shape layers, so most new Tunnel looks need only a recipe file.
//!
//! A setting can follow the music. `zoom = 1.006` sets the base value, and
//! `zoom.bass = 0.013` adds 0.013 times the bass level on every frame.

use std::collections::HashSet;

use crate::analysis::Frame;

/// Something a setting can follow: the music, or a slow wave over time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Fast band levels, 0 to 1, times the recipe's strength for that band.
    Bass,
    Mids,
    Treble,
    /// The same, smoothed over about a second.
    BassSlow,
    MidsSlow,
    TrebleSlow,
    /// Band levels against the song's own average: about 1, up to 4, times the strength.
    BassRel,
    MidsRel,
    TrebleRel,
    /// A kick-drum attack, 0 to 1, times the bass strength.
    Onset,
    /// A pulse on each detected beat that fades in a quarter second, times the bass strength.
    Beat,
    /// Stereo width, 0 (mono) to 1.
    Width,
    /// Overall level, 0 to 1.
    Level,
    /// Slow sine waves from -1 to 1, about 17, 12 and 15 seconds long.
    Drift1,
    Drift2,
    Drift3,
    /// A slow triangle wave from -1 to 1, about 11 seconds long.
    Drift4,
}

pub const SOURCES: usize = 17;

impl Source {
    pub const ALL: [Source; SOURCES] = [
        Source::Bass, Source::Mids, Source::Treble,
        Source::BassSlow, Source::MidsSlow, Source::TrebleSlow,
        Source::BassRel, Source::MidsRel, Source::TrebleRel,
        Source::Onset, Source::Beat, Source::Width, Source::Level,
        Source::Drift1, Source::Drift2, Source::Drift3, Source::Drift4,
    ];
    pub const NAMES: [&'static str; SOURCES] = [
        "bass", "mids", "treble",
        "bass_slow", "mids_slow", "treble_slow",
        "bass_rel", "mids_rel", "treble_rel",
        "onset", "beat", "width", "level",
        "drift1", "drift2", "drift3", "drift4",
    ];

    fn find(name: &str) -> Option<Source> {
        Self::NAMES.iter().position(|n| *n == name).map(|i| Self::ALL[i])
    }
}

/// The value of every source on one frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Inputs(pub [f32; SOURCES]);

impl Inputs {
    /// `t` is the visual's own clock in seconds, which runs at the recipe's speed.
    pub fn new(f: &Frame, p: &Preset, t: f32) -> Self {
        let m = &f.music;
        let strength = [p.bass, p.mids, p.treble];
        let mut v = [0.0; SOURCES];
        for band in 0..3 {
            v[Source::Bass as usize + band] = m.bands[band] * strength[band];
            v[Source::BassSlow as usize + band] = m.slow[band] * strength[band];
            v[Source::BassRel as usize + band] = m.rel[band] * strength[band];
        }
        v[Source::Onset as usize] = m.onset * p.bass;
        v[Source::Beat as usize] = m.pulse * p.bass;
        v[Source::Width as usize] = m.width;
        v[Source::Level as usize] = f.level;
        v[Source::Drift1 as usize] = (t * 0.37).sin();
        v[Source::Drift2 as usize] = (t * 0.53).sin();
        v[Source::Drift3 as usize] = (t * 0.41).cos();
        v[Source::Drift4 as usize] = 1.0 - 2.0 * ((t * 0.18).rem_euclid(2.0) - 1.0).abs();
        Inputs(v)
    }
}

/// One setting: a base value, plus an amount for each source it follows. The result is
/// always kept inside the setting's range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Param {
    pub base: f32,
    pub mods: [f32; SOURCES],
    pub lo: f32,
    pub hi: f32,
}

impl Param {
    pub fn new(lo: f32, hi: f32, base: f32) -> Self {
        Param { base, mods: [0.0; SOURCES], lo, hi }
    }

    pub fn with(mut self, source: Source, amount: f32) -> Self {
        self.mods[source as usize] = amount;
        self
    }

    /// The same range with a new base value and no sources.
    fn only(self, base: f32) -> Self {
        Param::new(self.lo, self.hi, base)
    }

    pub fn eval(&self, inputs: &Inputs) -> f32 {
        let linked: f32 = self.mods.iter().zip(&inputs.0).map(|(a, v)| a * v).sum();
        (self.base + linked).clamp(self.lo, self.hi)
    }

    /// The largest amount a source may add per unit.
    fn limit(&self) -> f32 {
        4.0 * (self.hi - self.lo)
    }
}

/// Declares a group of settings with their ranges, defaults, and default sources.
macro_rules! settings {
    ($(#[$doc:meta])* $group:ident {
        $($(#[$field_doc:meta])* $field:ident [$lo:expr, $hi:expr] $base:expr $(, $src:ident * $amt:expr)*;)*
    }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct $group {
            $($(#[$field_doc])* pub $field: Param,)*
        }

        impl Default for $group {
            fn default() -> Self {
                Self { $($field: Param::new($lo, $hi, $base)$(.with(Source::$src, $amt))*,)* }
            }
        }

        impl $group {
            pub const NAMES: &'static [&'static str] = &[$(stringify!($field)),*];

            pub fn params(&self) -> Vec<&Param> {
                vec![$(&self.$field),*]
            }

            pub fn get_mut(&mut self, name: &str) -> Option<&mut Param> {
                match name {
                    $(stringify!($field) => Some(&mut self.$field),)*
                    _ => None,
                }
            }
        }
    };
}

settings! {
    /// How the picture moves on each frame, read only by Tunnel. Rates are per frame at
    /// 60 frames a second, and the recipe's speed scales them.
    Motion {
        /// Zoom per frame. Above 1 the picture flies outward; below 1 it falls inward.
        zoom [0.8, 1.25] 1.006, Bass * 0.013, Onset * 0.008;
        /// Extra zoom that grows from nothing at the center to this much at the edges.
        zoom_edge [-0.1, 0.1] 0.0;
        /// Turn per frame, in radians.
        rot [-0.2, 0.2] 0.0, Drift1 * 0.003, MidsSlow * 0.006;
        /// Turn at the center that fades out toward the edges, in radians per frame.
        swirl [-0.3, 0.3] 0.0;
        /// Rings of zoom that travel outward from the center.
        ripple [-1.0, 1.0] 0.0;
        /// Sine-wave flow that bends the trails. The recipe's warp scales it.
        flow [0.0, 0.5] 0.025, Mids * 0.08;
        /// Drift per frame, as a share of the width (dx) and the height (dy).
        dx [-0.05, 0.05] 0.0, Drift2 * 0.003;
        dy [-0.05, 0.05] 0.0, Drift3 * 0.003;
        /// The center of the motion, as a share of the width and the height.
        cx [0.0, 1.0] 0.5;
        cy [0.0, 1.0] 0.5;
        /// Light the center loses on each frame, so long trails cannot fill it with white.
        center_dark [0.0, 1.0] 0.0;
        /// Light of a frame drawn around the edge on each frame. A zoom below 1 pulls it in.
        border [0.0, 2.0] 0.0;
        /// The border's width as a share of the shorter side.
        border_size [0.0, 0.2] 0.015;
        /// The border's place on the skin's gradient, 0 deep to 1 hot.
        border_color [0.0, 1.0] 0.7;
    }
}

settings! {
    /// Effects on the finished picture, for Tunnel and Orbs. They never feed back into the
    /// motion.
    Effects {
        /// Blends in a zoomed copy of the picture. `echo_mirror` can flip the copy.
        echo [0.0, 1.0] 0.0;
        /// The copy's size. Above 1 it is larger than the picture.
        echo_zoom [0.25, 4.0] 1.0;
        /// Above 1 brightens dark and middle tones; below 1 darkens them.
        gamma [0.25, 4.0] 1.0;
        /// Turns the brightest areas dark again, as in an over-exposed photo.
        solarize [0.0, 1.0] 0.0;
        /// Turns the picture into its negative.
        invert [0.0, 1.0] 0.0;
    }
}

settings! {
    /// A trace of the sound, drawn on each frame. Sizes are shares of the shorter side.
    WaveSettings {
        /// The center, as a share of the width and the height.
        x [0.0, 1.0] 0.5;
        y [0.0, 1.0] 0.5;
        /// A ring's radius, half a line's length, or the size of an xy figure.
        size [0.0, 2.0] 0.12, Bass * 0.05;
        /// How far the sound moves the trace.
        amp [0.0, 1.0] 0.12;
        /// Turn speed in radians per second.
        spin [-6.0, 6.0] 0.6;
        /// A fixed turn in radians.
        angle [-6.3, 6.3] 0.0;
        /// The place on the skin's gradient, 0 deep to 1 hot.
        color [0.0, 1.0] 0.5, Drift4 * 0.3;
        /// How far the color changes along the trace.
        spread [0.0, 1.0] 0.2;
        /// Light added on each frame.
        light [0.0, 3.0] 0.22, Bass * 0.25;
        /// A slow three-lobed bulge of a ring, or a bend of a line.
        wobble [0.0, 1.0] 0.0, Mids * 0.16;
        /// A fine ripple along the trace. The recipe's detail scales it.
        fringe [0.0, 0.5] 0.0, Treble * 0.045;
        /// The brush width.
        thick [0.002, 0.05] 0.01;
    }
}

settings! {
    /// A regular polygon, drawn on each frame. Sizes are shares of the shorter side.
    ShapeSettings {
        /// The number of corners, rounded to a whole number.
        sides [3.0, 32.0] 6.0;
        /// The center, as a share of the width and the height.
        x [0.0, 1.0] 0.5;
        y [0.0, 1.0] 0.5;
        /// The distance from the center to each corner.
        size [0.0, 2.0] 0.2;
        /// Turn speed in radians per second.
        spin [-6.0, 6.0] 0.0;
        /// A fixed turn in radians.
        angle [-6.3, 6.3] 0.0;
        /// The place on the skin's gradient, 0 deep to 1 hot.
        color [0.0, 1.0] 0.5;
        /// Outline: light added on each frame. Filled: the light it fills to.
        light [0.0, 3.0] 0.3;
        /// Filled and textured: how strongly the fill covers the trails, per frame.
        opacity [0.0, 1.0] 0.3;
        /// Textured: 1 fits the picture's shorter side across the shape. Larger values
        /// show a smaller part of the picture, closer up.
        zoom [0.1, 8.0] 1.0;
        /// Textured: a turn of the copied picture, in radians.
        twist [-6.3, 6.3] 0.0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaveKind {
    /// The waveform around a circle.
    Ring,
    /// The waveform along a straight line.
    Line,
    /// The spectrum along a line, bass at the start.
    Spectrum,
    /// Left against right, as on the Vector visual.
    Xy,
}

impl WaveKind {
    const ALL: [WaveKind; 4] = [WaveKind::Ring, WaveKind::Line, WaveKind::Spectrum, WaveKind::Xy];
    const NAMES: [&'static str; 4] = ["ring", "line", "spectrum", "xy"];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wave {
    pub kind: WaveKind,
    pub set: WaveSettings,
}

impl Wave {
    /// A wave with the defaults for its kind.
    pub fn new(kind: WaveKind) -> Self {
        let mut set = WaveSettings::default();
        if kind != WaveKind::Ring {
            set.spin = set.spin.only(0.0);
            set.size = set.size.only(if kind == WaveKind::Xy { 0.3 } else { 0.75 });
        }
        match kind {
            WaveKind::Line => set.amp = set.amp.only(0.15),
            WaveKind::Spectrum => {
                set.amp = set.amp.only(0.3);
                set.wobble = set.wobble.only(0.0);
            }
            WaveKind::Xy => {
                set.wobble = set.wobble.only(0.0);
                set.fringe = set.fringe.only(0.0);
            }
            WaveKind::Ring => {}
        }
        Wave { kind, set }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    /// Lines of light along the edges.
    Outline,
    /// A soft fill of light.
    Filled,
    /// A fill with a smaller, turned copy of the whole picture. Because the copy feeds back,
    /// copies appear inside copies.
    Textured,
}

impl ShapeKind {
    const ALL: [ShapeKind; 3] = [ShapeKind::Outline, ShapeKind::Filled, ShapeKind::Textured];
    const NAMES: [&'static str; 3] = ["outline", "filled", "textured"];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub kind: ShapeKind,
    pub set: ShapeSettings,
}

impl Shape {
    /// A shape with the defaults for its kind.
    pub fn new(kind: ShapeKind) -> Self {
        let mut set = ShapeSettings::default();
        match kind {
            ShapeKind::Outline => {}
            ShapeKind::Filled => {
                set.light = set.light.only(0.8);
                set.opacity = set.opacity.only(0.15);
            }
            ShapeKind::Textured => {
                set.size = set.size.only(0.3);
                set.opacity = set.opacity.only(0.5);
            }
        }
        Shape { kind, set }
    }
}

/// How the echo copy is flipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mirror {
    None,
    /// Left to right.
    X,
    /// Top to bottom.
    Y,
    /// Both ways.
    Both,
}

impl Mirror {
    const ALL: [Mirror; 4] = [Mirror::None, Mirror::X, Mirror::Y, Mirror::Both];
    const NAMES: [&'static str; 4] = ["none", "x", "y", "xy"];
}

/// The number of wave layers and of shape layers.
pub const LAYERS: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub name: String,
    pub seed: u32,
    pub speed: f32,
    pub warp: f32,
    pub persistence: f32,
    pub bass: f32,
    pub mids: f32,
    pub treble: f32,
    pub background: f32,
    pub body: f32,
    pub detail: f32,
    pub motion: Motion,
    pub effects: Effects,
    pub mirror: Mirror,
    pub waves: [Option<Wave>; LAYERS],
    pub shapes: [Option<Shape>; LAYERS],
}

impl Default for Preset {
    fn default() -> Self { Self::builtin(0) }
}

/// The recipes in the code. The ones from files follow them.
const CODED: [&str; 3] = ["Liquid", "Slow Drift", "Electric"];

/// Built-in recipes kept as files, so they also serve as examples of the format.
const FILES: [&str; 4] = [
    include_str!("../presets/echo-chamber.preset"),
    include_str!("../presets/glass-tiles.preset"),
    include_str!("../presets/ripple-pool.preset"),
    include_str!("../presets/solar-flare.preset"),
];

#[derive(Clone, Copy)]
enum Layer {
    Wave(usize),
    Shape(usize),
}

/// `wave1` to `wave4` and `shape1` to `shape4`.
fn layer(key: &str) -> Option<Layer> {
    let (kind, n) = if let Some(n) = key.strip_prefix("wave") { (0, n) } else { (1, key.strip_prefix("shape")?) };
    let n: usize = n.parse().ok().filter(|n| (1..=LAYERS).contains(n))?;
    Some(if kind == 0 { Layer::Wave(n - 1) } else { Layer::Shape(n - 1) })
}

fn choose<T: Copy>(key: &str, value: &str, all: &[T], names: &[&str]) -> Result<T, String> {
    names.iter().position(|n| n.eq_ignore_ascii_case(value)).map(|i| all[i])
        .ok_or_else(|| format!("{key} must be one of: {}", names.join(", ")))
}

/// A layer's kind, or None for `none`.
fn kind<T: Copy>(key: &str, value: &str, all: &[T], names: &[&str]) -> Result<Option<T>, String> {
    if value.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    choose(key, value, all, names).map(Some).map_err(|e| e + ", none")
}

fn number(key: &str, value: &str) -> Result<f32, String> {
    value.parse::<f32>().ok().filter(|n| n.is_finite()).ok_or_else(|| format!("{key} must be a number"))
}

impl Preset {
    pub fn builtin_count() -> usize {
        CODED.len() + FILES.len()
    }

    pub fn builtin(index: usize) -> Self {
        let index = index % Self::builtin_count();
        if let Some(text) = index.checked_sub(CODED.len()).map(|i| FILES[i]) {
            return Self::parse(text).expect("Built-in recipe files are checked by the tests");
        }
        let mut waves = [None; LAYERS];
        waves[0] = Some(Wave::new(WaveKind::Ring));
        let mut p = Self {
            name: CODED[index].into(), seed: crate::interaction::DEFAULT_SEED, speed: 1.0, warp: 1.0,
            persistence: 0.975, bass: 1.0, mids: 1.0, treble: 1.0,
            background: 0.42, body: 1.0, detail: 0.7,
            motion: Motion::default(), effects: Effects::default(), mirror: Mirror::None,
            waves, shapes: [None; LAYERS],
        };
        match index {
            1 => { p.speed = 0.48; p.warp = 0.65; p.persistence = 0.987; p.bass = 0.7; p.treble = 0.5; p.detail = 0.35; }
            2 => { p.speed = 1.35; p.warp = 1.6; p.persistence = 0.96; p.mids = 1.25; p.treble = 1.4; p.detail = 1.0; }
            _ => {}
        }
        p
    }

    pub fn builtin_names() -> Vec<String> {
        (0..Self::builtin_count()).map(|i| Self::builtin(i).name).collect()
    }

    pub fn builtin_index(name: &str) -> Option<usize> {
        Self::builtin_names().iter().position(|n| n.eq_ignore_ascii_case(name))
    }

    pub fn load(value: &str) -> Result<Self, String> {
        if let Some(i) = Self::builtin_index(value) {
            return Ok(Self::builtin(i));
        }
        let text = std::fs::read_to_string(value).map_err(|e| format!("Could not read preset {value}: {e}"))?;
        Self::parse(&text).map_err(|e| format!("Preset {value}: {e}"))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut p = Self::default();
        let mut seen = HashSet::new();
        let mut lines = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() { continue; }
            let (key, value) = line.split_once('=').ok_or_else(|| format!("Line {} needs key = value", i + 1))?;
            let (key, value) = (key.trim(), value.trim());
            if !seen.insert(key) { return Err(format!("Repeated key: {key}")); }
            lines.push((key, value));
        }
        // Layer kinds first, so a layer's settings may come before or after its kind line.
        for &(key, value) in &lines {
            match layer(key) {
                Some(Layer::Wave(i)) => p.waves[i] = kind(key, value, &WaveKind::ALL, &WaveKind::NAMES)?.map(Wave::new),
                Some(Layer::Shape(i)) => p.shapes[i] = kind(key, value, &ShapeKind::ALL, &ShapeKind::NAMES)?.map(Shape::new),
                None => {}
            }
        }
        for &(key, value) in &lines {
            if layer(key).is_none() {
                p.set(key, value)?;
            }
        }
        Ok(p)
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "seed" => {
                self.seed = value.parse().map_err(|_| "seed must be a whole number from 0 to 4294967295")?;
                return Ok(());
            }
            "name" => {
                if value.is_empty() || value.len() > 32 || !value.chars().all(|c| c.is_ascii_alphanumeric() || " -_".contains(c)) {
                    return Err("Name must be 1-32 letters, numbers, spaces, hyphens or underscores".into());
                }
                self.name = value.into();
                return Ok(());
            }
            "echo_mirror" => {
                self.mirror = choose(key, value, &Mirror::ALL, &Mirror::NAMES)?;
                return Ok(());
            }
            _ => {}
        }
        let basic = match key {
            "speed" => Some((&mut self.speed, 0.1, 3.0)),
            "warp" => Some((&mut self.warp, 0.0, 3.0)),
            "persistence" => Some((&mut self.persistence, 0.8, 0.995)),
            "bass" => Some((&mut self.bass, 0.0, 2.0)),
            "mids" => Some((&mut self.mids, 0.0, 2.0)),
            "treble" => Some((&mut self.treble, 0.0, 2.0)),
            "background" => Some((&mut self.background, 0.0, 1.0)),
            "body" => Some((&mut self.body, 0.0, 1.0)),
            "detail" => Some((&mut self.detail, 0.0, 1.0)),
            _ => None,
        };
        if let Some((slot, low, high)) = basic {
            let n = number(key, value)?;
            if n < low || n > high { return Err(format!("{key} must be between {low} and {high}")); }
            *slot = n;
            return Ok(());
        }
        self.set_param(key, value)
    }

    /// `setting = base` or `setting.source = amount`, where a layer's setting is written
    /// as `wave1.size`.
    fn set_param(&mut self, key: &str, value: &str) -> Result<(), String> {
        let unknown = || format!("Unknown key: {key}");
        let mut parts = key.split('.');
        let first = parts.next().unwrap_or_default();
        let param = match layer(first) {
            Some(which) => {
                let setting = parts.next().ok_or_else(unknown)?;
                let missing = || format!("{first} has settings but no kind. Add a line such as {first} = {}",
                    if matches!(which, Layer::Wave(_)) { "ring" } else { "outline" });
                match which {
                    Layer::Wave(i) => self.waves[i].as_mut().ok_or_else(missing)?.set.get_mut(setting),
                    Layer::Shape(i) => self.shapes[i].as_mut().ok_or_else(missing)?.set.get_mut(setting),
                }
            }
            None => self.motion.get_mut(first).or_else(|| self.effects.get_mut(first)),
        }
        .ok_or_else(unknown)?;
        let n = number(key, value)?;
        match (parts.next(), parts.next()) {
            (None, _) => {
                if n < param.lo || n > param.hi {
                    return Err(format!("{key} must be between {} and {}", param.lo, param.hi));
                }
                param.base = n;
            }
            (Some(source), None) => {
                let source = Source::find(source).ok_or_else(|| {
                    format!("Unknown source in {key}. Sources: {}", Source::NAMES.join(", "))
                })?;
                let limit = param.limit();
                if n.abs() > limit {
                    return Err(format!("{key} must be between {} and {limit}", -limit));
                }
                param.mods[source as usize] = n;
            }
            _ => return Err(unknown()),
        }
        Ok(())
    }

    pub fn text(&self) -> String {
        let mut s = format!(
            "# Sonograph 99 visual recipe. Tunnel and Orbs use the first block and the effects;\n\
             # Tunnel also uses the motion and the layers. The seed also applies to Kaleidoscope and Mandala.\n\
             # A line such as zoom.bass = 0.013 adds 0.013 times the bass level to zoom.\n\
             name = {}\nseed = {}\nspeed = {}\nwarp = {}\npersistence = {}\nbass = {}\nmids = {}\ntreble = {}\nbackground = {}\nbody = {}\ndetail = {}\n",
            self.name, self.seed, self.speed, self.warp, self.persistence, self.bass, self.mids,
            self.treble, self.background, self.body, self.detail);
        s += "\n# Motion (Tunnel), per frame at 60 frames a second\n";
        write_group(&mut s, "", Motion::NAMES, &self.motion.params(), &Motion::default().params());
        s += "\n# Effects on the finished picture\n";
        write_group(&mut s, "", Effects::NAMES, &self.effects.params(), &Effects::default().params());
        s += &format!("echo_mirror = {}\n", Mirror::NAMES[Mirror::ALL.iter().position(|m| *m == self.mirror).unwrap_or(0)]);
        for (i, wave) in self.waves.iter().enumerate() {
            let n = i + 1;
            match wave {
                Some(w) => {
                    let kind = WaveKind::NAMES[WaveKind::ALL.iter().position(|k| *k == w.kind).unwrap_or(0)];
                    s += &format!("\n# Wave layer {n} (Tunnel)\nwave{n} = {kind}\n");
                    write_group(&mut s, &format!("wave{n}."), WaveSettings::NAMES, &w.set.params(), &Wave::new(w.kind).set.params());
                }
                // The first wave is on unless a recipe turns it off.
                None if i == 0 => s += "\nwave1 = none\n",
                None => {}
            }
        }
        for (i, shape) in self.shapes.iter().enumerate() {
            if let Some(sh) = shape {
                let n = i + 1;
                let kind = ShapeKind::NAMES[ShapeKind::ALL.iter().position(|k| *k == sh.kind).unwrap_or(0)];
                s += &format!("\n# Shape layer {n} (Tunnel)\nshape{n} = {kind}\n");
                write_group(&mut s, &format!("shape{n}."), ShapeSettings::NAMES, &sh.set.params(), &Shape::new(sh.kind).set.params());
            }
        }
        s
    }

    /// Create a new file, never silently overwrite a user's recipe.
    pub fn save(&self, path: &str) -> Result<(), String> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)
            .map_err(|e| format!("Could not create preset {path}: {e}"))?;
        file.write_all(self.text().as_bytes()).map_err(|e| format!("Could not save preset: {e}"))
    }
}

/// Writes each setting's base, and each source it follows or followed by default.
fn write_group(s: &mut String, prefix: &str, names: &[&str], params: &[&Param], defaults: &[&Param]) {
    for ((name, p), d) in names.iter().zip(params).zip(defaults) {
        s.push_str(&format!("{prefix}{name} = {}\n", p.base));
        for (k, source) in Source::NAMES.iter().enumerate() {
            if p.mods[k] != 0.0 || d.mods[k] != 0.0 {
                s.push_str(&format!("{prefix}{name}.{source} = {}\n", p.mods[k]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipes_round_trip_and_reject_invalid_values() {
        for i in 0..Preset::builtin_count() {
            let p = Preset::builtin(i);
            assert_eq!(Preset::parse(&p.text()).unwrap(), p, "{}", p.name);
            assert_eq!(Preset::builtin_index(&p.name), Some(i));
        }
        for bad in ["warp = NaN", "speed = inf", "persistence = 1", "body = -1", "typo = 2", "warp = 1\nwarp = 2", "name =",
            "zoom = 2", "zoom.bass = 9", "zoom.loudness = 0.1", "zoom.bass.x = 1", "wave2.size = 0.2", "wave1 = spiral",
            "wave5 = ring", "shape1 = outline\nshape1.colour = 1", "echo_mirror = z", "seed = -1"] {
            assert!(Preset::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn old_recipe_files_still_load_with_the_original_look() {
        let old = "name = Mine\nseed = 7\nspeed = 1.2\nwarp = 0.5\npersistence = 0.97\nbass = 1\nmids = 1\ntreble = 1\nbackground = 0.4\nbody = 1\ndetail = 0.6\n";
        let p = Preset::parse(old).unwrap();
        assert_eq!((p.name.as_str(), p.seed, p.speed), ("Mine", 7, 1.2));
        assert_eq!(p.motion, Motion::default());
        assert_eq!(p.waves[0], Some(Wave::new(WaveKind::Ring)));
        assert!(p.shapes.iter().all(Option::is_none));
    }

    #[test]
    fn settings_follow_their_sources_inside_their_range() {
        let p = Preset::parse("zoom = 1.01\nzoom.bass = 0.02\nzoom.bass_rel = -0.01\nwave1 = line\nwave1.size = 0.5\nshape2 = textured\nshape2.sides = 5").unwrap();
        let mut inputs = Inputs::default();
        assert_eq!(p.motion.zoom.eval(&inputs), 1.01);
        inputs.0[Source::Bass as usize] = 1.0;
        inputs.0[Source::Onset as usize] = 1.0;
        assert!((p.motion.zoom.eval(&inputs) - (1.01 + 0.02 + 0.008)).abs() < 1e-6, "Default sources stay");
        inputs.0[Source::Bass as usize] = 100.0;
        assert_eq!(p.motion.zoom.eval(&inputs), 1.25, "Kept inside the range");
        assert_eq!(p.waves[0].unwrap().kind, WaveKind::Line);
        assert_eq!(p.waves[0].unwrap().set.size.base, 0.5);
        assert_eq!(p.shapes[1].unwrap().set.sides.base, 5.0);
        // A layer's settings may come before its kind line.
        assert!(Preset::parse("wave3.size = 0.4\nwave3 = xy").is_ok());
    }

    #[test]
    fn inputs_scale_with_the_recipe_strengths() {
        let mut a = crate::analysis::Analyzer::new();
        a.frame.music.bands = [0.5, 0.5, 0.5];
        a.frame.music.rel = [1.0, 2.0, 3.0];
        let p = Preset { bass: 2.0, treble: 0.0, ..Preset::default() };
        let inputs = Inputs::new(&a.frame, &p, 3.0);
        assert_eq!(inputs.0[Source::Bass as usize], 1.0);
        assert_eq!(inputs.0[Source::Treble as usize], 0.0);
        assert_eq!(inputs.0[Source::MidsRel as usize], 2.0);
        assert!(inputs.0[Source::Drift1 as usize..].iter().all(|v| (-1.0..=1.0).contains(v)));
    }
}
