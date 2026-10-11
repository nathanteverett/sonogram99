//! Saved shortcut assignments and the catalogue shared by mouse and keyboard input.

use std::path::{Path, PathBuf};

use minifb::Key;

use crate::canvas::{Canvas, Rgb};
use crate::modes;
use crate::palette::{BEVEL_HI, BEVEL_LO, CHASSIS, GOLD, LABEL_DIM, SCREEN};

/// The visuals come first in `ENTRIES`, in the order of `modes::all`. Functions follow.
pub const VISUALS: usize = modes::COUNT;
/// The number keys 1 to 9, then 0.
const SLOTS: usize = 10;
const FORMAT_PREFIX: &str = "# Shortcut format:";
const FORMAT: u32 = 6;
const SLOT_KEYS: [Key; SLOTS] = [Key::Key1, Key::Key2, Key::Key3, Key::Key4, Key::Key5,
    Key::Key6, Key::Key7, Key::Key8, Key::Key9, Key::Key0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Visual(usize),
    Function(Key),
    Rendering(crate::rendering::Preference),
}

pub struct Entry {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub action: Action,
}

pub const ENTRIES: &[Entry] = &[
    Entry { id: "bars", name: "Bars", description: "Spectrum with LED segments and peak caps.", action: Action::Visual(0) },
    Entry { id: "scope", name: "Scope", description: "An oscilloscope following the waveform.", action: Action::Visual(1) },
    Entry { id: "sonogram", name: "Sonogram", description: "A scrolling map of pitch and loudness.", action: Action::Visual(2) },
    Entry { id: "halo", name: "Halo", description: "A radial spectrum with trails of light.", action: Action::Visual(3) },
    Entry { id: "vector", name: "Vector", description: "Stereo motion and phase correlation.", action: Action::Visual(4) },
    Entry { id: "tunnel", name: "Tunnel", description: "Waveform rings spiral into a glowing tunnel.", action: Action::Visual(5) },
    Entry { id: "orbs", name: "Orbs", description: "Fluid luminous forms merge and bend with music.", action: Action::Visual(6) },
    Entry { id: "kaleidoscope", name: "Kaleidoscope", description: "Mirror tube of random glass, full screen.", action: Action::Visual(7) },
    Entry { id: "wireframe", name: "Wireframe", description: "3D wire solids that change shape and swap places.", action: Action::Visual(8) },
    Entry { id: "mandala", name: "Mandala", description: "Still kaleidoscope rosette, centered, full screen.", action: Action::Visual(9) },
    Entry { id: "gravity-waves", name: "Gravity Waves", description: "Black holes collide; bass pulls them together.", action: Action::Visual(10) },
    Entry { id: "synthwave", name: "Synthwave", description: "Ride a neon grid toward a sun; peaks follow the music.", action: Action::Visual(11) },
    Entry { id: "next-visual", name: "Next visual", description: "Move forward through all twelve visuals.", action: Action::Function(Key::Right) },
    Entry { id: "previous-visual", name: "Previous visual", description: "Move backward through all twelve visuals.", action: Action::Function(Key::Left) },
    Entry { id: "auto", name: "Toggle auto", description: "Change visual on a beat about every 15 s.", action: Action::Function(Key::A) },
    Entry { id: "recipe", name: "Next recipe", description: "Cycle recipes while in Tunnel or Orbs.", action: Action::Function(Key::R) },
    Entry { id: "skin", name: "Next skin", description: "Cycle 14 skins with deep gradients and colored backgrounds.", action: Action::Function(Key::S) },
    Entry { id: "input", name: "Next input", description: "Cycle system audio, microphone and test.", action: Action::Function(Key::I) },
    Entry { id: "pixel", name: "Pixel size", description: "Cycle pixel sizes from 1 through 4.", action: Action::Function(Key::P) },
    Entry { id: "help", name: "Toggle help", description: "Show or hide the shortcut help strip.", action: Action::Function(Key::H) },
    Entry { id: "previous-track", name: "Previous track", description: "Send Previous to the current media app.", action: Action::Function(Key::Z) },
    Entry { id: "play", name: "Play", description: "Send Play to the current media app.", action: Action::Function(Key::X) },
    Entry { id: "pause", name: "Pause / resume", description: "Toggle playback in the current media app.", action: Action::Function(Key::C) },
    Entry { id: "stop", name: "Stop", description: "Stop playback, or pause if unsupported.", action: Action::Function(Key::V) },
    Entry { id: "next-track", name: "Next track", description: "Send Next to the current media app.", action: Action::Function(Key::B) },
    Entry { id: "shuffle", name: "Shuffle visual", description: "Try a new variation of the current visual.", action: Action::Function(Key::N) },
    Entry { id: "reset", name: "Reset visual", description: "Restore this visual's original variation.", action: Action::Function(Key::Backspace) },
    Entry { id: "fullscreen", name: "Toggle fullscreen", description: "Fill the monitor. Esc returns to the window.", action: Action::Function(Key::F11) },
    Entry { id: "slower", name: "Slower patterns", description: "Lower motion and pattern rate, down to 0.25x.", action: Action::Function(Key::Down) },
    Entry { id: "faster", name: "Faster patterns", description: "Raise motion and pattern rate, up to 2x.", action: Action::Function(Key::Up) },
    Entry { id: "pulse", name: "Toggle pulse", description: "Turn beat and attack accents on or off.", action: Action::Function(Key::T) },
    Entry { id: "miniaturize", name: "Miniaturize", description: "Floating Halo. Drag center; double/right-click to restore.", action: Action::Function(Key::F12) },
    Entry { id: "renderer-auto", name: "Rendering: Automatic", description: "Use GPU acceleration when available. Saved automatically.", action: Action::Rendering(crate::rendering::Preference::Auto) },
    Entry { id: "renderer-software", name: "Rendering: Software", description: "Use the CPU renderer. Saved automatically.", action: Action::Rendering(crate::rendering::Preference::Software) },
];

