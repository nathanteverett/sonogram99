//! The visual engine shared by the desktop app, the offline renderer, and the web build:
//! the twelve modes, the transitions between them, auto mode, recipes, and each visual's
//! mouse controls.

use crate::analysis::Frame;
use crate::canvas::{pack, Canvas, Rgb};
use crate::interaction::{Controls, Rng};
use crate::modes::{self, Mode};
use crate::palette::{Palette, SCREEN};
use crate::preset::Preset;
use crate::transition::{Pattern, Transition, PATTERNS, SECONDS};

/// Auto mode changes the visual about this often, give or take 30%.
pub const AUTO_SECONDS: f32 = 15.0;
/// How long auto mode waits for a beat after a change is due, before it changes anyway.
pub const AUTO_GRACE: f32 = 4.0;
/// A beat at least this strong cuts straight to the next visual.
pub const CUT_BEAT: f32 = 0.7;
/// The fade when auto mode changes on a softer beat.
pub const BEAT_FADE: f32 = 1.2;

/// User-selectable visual clock multipliers. Audio keeps its real-time clock.
pub const RATES: [(f32, &str); 5] = [(0.25, "0.25X"), (0.5, "0.5X"), (1.0, "1X"), (1.5, "1.5X"), (2.0, "2X")];

/// Changes the visual on a beat, about every 15 seconds.
pub struct Auto {
    pub on: bool,
    /// Seconds since the visual last changed.
    pub t: f32,
    /// When the next change is due.
    pub due: f32,
    rng: Rng,
}

impl Auto {
    fn new(on: bool, seed: u32) -> Self {
        let mut auto = Auto { on, t: 0.0, due: AUTO_SECONDS, rng: Rng::new(seed) };
        auto.restart();
        auto
    }

    /// Starts timing again and picks when the next change is due.
    pub fn restart(&mut self) {
        self.t = 0.0;
        self.due = AUTO_SECONDS * (0.7 + 0.6 * self.rng.unit());
    }

    /// Advances the clock by `dt`. When it is time to change, returns the fade length:
    /// none for a strong beat, short for a softer one, full length if no beat came.
    pub fn step(&mut self, beat: f32, dt: f32) -> Option<f32> {
        if !self.on {
            return None;
        }
        self.t += dt.max(0.0);
        if self.t < self.due {
            None
        } else if beat >= CUT_BEAT {
            Some(0.0)
        } else if beat > 0.0 {
            Some(BEAT_FADE)
        } else if self.t >= self.due + AUTO_GRACE {
            Some(SECONDS)
        } else {
            None
        }
    }
}

pub struct Show {
    pub modes: Vec<Box<dyn Mode>>,
    pub mode: usize,
    /// Each visual keeps its own seed and mouse adjustments.
    pub controls: Vec<Controls>,
    pub preset: Preset,
    /// The built-in recipe in use. None for a recipe from a file under its own name.
    pub recipe: Option<usize>,
    /// The current mode's own picture.
    pub viz: Canvas,
    /// What the viewer sees: the picture, or a blend during a transition.
    pub composite: Canvas,
    pub transition: Option<Transition>,
    pub auto: Auto,
    /// Enable beat and attack accents without changing audio analysis or auto timing.
    pub pulse: bool,
    rate_index: usize,
    rng: Rng,
    last_pattern: Pattern,
    background: Rgb,
}

impl Show {
    /// `seed` sets the random choices: transition patterns and auto mode's timing.
    pub fn new(mode: usize, preset: Preset, auto: bool, seed: u32) -> Self {
        let mut modes = modes::all();
        for m in &mut modes {
            m.configure(&preset);
        }
        Show {
            controls: vec![Controls::new(preset.seed); modes.len()],
            mode: mode % modes.len(),
            modes,
            recipe: Preset::builtin_index(&preset.name),
            preset,
            viz: Canvas::new(1, 1),
            composite: Canvas::new(1, 1),
            transition: None,
            auto: Auto::new(auto, seed ^ 0xA5A5_5A5A),
            pulse: true,
            rate_index: 2,
            rng: Rng::new(seed),
            last_pattern: Pattern::Fade,
            background: SCREEN,
        }
    }

