//! A built-in test signal: a 16-bar synth loop (Am F C G at 112 BPM) rendered in software.
//! It goes through the same analysis as captured audio, so every mode can be checked
//! without a sound card or a streaming app.

use std::f32::consts::TAU;

const BPM: f32 = 112.0;
const BARS: usize = 16;

struct Chord {
    root: f32,
    pad: [f32; 3],
}

const PROG: [Chord; 4] = [
    Chord { root: 45.0, pad: [57.0, 60.0, 64.0] }, // A minor
    Chord { root: 41.0, pad: [53.0, 57.0, 60.0] }, // F major
    Chord { root: 48.0, pad: [55.0, 60.0, 64.0] }, // C major
    Chord { root: 43.0, pad: [55.0, 59.0, 62.0] }, // G major
];
const ARP: [usize; 16] = [0, 1, 2, 3, 4, 5, 4, 3, 1, 2, 3, 4, 5, 3, 2, 1];
const BASS: [i8; 16] = [0, -1, -1, 0, -1, -1, 12, -1, 0, -1, 0, 12, -1, -1, 0, -1];

fn mtof(m: f32) -> f32 {
    440.0 * 2f32.powf((m - 69.0) / 12.0)
}

#[derive(Clone, Copy)]
enum Kind {
    Kick,
    Snare(f32),
    Hat { open: bool, amp: f32 },
    Bass(f32),
    Arp { freq: f32, pan: f32, accent: bool },
    Pad([f32; 3]),
}

struct Voice {
    kind: Kind,
    age: f32,
    len: f32,
    phase: [f32; 6],
    lp: [f32; 2],
    prev_noise: f32,
}

pub struct TestSignal {
    sr: f32,
    t: f64,
    step: usize,
    next_step: f64,
    voices: Vec<Voice>,
    seed: u32,
}

impl TestSignal {
    pub fn new(sr: f32) -> Self {
        TestSignal { sr, t: 0.0, step: 0, next_step: 0.0, voices: Vec::new(), seed: 0x9E37_79B9 }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sr
    }