/// The position of an entry in `ENTRIES`, from its saved id.
pub fn index(id: &str) -> Option<usize> {
    ENTRIES.iter().position(|entry| entry.id == id)
}

pub fn number_key(key: Key) -> Option<usize> {
    match key {
        Key::Key1 | Key::NumPad1 => Some(0),
        Key::Key2 | Key::NumPad2 => Some(1),
        Key::Key3 | Key::NumPad3 => Some(2),
        Key::Key4 | Key::NumPad4 => Some(3),
        Key::Key5 | Key::NumPad5 => Some(4),
        Key::Key6 | Key::NumPad6 => Some(5),
        Key::Key7 | Key::NumPad7 => Some(6),
        Key::Key8 | Key::NumPad8 => Some(7),
        Key::Key9 | Key::NumPad9 => Some(8),
        Key::Key0 | Key::NumPad0 => Some(9),
        _ => None,
    }
}

/// The digit on a number slot's key: 1 to 9, then 0.
pub fn slot_name(slot: usize) -> String {
    ((slot + 1) % 10).to_string()
}

const LETTER_KEYS: [Key; 26] = [Key::A, Key::B, Key::C, Key::D, Key::E, Key::F,
    Key::G, Key::H, Key::I, Key::J, Key::K, Key::L, Key::M, Key::N, Key::O, Key::P,
    Key::Q, Key::R, Key::S, Key::T, Key::U, Key::V, Key::W, Key::X, Key::Y, Key::Z];

fn key_name(key: Key) -> String {
    if let Some(slot) = number_key(key) { return slot_name(slot); }
    format!("{key:?}")
}