    /// New random choices, for example from the clock, so that sessions differ.
    pub fn reseed(&mut self, seed: u32) {
        self.rng = Rng::new(seed);
        self.auto.rng = Rng::new(seed ^ 0xA5A5_5A5A);
        self.auto.restart();
    }

    pub fn name(&self) -> &'static str {
        self.modes[self.mode].name()
    }

    pub fn rate_index(&self) -> usize { self.rate_index }

    /// Change pace without restarting the visual, transition, or auto timer.
    pub fn set_rate(&mut self, index: usize) {
        if index < RATES.len() { self.rate_index = index; }
    }

    pub fn step_rate(&mut self, delta: i32) {
        self.rate_index = (self.rate_index as i32).saturating_add(delta).clamp(0, RATES.len() as i32 - 1) as usize;
    }

    /// Tunnel, Orbs, Kaleidoscope, Mandala and Gravity Waves respond to the mouse and have
    /// seeds.
    pub fn interactive(&self) -> bool {
        matches!(self.mode, 5..=7 | 9 | 10)
    }

    /// Tunnel and Orbs follow the recipe.
    pub fn uses_recipe(&self) -> bool {
        matches!(self.mode, 5 | 6)
    }

    /// The viewer's own change of visual: a full-length transition in a random pattern.
    pub fn set_mode(&mut self, i: usize) {
        self.switch(i, SECONDS);
    }

    /// Changes the visual with a transition of `seconds`. 0 is a hard cut.
    pub fn switch(&mut self, i: usize, seconds: f32) {
        let next = i % self.modes.len();
        if next == self.mode {
            return;
        }
        self.controls[self.mode].pointer = None;
        self.transition = if self.viz.w > 1 && self.viz.h > 1 && seconds > 0.0 {
            let mut from = Canvas::new(self.viz.w, self.viz.h);
            // An interrupted transition starts again from exactly what was on screen.
            let outgoing = if self.transition.is_some() {
                from.px.clone_from(&self.composite.px);
                None
            } else {
                from.px.clone_from(&self.viz.px);
                Some(self.mode)
            };
            let pattern = self.next_pattern();
            Some(Transition::with(from, outgoing, pattern, seconds, self.rng.next_u32()))
        } else {
            None
        };
        self.mode = next;
        self.auto.restart();
        self.modes[self.mode].reset(&mut self.viz);
        self.viz.fill(self.background);
    }

    /// Any pattern but the last one.
    fn next_pattern(&mut self) -> Pattern {
        let others: Vec<Pattern> = PATTERNS.into_iter().filter(|p| *p != self.last_pattern).collect();
        self.last_pattern = others[self.rng.below(others.len())];
        self.last_pattern
    }

    /// Uses a recipe in Tunnel and Orbs. Every visual returns to the recipe's seed.
    pub fn set_preset(&mut self, preset: Preset) {
        self.recipe = Preset::builtin_index(&preset.name);
        self.controls.fill(Controls::new(preset.seed));
        for m in &mut self.modes {
            m.configure(&preset);
        }
        self.preset = preset;
    }

    /// The next built-in recipe. A recipe from a file goes on to the first built-in one.
    pub fn next_recipe(&mut self) {
        let next = self.recipe.map_or(0, |i| (i + 1) % Preset::builtin_count());
        self.set_preset(Preset::builtin(next));
    }

    /// Shuffle picks a new seed; otherwise the recipe's own seed returns. Either way the
    /// trails and mouse adjustments clear. Returns false for visuals without a seed.
    pub fn reset_visual(&mut self, shuffle: bool) -> bool {
        if !self.interactive() {
            return false;
        }
        if shuffle {
            self.controls[self.mode].shuffle();
        } else {
            self.controls[self.mode].reset();
        }
        self.transition = None;
        self.auto.restart();
        self.modes[self.mode].interact(self.controls[self.mode]);
        self.modes[self.mode].reset(&mut self.viz);
        self.viz.fill(self.background);
        true
    }

    /// Fits the picture to the drawing area. A new size clears the trails and any transition.
    pub fn resize(&mut self, w: usize, h: usize) {
        if self.viz.resize(w.max(1), h.max(1)) {
            self.transition = None;
            self.modes[self.mode].reset(&mut self.viz);
            self.viz.fill(self.background);
        }
    }

    /// Draws one frame into `composite`. Returns true when auto mode changed the visual.
    pub fn frame(&mut self, f: &Frame, pal: &Palette, dt: f32) -> bool {
        if self.background != pal.skin.screen {
            // Recolor empty pixels immediately, keeping the current motion and light field.
            // Also update a frozen outgoing image if a transition was interrupted.
            let old = pack(self.background);
            let new = pack(pal.skin.screen);
            for canvas in [&mut self.viz, &mut self.composite] {
                for pixel in &mut canvas.px {
                    if *pixel == old { *pixel = new; }
                }
            }
            if let Some(transition) = &mut self.transition {
                for pixel in &mut transition.from.px {
                    if *pixel == old { *pixel = new; }
                }
            }
            self.background = pal.skin.screen;
        }
        let dt = dt.clamp(0.0, 0.1) * RATES[self.rate_index].0;
        let change = self.auto.step(f.music.beat, dt);
        if let Some(seconds) = change {
            self.switch(self.mode + 1, seconds);
        }
        // Auto keeps the original beat clock. Only the picture loses its accents,
        // including the outgoing visual while a transition is in progress.
        let mut unpulsed;
        let f = if self.pulse { f } else {
            unpulsed = f.clone();
            unpulsed.music.beat = 0.0;
            unpulsed.music.pulse = 0.0;
            unpulsed.music.onset = 0.0;
            &unpulsed
        };
        self.composite.resize(self.viz.w, self.viz.h);
        self.modes[self.mode].interact(self.controls[self.mode]);
        self.modes[self.mode].draw(&mut self.viz, f, pal, dt);
        let finished = if let Some(transition) = &mut self.transition {
            if let Some(mode) = transition.mode {
                self.modes[mode].interact(self.controls[mode]);
                self.modes[mode].draw(&mut transition.from, f, pal, dt);
            }
            transition.blend(&self.viz, &mut self.composite, dt)
        } else {
            self.composite.px.clone_from(&self.viz.px);
            false
        };
        if finished {
            self.transition = None;
        }
        change.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{Analyzer, FFT};
    use crate::testsignal::TestSignal;

    #[test]
    fn sonogram_keeps_audio_history_between_frames_in_every_skin() {
        for skin in 0..crate::palette::SKINS.len() {
            let pal = Palette::new(skin);
            let mut audio = Analyzer::new();
            let mut show = Show::new(2, Preset::default(), false, 7);
            show.resize(96, 64);
            audio.frame.spec.fill(0.8);
            show.frame(&audio.frame, &pal, 0.1);
            let lit = show.composite.px[20 * 96 + 93];
            assert_ne!(lit, pack(pal.skin.screen));
            audio.frame.spec.fill(0.0);
            show.frame(&audio.frame, &pal, 0.1);
            assert_eq!(show.composite.px[20 * 96 + 89], lit, "{}: previous audio must scroll, not disappear", pal.skin.name);
            assert_eq!(show.composite.px[20 * 96 + 95], pack(pal.skin.screen));
        }
    }

    #[test]
    fn colored_skin_backgrounds_survive_silence_switches_resets_and_resizes() {
        let audio = Analyzer::new();
        let original = Palette::new(0);
        for skin in 4..crate::palette::SKINS.len() {
            let pal = Palette::new(skin);
            // The original eleven visuals use skin backgrounds. Synthwave has its own sky.
            for mode in 0..11 {
                let mut show = Show::new(mode, Preset::default(), false, 7);
                show.resize(96, 64);
                show.frame(&audio.frame, &original, 1.0 / 60.0);
                for _ in 0..3 { show.frame(&audio.frame, &pal, 1.0 / 60.0); }
                assert!(!show.composite.px.contains(&pack(SCREEN)), "{} / {mode}: old background", pal.skin.name);
                assert!(show.composite.px.contains(&pack(pal.skin.screen)), "{} / {mode}: selected background", pal.skin.name);
                show.reset_visual(false);
                show.resize(104, 72);
                show.frame(&audio.frame, &pal, 1.0 / 60.0);
                assert!(!show.composite.px.contains(&pack(SCREEN)), "{} / {mode}: resize/reset", pal.skin.name);
                show.set_mode((mode + 1) % 11);
                for _ in 0..3 { show.frame(&audio.frame, &pal, 1.0 / 60.0); }
                assert!(!show.composite.px.contains(&pack(SCREEN)), "{} / {mode}: transition", pal.skin.name);
            }
        }
    }

    #[test]
    fn pulse_off_filters_both_visuals_without_changing_the_source_audio() {
        let mut audio = Analyzer::new();
        audio.frame.spec.fill(0.5);
        audio.frame.peak = 0.3;
        audio.frame.bass = 0.6;
        audio.frame.music.bands = [0.6, 0.4, 0.3];
        audio.frame.music.onset = 0.9;
        audio.frame.music.beat = 0.8;
        audio.frame.music.pulse = 0.8;
        let mut filtered = audio.frame.clone();
        filtered.music.onset = 0.0;
        filtered.music.beat = 0.0;
        filtered.music.pulse = 0.0;
        let pal = Palette::new(1);
        let mut accented_modes = 0;
        for mode in 0..modes::COUNT {
            let mut off = Show::new(mode, Preset::default(), false, 7);
            let mut reference = Show::new(mode, Preset::default(), false, 7);
            let mut on = Show::new(mode, Preset::default(), false, 7);
            assert!(off.pulse);
            off.pulse = false;
            for show in [&mut off, &mut reference, &mut on] { show.resize(80, 60); }
            for step in 0..30 {
                if step == 15 {
                    if off.composite.px != on.composite.px { accented_modes += 1; }
                    for show in [&mut off, &mut reference, &mut on] {
                        show.set_mode((mode + 1) % modes::COUNT);
                    }
                    assert!(off.transition.is_some());
                }
                off.frame(&audio.frame, &pal, 1.0 / 60.0);
                reference.frame(&filtered, &pal, 1.0 / 60.0);
                on.frame(&audio.frame, &pal, 1.0 / 60.0);
                assert_eq!(off.composite.px, reference.composite.px, "mode {mode}, step {step}");
            }
        }
        assert!(accented_modes > 0, "Pulse must change the rendered picture");
        assert_eq!(audio.frame.music.onset, 0.9);
        assert_eq!(audio.frame.music.beat, 0.8);
        assert_eq!(audio.frame.music.pulse, 0.8);
    }

    #[test]
    fn pulse_setting_survives_visual_changes_and_keeps_auto_on_the_beat() {
        let mut show = Show::new(5, Preset::default(), true, 7);
        show.pulse = false;
        show.next_recipe();
        show.reset_visual(false);
        show.set_mode(8);
        assert!(!show.pulse);
        show.resize(40, 30);
        let mut audio = Analyzer::new();
        audio.frame.music.beat = 1.0;
        show.auto.t = show.auto.due;
        assert!(show.frame(&audio.frame, &Palette::new(0), 1.0 / 60.0));
        assert_eq!(show.mode, 9);
        assert!(show.transition.is_none(), "Strong beat still triggers a hard cut");
        assert!(!show.pulse);
    }

    #[test]
    fn rate_scales_rendering_auto_and_both_sides_of_transitions() {
        let audio = Analyzer::new();
        let pal = Palette::new(1);
        for mode in 0..modes::COUNT {
            for (index, &(rate, _)) in RATES.iter().enumerate() {
                let mut paced = Show::new(mode, Preset::default(), true, 7);
                let mut reference = Show::new(mode, Preset::default(), true, 7);
                paced.set_rate(index);
                paced.resize(40, 30);
                reference.resize(40, 30);
                for step in 0..40 {
                    if step == 10 {
                        paced.set_mode((mode + 1) % modes::COUNT);
                        reference.set_mode((mode + 1) % modes::COUNT);
                    }
                    paced.frame(&audio.frame, &pal, 0.04);
                    reference.frame(&audio.frame, &pal, 0.04 * rate);
                    assert_eq!(paced.composite.px, reference.composite.px, "mode {mode}, rate {rate}, frame {step}");
                    assert_eq!(paced.auto.t, reference.auto.t);
                    assert_eq!(paced.transition.is_some(), reference.transition.is_some());
                }
            }
        }
    }

    #[test]
    fn rate_changes_preserve_state_and_have_safe_limits() {
        let mut show = Show::new(5, Preset::default(), true, 7);
        show.auto.t = 3.0;
        show.set_rate(4);
        assert_eq!(show.auto.t, 3.0);
        show.next_recipe();
        show.reset_visual(false);
        show.set_mode(8);
        assert_eq!(show.rate_index(), 4);
        show.set_rate(usize::MAX);
        assert_eq!(show.rate_index(), 4);
        show.step_rate(i32::MAX);
        assert_eq!(show.rate_index(), 4);
        show.step_rate(i32::MIN);
        assert_eq!(show.rate_index(), 0);
    }

    #[test]
    fn auto_waits_for_a_beat_and_cuts_on_strong_ones() {
        let mut auto = Auto::new(true, 3);
        let mut dues = Vec::new();
        for _ in 0..20 {
            auto.restart();
            assert!((AUTO_SECONDS * 0.7..=AUTO_SECONDS * 1.3).contains(&auto.due));
            dues.push(auto.due);
            assert_eq!(auto.step(1.0, auto.due * 0.99), None, "No change before it is due, even on a beat");
            assert_eq!(auto.step(0.0, auto.due * 0.02), None, "Due, but waiting for a beat");
            assert_eq!(auto.step(0.3, 0.0), Some(BEAT_FADE));
            assert_eq!(auto.step(0.9, 0.0), Some(0.0));
            assert_eq!(auto.step(0.0, AUTO_GRACE), Some(SECONDS), "No beat: change anyway");
        }
        assert!(dues.windows(2).any(|d| (d[0] - d[1]).abs() > 1.0), "The due time varies: {dues:?}");
        auto.on = false;
        assert_eq!(auto.step(1.0, 100.0), None);
    }

    #[test]
    fn show_switches_on_beats_of_the_test_signal() {
        let mut show = Show::new(0, Preset::default(), true, 11);
        assert_eq!(show.modes.len(), modes::COUNT);
        show.resize(48, 30);
        let pal = Palette::new(0);
        let mut test = TestSignal::new(48_000.0);
        let mut analyzer = Analyzer::new();
        let (mut l, mut r) = (vec![0.0; FFT], vec![0.0; FFT]);
        let mut changes = Vec::new();
        for frame in 0..60 * 75 {
            let mut fresh = Vec::new();
            test.render(800, |a, b| fresh.push((a, b)));
            l.drain(..800);
            r.drain(..800);
            l.extend(fresh.iter().map(|s| s.0));
            r.extend(fresh.iter().map(|s| s.1));
            analyzer.process(&l, &r, 48_000.0, 1.0 / 60.0);
            let beat = analyzer.frame.music.beat;
            let (before, waited) = (show.mode, show.auto.t + 1.0 / 60.0 - show.auto.due);
            if show.frame(&analyzer.frame, &pal, 1.0 / 60.0) {
                assert_eq!(show.mode, (before + 1) % modes::COUNT);
                assert!(waited >= 0.0, "Frame {frame}: changed before it was due");
                assert!(beat > 0.0 || waited >= AUTO_GRACE - 0.02, "Frame {frame}: changed between beats");
                assert_eq!(show.transition.is_none(), beat >= CUT_BEAT, "Strong beats cut; others fade");
                changes.push(beat);
            }
        }
        assert!(changes.len() >= 3, "{changes:?}");
        assert!(changes.iter().any(|b| *b > 0.0), "Some changes land on beats: {changes:?}");
    }

    #[test]
    fn viewer_changes_use_varied_patterns_and_survive_interruption() {
        let mut show = Show::new(6, Preset::default(), false, 5);
        show.resize(40, 30);
        let silent = Analyzer::new();
        let pal = Palette::new(1);
        let mut patterns = Vec::new();
        for i in 0..8 {
            show.frame(&silent.frame, &pal, 1.0 / 60.0);
            show.set_mode(i);
            if let Some(t) = &show.transition {
                patterns.push(t.pattern);
            }
        }
        assert!(patterns.windows(2).all(|p| p[0] != p[1]), "{patterns:?}");
        let shown = show.composite.px.clone();
        show.set_mode(3);
        assert_eq!(show.transition.as_ref().unwrap().mode, None);
        assert_eq!(show.transition.as_ref().unwrap().from.px, shown);
        show.switch(4, 0.0);
        assert!(show.transition.is_none(), "A hard cut drops the transition");
        show.next_recipe();
        assert_eq!(show.recipe, Some(1));
    }
}