    fn noise(&mut self) -> f32 {
        // xorshift32
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    fn spawn(&mut self, kind: Kind, len: f32) {
        self.voices.push(Voice { kind, age: 0.0, len, phase: [0.0; 6], lp: [0.0; 2], prev_noise: 0.0 });
    }

    fn trigger(&mut self, step: usize) {
        let step_len = 60.0 / BPM / 4.0;
        let bar = step / 16;
        let s = step % 16;
        let chord = &PROG[bar % 4];
        let groove = bar >= 2;
        let full = bar >= 4;
        if s == 0 {
            self.spawn(Kind::Pad(chord.pad), step_len * 16.0 + 0.6);
        }
        let arp_notes = [
            chord.pad[0] + 12.0, chord.pad[1] + 12.0, chord.pad[2] + 12.0,
            chord.pad[0] + 24.0, chord.pad[1] + 24.0, chord.pad[2] + 24.0,
        ];
        let pan = if s % 2 == 1 { 0.55 } else { -0.55 };
        self.spawn(Kind::Arp { freq: mtof(arp_notes[ARP[s]]), pan, accent: s.is_multiple_of(4) }, 0.22);
        if groove && BASS[s] >= 0 {
            self.spawn(Kind::Bass(mtof(chord.root + BASS[s] as f32)), step_len * 1.6 + 0.06);
        }
        if full {
            if s == 0 || s == 8 || (s == 10 && bar % 2 == 1) {
                self.spawn(Kind::Kick, 0.45);
            }
            if s == 4 || s == 12 {
                self.spawn(Kind::Snare(1.0), 0.22);
            }
            if bar == BARS - 1 && s >= 13 {
                self.spawn(Kind::Snare(0.55), 0.22);
            }
        }
        if groove {
            if s.is_multiple_of(2) {
                let open = s == 14 && bar % 2 == 1;
                let amp = if s % 4 == 2 { 0.2 } else { 0.12 };
                self.spawn(Kind::Hat { open, amp }, if open { 0.32 } else { 0.05 });
            } else if bar >= 8 {
                self.spawn(Kind::Hat { open: false, amp: 0.06 }, 0.05);
            }
        }
    }

    /// Appends `n` stereo frames to the callback.
    pub fn render(&mut self, n: usize, mut out: impl FnMut(f32, f32)) {
        let dt = 1.0 / self.sr;
        let step_len = 60.0 / BPM as f64 / 4.0;
        for _ in 0..n {
            while self.t >= self.next_step {
                let step = self.step;
                self.trigger(step);
                self.step = (self.step + 1) % (BARS * 16);
                self.next_step += step_len;
            }
            let (mut l, mut r) = (0.0f32, 0.0f32);
            let mut i = 0;
            while i < self.voices.len() {
                let noise = self.noise();
                let v = &mut self.voices[i];
                let a = v.age;
                match v.kind {
                    Kind::Kick => {
                        let f = 44.0 + 106.0 * (-a / 0.035).exp();
                        v.phase[0] = (v.phase[0] + f * dt) % 1.0;
                        let s = (v.phase[0] * TAU).sin() * 0.8 * (-a / 0.09).exp();
                        l += s;
                        r += s;
                    }
                    Kind::Snare(amt) => {
                        let hp = (noise - v.prev_noise) * 0.5;
                        v.prev_noise = noise;
                        let f = 150.0 + 60.0 * (-a / 0.03).exp();
                        v.phase[0] = (v.phase[0] + f * dt) % 1.0;
                        let tri = 1.0 - 4.0 * (v.phase[0] - 0.5).abs();
                        let s = (hp * 0.5 * (-a / 0.045).exp() + tri * 0.25 * (-a / 0.03).exp()) * amt;
                        l += s;
                        r += s;
                    }
                    Kind::Hat { open, amp } => {
                        let hp = noise - v.prev_noise;
                        v.prev_noise = noise;
                        let s = hp * 0.5 * amp * (-a / if open { 0.08 } else { 0.012 }).exp();
                        l += s * 0.7;
                        r += s * 1.3;
                    }
                    Kind::Bass(f) => {
                        v.phase[0] = (v.phase[0] + f * dt) % 1.0;
                        v.phase[1] = (v.phase[1] + f * 0.5 * dt) % 1.0;
                        let saw = v.phase[0] * 2.0 - 1.0;
                        let sub = if v.phase[1] < 0.5 { 0.35 } else { -0.35 };
                        let cutoff = 160.0 + 1340.0 * (-a / 0.05).exp();
                        let k = 1.0 - (-TAU * cutoff * dt).exp();
                        v.lp[0] += (saw + sub - v.lp[0]) * k;
                        let env = if a < 0.006 { a / 0.006 } else if a > v.len - 0.06 { ((v.len - a) / 0.06).max(0.0) } else { 1.0 };
                        let s = v.lp[0] * 0.3 * env;
                        l += s;
                        r += s;
                    }
                    Kind::Arp { freq, pan, accent } => {
                        v.phase[0] = (v.phase[0] + freq * dt) % 1.0;
                        let sq = if v.phase[0] < 0.5 { 1.0 } else { -1.0 };
                        let cutoff = 700.0 + (if accent { 3500.0 } else { 1900.0 }) * (-a / 0.06).exp();
                        let k = 1.0 - (-TAU * cutoff * dt).exp();
                        v.lp[0] += (sq - v.lp[0]) * k;
                        let s = v.lp[0] * (if accent { 0.07 } else { 0.05 }) * (-a / 0.05).exp();
                        l += s * (1.0 - pan);
                        r += s * (1.0 + pan);
                    }
                    Kind::Pad(notes) => {
                        let attack = (a / 0.5).min(1.0);
                        let release = ((v.len - a) / 0.6).clamp(0.0, 1.0);
                        let env = attack * release * 0.06;
                        let (mut pl, mut pr) = (0.0, 0.0);
                        for (n, m) in notes.iter().enumerate() {
                            let f = mtof(*m);
                            for side in 0..2 {
                                let detune = if side == 0 { 0.9948 } else { 1.0052 };
                                let p = &mut v.phase[n * 2 + side];
                                *p = (*p + f * detune * dt) % 1.0;
                                let saw = *p * 2.0 - 1.0;
                                if side == 0 { pl += saw } else { pr += saw }
                            }
                        }
                        let k = 1.0 - (-TAU * 1100.0 * dt).exp();
                        v.lp[0] += (pl - v.lp[0]) * k;
                        v.lp[1] += (pr - v.lp[1]) * k;
                        l += (v.lp[0] * 0.8 + v.lp[1] * 0.2) * env;
                        r += (v.lp[1] * 0.8 + v.lp[0] * 0.2) * env;
                    }
                }
                v.age += dt;
                if v.age >= v.len {
                    self.voices.swap_remove(i);
                } else {
                    i += 1;
                }
            }
            out((l * 0.9).tanh(), (r * 0.9).tanh());
            self.t += dt as f64;
        }
    }
}