fn editable_keys() -> impl Iterator<Item = Key> {
    LETTER_KEYS.into_iter().filter(|key| *key != Key::M)
        .chain([Key::Left, Key::Right, Key::Up, Key::Down, Key::Tab, Key::Space, Key::Backspace,
            Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6,
            Key::F7, Key::F8, Key::F9, Key::F10, Key::F11, Key::F12])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings(pub [Option<usize>; SLOTS], Vec<(Key, usize)>);

impl Default for Bindings {
    fn default() -> Self {
        let mut extra: Vec<_> = ENTRIES.iter().enumerate().filter_map(|(i, entry)| {
            if let Action::Function(key) = entry.action { Some((key, i)) } else { None }
        }).collect();
        // Tab is a second key for the first function, Next visual. The number keys hold the
        // first ten visuals, so the eleventh and twelfth have letters.
        extra.push((Key::Tab, VISUALS));
        extra.push((Key::G, index("gravity-waves").unwrap()));
        extra.push((Key::W, index("synthwave").unwrap()));
        extra.sort_by_key(|(key, _)| *key);
        Self(std::array::from_fn(Some), extra)
    }
}

impl Bindings {
    pub fn entry(&self, key: Key) -> Option<usize> {
        if let Some(slot) = number_key(key) { return self.0[slot]; }
        self.1.iter().find(|(bound, _)| *bound == key).map(|(_, entry)| *entry)
    }

    fn set(&mut self, key: Key, entry: Option<usize>) {
        if let Some(slot) = number_key(key) { self.0[slot] = entry; return; }
        self.1.retain(|(bound, _)| *bound != key);
        if let Some(entry) = entry { self.1.push((key, entry)); }
        self.1.sort_by_key(|(key, _)| *key);
    }

    fn clear(&mut self, entry: usize) {
        for slot in &mut self.0 { if *slot == Some(entry) { *slot = None; } }
        self.1.retain(|(_, bound)| *bound != entry);
    }

    pub fn labels(&self, entry: usize) -> String {
        let mut labels: Vec<_> = self.0.iter().enumerate().filter(|(_, e)| **e == Some(entry))
            .map(|(slot, _)| slot_name(slot)).collect();
        labels.extend(self.1.iter().filter(|(_, e)| *e == entry).map(|(key, _)| key_name(*key)));
        if labels.is_empty() { "-".into() } else { labels.join(",") }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut bindings = Self::default();
        let mut seen = std::collections::HashSet::new();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let (key, id) = line.split_once('=').ok_or("Expected key = action")?;
            let key = key.trim();
            let key = SLOT_KEYS.into_iter().chain(editable_keys())
                .find(|candidate| key_name(*candidate).eq_ignore_ascii_case(key)).ok_or("Invalid key")?;
            if !seen.insert(key) { return Err("Repeated key".into()); }
            let entry = if id.trim() == "none" { None } else {
                Some(index(id.trim()).ok_or("Unknown action")?)
            };
            bindings.set(key, entry);
        }
        // Older writers saved each unused key as none, including keys that later became
        // defaults: F11 (2), T (3), G (4), F12 (5), W (6). Give a file from before
        // a default its new key only when neither the key nor its action has a custom
        // binding. Files in that format or later keep deliberately cleared keys.
        let format = text.lines().find_map(|line| line.trim().strip_prefix(FORMAT_PREFIX))
            .and_then(|n| n.trim().parse::<u32>().ok()).unwrap_or(0);
        for (since, key, id) in [(2, Key::F11, "fullscreen"), (3, Key::T, "pulse"), (4, Key::G, "gravity-waves"), (5, Key::F12, "miniaturize"), (6, Key::W, "synthwave")] {
            let action = index(id).unwrap();
            if format < since && bindings.entry(key).is_none() && bindings.labels(action) == "-" {
                bindings.set(key, Some(action));
            }
        }
        Ok(bindings)
    }

    fn text(&self) -> String {
        let mut text = format!("# Sonograph 99 shortcut assignments\n{FORMAT_PREFIX} {FORMAT}\n");
        for (slot, entry) in self.0.iter().enumerate() {
            text.push_str(&format!("{} = {}\n", slot_name(slot), entry.map_or("none", |i| ENTRIES[i].id)));
        }
        // Include unassigned keys so removed defaults stay removed after a restart.
        for key in editable_keys() {
            text.push_str(&format!("{} = {}\n", key_name(key), self.entry(key).map_or("none", |i| ENTRIES[i].id)));
        }
        text
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        let save = || -> std::io::Result<()> {
            if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
            let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
            let result = (|| {
                let mut file = std::fs::File::create(&temporary)?;
                file.write_all(self.text().as_bytes())?;
                file.sync_all()?;
                drop(file);
                std::fs::rename(&temporary, path)
            })();
            if result.is_err() { let _ = std::fs::remove_file(&temporary); }
            result
        };
        save().map_err(|e| e.to_string())
    }
}

pub fn config_path() -> Option<PathBuf> {
    let root = std::env::var_os("APPDATA").map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")));
    root.map(|p| p.join("Sonograph99").join("hotkeys.conf"))
}

#[derive(Default)]
pub struct Catalogue {
    pub renderer: String,
    pub open: bool,
    pub selected: usize,
    pub bindings: Bindings,
    pub status: String,
    pub path: Option<PathBuf>,
    pub rebinding: bool,
    offset: usize,
}

#[derive(Clone, Copy)]
pub enum Hit { Close, Tab(bool), Select(usize), Bind(usize), Rebind, Run, Clear, Defaults }

// The same layout drives drawing and mouse hit testing, including small windows.
struct Layout { x: i32, y: i32, w: i32, h: i32, rows: usize }
impl Layout {
    fn new(w: usize, h: usize) -> Self {
        let pw = (w as i32 - 8).clamp(232, 540);
        let ph = (h as i32 - 8).clamp(112, 322);
        Self { x: (w as i32 - pw) / 2, y: (h as i32 - ph) / 2, w: pw, h: ph, rows: ((ph - 100) / 14).max(1) as usize }
    }
}

impl Catalogue {
    pub fn load(&mut self) {
        self.path = config_path();
        if let Some(path) = &self.path {
            match Bindings::load(path) {
                Ok(bindings) => self.bindings = bindings,
                Err(e) => {
                    eprintln!("Could not load {}: {e}", path.display());
                    self.status = "Could not load hotkeys. Using defaults.".into();
                }
            }
        }
    }

    fn save(&mut self, message: String) {
        self.status = match &self.path {
            Some(path) => match self.bindings.save(path) {
                Ok(()) => format!("Saved: {message}"),
                Err(e) => {
                    eprintln!("Could not save {}: {e}", path.display());
                    "Save failed. Active this session.".into()
                }
            },
            None => "Session only: no settings folder.".into(),
        };
    }

    pub fn bind(&mut self, slot: usize) {
        self.bind_key(SLOT_KEYS[slot]);
    }

    pub fn bind_key(&mut self, key: Key) {
        if number_key(key).is_none() && !editable_keys().any(|candidate| candidate == key) {
            self.status = "Use 0-9, a letter, arrow or F1-F12.".into();
            return;
        }
        // A function has one shortcut after reassignment. Its old fixed letter must
        // not continue to execute it or overwrite the new shortcut through recursion.
        if !matches!(ENTRIES[self.selected].action, Action::Visual(_)) {
            self.bindings.clear(self.selected);
        }
        self.bindings.set(key, Some(self.selected));
        self.rebinding = false;
        self.save(format!("{} = {}", key_name(key), ENTRIES[self.selected].name));
    }

    pub fn start_rebind(&mut self) {
        self.rebinding = true;
        self.status = "Press new key. Esc cancels; M reserved.".into();
    }

    pub fn clear(&mut self) {
        self.bindings.clear(self.selected);
        self.rebinding = false;
        self.save(format!("{} unassigned", ENTRIES[self.selected].name));
    }

    pub fn defaults(&mut self) {
        self.bindings = Bindings::default();
        self.rebinding = false;
        self.save("Default shortcuts restored".into());
    }

    pub fn tab(&mut self, functions: bool) {
        self.selected = if functions { VISUALS } else { 0 };
        self.offset = 0;
        self.status.clear();
        self.rebinding = false;
    }

    fn range(&self) -> std::ops::Range<usize> {
        if self.selected < VISUALS { 0..VISUALS } else { VISUALS..ENTRIES.len() }
    }

    pub fn navigate(&mut self, delta: i32) {
        let range = self.range();
        self.selected = (self.selected as i32 + delta).clamp(range.start as i32, range.end as i32 - 1) as usize;
        self.status.clear();
    }

    fn ensure_visible(&mut self, rows: usize) {
        let relative = self.selected - self.range().start;
        self.offset = self.offset.min(relative).max((relative + 1).saturating_sub(rows));
        self.offset = self.offset.min(self.range().len().saturating_sub(rows));
    }

    pub fn hit(&mut self, w: usize, h: usize, x: i32, y: i32) -> Option<Hit> {
        let l = Layout::new(w, h);
        self.ensure_visible(l.rows);
        let (x, y) = (x - l.x, y - l.y);
        if x < 0 || x >= l.w || y < 0 || y >= l.h { return None; }
        if y < 19 && x >= l.w - 48 { return Some(Hit::Close); }
        if (21..34).contains(&y) {
            if x < 84 { return Some(Hit::Tab(false)); }
            if x < 162 { return Some(Hit::Tab(true)); }
        }
        if (38..51).contains(&y) && (8..l.w - 8).contains(&x) {
            let n = SLOTS as i32;
            let slot = (0..n).find(|slot| x < 8 + (slot + 1) * (l.w - 16) / n).unwrap_or(n - 1);
            return Some(Hit::Bind(slot as usize));
        }
        if (56..56 + l.rows as i32 * 14).contains(&y) {
            let entry = self.range().start + self.offset + ((y - 56) / 14) as usize;
            if entry < self.range().end {
                if entry == self.selected && x >= l.w - 72 { return Some(Hit::Rebind); }
                return Some(Hit::Select(entry));
            }
        }
        if (l.h - 17..l.h - 3).contains(&y) {
            return Some(if x < 78 { Hit::Rebind } else if x < 156 { Hit::Clear } else { Hit::Defaults });
        }
        None
    }

    pub fn draw(&mut self, c: &mut Canvas, ink: Rgb) {
        let l = Layout::new(c.w, c.h);
        self.ensure_visible(l.rows);
        c.rect_blend(0, 0, c.w as i32, c.h as i32, SCREEN, 0.78);
        c.rect(l.x + 3, l.y + 3, l.w, l.h, BEVEL_LO);
        c.rect(l.x, l.y, l.w, l.h, CHASSIS);
        c.rect(l.x, l.y, l.w, 1, BEVEL_HI);
        c.rect(l.x, l.y, 1, l.h, BEVEL_HI);
        let (x, y, right) = (l.x + 8, l.y, l.x + l.w - 8);
        c.text("Catalogue / Hotkeys", x, y + 7, GOLD);
        c.text_right("Esc [x]", right, y + 7, LABEL_DIM);
        for (start, label, selected) in [(0, "Visuals", self.selected < VISUALS), (78, "Functions", self.selected >= VISUALS)] {
            if selected { c.rect(x + start - 2, y + 21, 74, 13, SCREEN); }
            c.text(label, x + start + 2, y + 24, if selected { ink } else { LABEL_DIM });
        }
        c.text_right(&format!("{}/{}", self.selected - self.range().start + 1, self.range().len()), right, y + 24, LABEL_DIM);
        let n = SLOTS as i32;
        for slot in 0..n {
            let left = x + slot * (l.w - 16) / n;
            let end = x + (slot + 1) * (l.w - 16) / n;
            let assigned = self.bindings.0[slot as usize] == Some(self.selected);
            c.rect(left, y + 38, end - left - 2, 13, if assigned { ink } else { SCREEN });
            c.text_center(&slot_name(slot as usize), (left + end - 2) / 2, y + 41, if assigned { SCREEN } else { LABEL_DIM });
        }
        for row in 0..l.rows {
            let i = self.range().start + self.offset + row;
            if i >= self.range().end { break; }
            let yy = y + 56 + row as i32 * 14;
            let selected = i == self.selected;
            if selected { c.rect(x - 2, yy, l.w - 12, 13, SCREEN); }
            c.text(if selected { ">" } else { " " }, x, yy + 3, ink);
            c.text(ENTRIES[i].name, x + 10, yy + 3, if selected { ink } else { GOLD });
            let keys = self.bindings.labels(i);
            c.text_right(&keys, right, yy + 3, if selected { ink } else { LABEL_DIM });
        }
        let clip = (x, right);
        let detail = if !self.status.is_empty() { &self.status }
            else if matches!(ENTRIES[self.selected].action, Action::Rendering(_)) && !self.renderer.is_empty() { &self.renderer }
            else { ENTRIES[self.selected].description };
        c.text_clipped(detail, x, y + l.h - 40, ink, clip);
        c.text_clipped("Keys assign  Space run  Arrows select", x, y + l.h - 29, LABEL_DIM, clip);
        c.text("Enter:bind", x, y + l.h - 13, GOLD);
        c.text("Del: clear", x + 70, y + l.h - 13, GOLD);
        c.text("D: defaults", x + 148, y + l.h - 13, GOLD);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miniaturize_migration_preserves_custom_and_cleared_shortcuts() {
        let mini = index("miniaturize").unwrap();
        let migrated = Bindings::parse("# Shortcut format: 4\nF12 = none").unwrap();
        assert_eq!(migrated.entry(Key::F12), Some(mini));
        assert_eq!(Bindings::parse(&migrated.text()).unwrap(), migrated);
        assert_eq!(Bindings::parse("F12 = skin").unwrap().entry(Key::F12), index("skin"));
        let custom = Bindings::parse("# Shortcut format: 4\nF12 = none\nF2 = miniaturize").unwrap();
        assert_eq!(custom.entry(Key::F12), None);
        assert_eq!(custom.entry(Key::F2), Some(mini));
        assert_eq!(Bindings::parse("# Shortcut format: 5\nF12 = none").unwrap().entry(Key::F12), None);
    }

    #[test]
    fn pulse_migration_preserves_custom_and_deliberately_cleared_keys() {
        let pulse = index("pulse").unwrap();
        let old = "# Shortcut format: 2\nT = none\nF11 = none";
        let migrated = Bindings::parse(old).unwrap();
        assert_eq!(migrated.entry(Key::T), Some(pulse));
        assert_eq!(migrated.entry(Key::F11), None, "Keep previously cleared fullscreen");
        assert_eq!(Bindings::parse(&migrated.text()).unwrap(), migrated);
        assert_eq!(Bindings::parse("T = skin").unwrap().entry(Key::T), index("skin"));
        let custom = Bindings::parse("T = none\nF2 = pulse").unwrap();
        assert_eq!(custom.entry(Key::T), None);
        assert_eq!(custom.entry(Key::F2), Some(pulse));
        let mut cleared = migrated;
        cleared.clear(pulse);
        assert_eq!(Bindings::parse(&cleared.text()).unwrap().entry(Key::T), None);
    }

    #[test]
    fn legacy_unused_f11_gains_fullscreen_without_overwriting_custom_bindings() {
        let fullscreen = index("fullscreen").unwrap();
        let old = "# Sonograph 99 shortcut assignments\n1 = bars\nF11 = none\nF2 = none";
        let migrated = Bindings::parse(old).unwrap();
        assert_eq!(migrated.entry(Key::F11), Some(fullscreen));
        assert_eq!(Bindings::parse(&migrated.text()).unwrap(), migrated);
        assert_eq!(Bindings::parse("F11 = skin").unwrap().entry(Key::F11), index("skin"));
        let custom = Bindings::parse("F11 = none\nF2 = fullscreen").unwrap();
        assert_eq!(custom.entry(Key::F11), None);
        assert_eq!(custom.entry(Key::F2), Some(fullscreen));
        let mut cleared = migrated;
        cleared.clear(fullscreen);
        assert_eq!(Bindings::parse(&cleared.text()).unwrap().entry(Key::F11), None);
    }

    fn id(name: &str) -> usize {
        index(name).unwrap()
    }

    #[test]
    fn visual_entries_match_the_modes_in_order() {
        let modes = modes::all();
        assert_eq!(modes.len(), VISUALS);
        for (i, entry) in ENTRIES.iter().enumerate() {
            match entry.action {
                Action::Visual(mode) => {
                    assert_eq!(mode, i, "{} is out of order", entry.id);
                    assert_eq!(entry.name.to_uppercase(), modes[mode].name());
                }
                Action::Function(_) | Action::Rendering(_) => assert!(i >= VISUALS, "{} is among the visuals", entry.id),
            }
        }
        let bindings = Bindings::default();
        assert_eq!(bindings.entry(Key::Key0), Some(id("mandala")));
        assert_eq!(bindings.entry(Key::NumPad0), Some(id("mandala")));
        assert_eq!(bindings.labels(id("mandala")), "0");
        assert_eq!(bindings.entry(Key::G), Some(id("gravity-waves")));
        assert_eq!(bindings.labels(id("gravity-waves")), "G");
        assert_eq!(bindings.entry(Key::W), Some(id("synthwave")));
        assert_eq!(bindings.labels(id("synthwave")), "W");
    }

    #[test]
    fn gravity_waves_gains_g_only_where_g_is_free() {
        let gravity = id("gravity-waves");
        let old = "# Shortcut format: 3\nG = none\nT = none";
        let migrated = Bindings::parse(old).unwrap();
        assert_eq!(migrated.entry(Key::G), Some(gravity));
        assert_eq!(migrated.entry(Key::T), None, "Keep a cleared key from format 3");
        assert_eq!(Bindings::parse(&migrated.text()).unwrap(), migrated);
        assert_eq!(Bindings::parse("# Shortcut format: 3\nG = skin").unwrap().entry(Key::G), index("skin"));
        let moved = Bindings::parse("# Shortcut format: 3\nG = none\nF3 = gravity-waves").unwrap();
        assert_eq!(moved.entry(Key::G), None);
        assert_eq!(moved.entry(Key::F3), Some(gravity));
        let mut cleared = migrated;
        cleared.clear(gravity);
        assert_eq!(Bindings::parse(&cleared.text()).unwrap().entry(Key::G), None);
    }

    #[test]
    fn synthwave_gains_w_only_where_w_is_free() {
        let synthwave = id("synthwave");
        let migrated = Bindings::parse("# Shortcut format: 5\nW = none").unwrap();
        assert_eq!(migrated.entry(Key::W), Some(synthwave));
        assert_eq!(Bindings::parse("# Shortcut format: 5\nW = skin").unwrap().entry(Key::W), index("skin"));
        let moved = Bindings::parse("# Shortcut format: 5\nW = none\nF4 = synthwave").unwrap();
        assert_eq!(moved.entry(Key::W), None);
        assert_eq!(moved.entry(Key::F4), Some(synthwave));
        assert_eq!(Bindings::parse("# Shortcut format: 6\nW = none").unwrap().entry(Key::W), None);
    }

    #[test]
    fn bindings_round_trip_and_reject_bad_config() {
        let mut bindings = Bindings::default();
        bindings.0[0] = Some(id("auto"));
        bindings.0[7] = None;
        bindings.0[9] = Some(id("bars"));
        assert_eq!(Bindings::parse(&bindings.text()).unwrap(), bindings);
        for bad in ["11 = bars", "10 = bars", "1 = unknown", "1 = bars\n1 = scope", "0 = bars\n0 = scope", "2 bars"] {
            assert!(Bindings::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn assignments_replace_slots_clear_and_restore() {
        let auto = id("auto");
        let mut menu = Catalogue { selected: auto, ..Default::default() };
        menu.bind(0);
        menu.bind(1);
        assert_eq!(menu.bindings.0[..2], [None, Some(auto)]);
        assert_eq!(menu.bindings.entry(Key::A), None, "Reassigning a function removes its previous letter");
        menu.clear();
        assert_eq!(menu.bindings.0[..2], [None, None]);
        menu.defaults();
        assert_eq!(menu.bindings, Bindings::default());
    }

    #[test]
    fn scrolling_and_hit_testing_follow_selection_at_all_sizes() {
        for (w, h) in [(240, 120), (320, 180), (640, 360), (1280, 720)] {
            let mut menu = Catalogue::default();
            menu.tab(true);
            menu.navigate(99);
            let mut c = Canvas::new(w, h);
            menu.draw(&mut c, GOLD);
            let l = Layout::new(w, h);
            let row = menu.selected - menu.range().start - menu.offset;
            assert!(matches!(menu.hit(w, h, l.x + 20, l.y + 59 + row as i32 * 14), Some(Hit::Select(i)) if i == ENTRIES.len() - 1));
            assert!(matches!(menu.hit(w, h, l.x + l.w - 10, l.y + 40), Some(Hit::Bind(9))));
            assert!(matches!(menu.hit(w, h, l.x + 10, l.y + 40), Some(Hit::Bind(0))));
        }
    }

    #[test]
    fn saved_assignments_survive_reloading_and_replacement() {
        let dir = std::env::temp_dir().join(format!("sonograph-hotkeys-test-{}", std::process::id()));
        let path = dir.join("hotkeys.conf");
        let mut bindings = Bindings::default();
        bindings.save(&path).unwrap();
        bindings.0[8] = Some(id("auto"));
        bindings.clear(id("skin"));
        bindings.set(Key::F2, Some(id("skin")));
        bindings.save(&path).unwrap();
        assert_eq!(Bindings::load(&path).unwrap(), bindings);
        assert_eq!(Bindings::load(&path).unwrap().entry(Key::S), None);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn legacy_number_only_settings_keep_function_defaults() {
        let legacy = Bindings::parse("1 = bars\n9 = auto").unwrap();
        assert_eq!(legacy.entry(Key::S), Some(id("skin")));
        assert_eq!(legacy.entry(Key::NumPad9), Some(id("auto")));
        assert_eq!(legacy.entry(Key::Key0), Some(id("mandala")), "Files from before Mandala get it on 0");
        assert_eq!(legacy.entry(Key::G), Some(id("gravity-waves")));
        for (entry, item) in ENTRIES.iter().enumerate().skip(VISUALS) {
            if let Action::Function(key) = item.action {
                assert_eq!(legacy.entry(key), Some(entry));
            } else {
                assert!(matches!(item.action, Action::Rendering(_)));
                assert_eq!(legacy.labels(entry), "-", "Rendering settings have no default hotkey");
            }
        }
    }
}
