//! Turns the newest samples into what the modes draw: a smoothed log-magnitude spectrum
//! and the raw left, right, and mid waveforms.

use std::sync::Arc;

use rustfft::{num_complex::Complex, Fft, FftPlanner};

pub const FFT: usize = 4096;
pub const BINS: usize = FFT / 2;
/// Waveform length handed to the time-domain modes.
pub const TD: usize = 2048;

/// The dB range mapped onto 0..1. It matches the AnalyserNode settings in the web version.
const MIN_DB: f32 = -95.0;
const MAX_DB: f32 = -20.0;
/// Magnitude smoothing between frames, as in an AnalyserNode.
const SMOOTHING: f32 = 0.7;
/// Band power at -60 dB on the band scale, where the band levels read 0. The averages
/// skip quieter frames, so silence gives relative levels near 0, not noise.
const FLOOR: f32 = 1e-6 * 3.0 / 16.0;
/// How long the song average remembers, in seconds.
const AVERAGE_SECONDS: f32 = 5.0;
/// After this much silence in a band, its average starts again with the next sound.
const FORGET_SECONDS: f32 = 2.0;
/// A beat needs bass power this many times the song average...
const BEAT_LEVEL: f32 = 1.4;
/// ...and this many times the average of the last eighth of a second.
const BEAT_RISE: f32 = 1.3;
/// The shortest time between two beats, in seconds.
const BEAT_GAP: f32 = 0.2;

#[derive(Clone)]
pub struct Frame {
    /// Per-bin level, 0 (at or below -95 dB) to 1 (-20 dB or louder).
    pub spec: Vec<f32>,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub mid: Vec<f32>,
    pub sample_rate: f32,
    /// Kick-drum energy, 0..1.
    pub bass: f32,
    /// Overall energy, 0..1.
    pub level: f32,
    /// Sample peak of the newest window, linear.
    pub peak: f32,
    pub music: Music,
}

/// Independent musical controls. Fast envelopes catch attacks; slow envelopes guide motion.
#[derive(Clone, Copy, Default)]
pub struct Music {
    pub bands: [f32; 3],
    pub slow: [f32; 3],
    /// Each band's power against its own average over the last few seconds: about 1 for
    /// typical playing, up to 4 on loud hits, 0 in silence. A quiet recording and a loud one
    /// give the same values.
    pub rel: [f32; 3],
    pub onset: f32,
    /// The strength (0.1 to 1) of a bass beat that starts on this frame. 0 on other frames.
    pub beat: f32,
    /// Jumps to each beat's strength, then fades out over about a quarter of a second.
    pub pulse: f32,
    pub width: f32,
}

fn follow(value: &mut f32, target: f32, attack: f32, release: f32, dt: f32) {
    let rate = if target > *value { attack } else { release };
    *value += (target - *value) * (1.0 - (-rate * dt).exp());
}

impl Frame {
    pub fn hz_per_bin(&self) -> f32 {
        self.sample_rate / FFT as f32
    }

    pub fn band_avg(&self, f0: f32, f1: f32) -> f32 {
        let hz = self.hz_per_bin();
        let s = ((f0 / hz).floor() as usize).min(BINS - 1);
        let e = ((f1 / hz).ceil() as usize).min(BINS - 1).max(s);
        self.spec[s..=e].iter().sum::<f32>() / (e - s + 1) as f32
    }
}

pub struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    mag: Vec<f32>,
    /// Each band's average power over the last few seconds, as a weighted sum and the sum
    /// of its weights. The weights start at zero, so the first second of a song is averaged
    /// over just that second.
    sum: [f32; 3],
    weight: [f32; 3],
    quiet: [f32; 3],
    /// Bass power over the last eighth of a second.
    recent: f32,
    since_beat: f32,
    pub frame: Frame,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Self {
        let fft = FftPlanner::new().plan_fft_forward(FFT);
        let scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
        let window = (0..FFT)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT as f32).cos())
            .collect();
        Analyzer {
            fft,
            window,
            buf: vec![Complex::new(0.0, 0.0); FFT],
            scratch,
            mag: vec![0.0; BINS],
            sum: [0.0; 3],
            weight: [0.0; 3],
            quiet: [0.0; 3],
            recent: 0.0,
            since_beat: BEAT_GAP,
            frame: Frame {
                spec: vec![0.0; BINS],
                left: vec![0.0; TD],
                right: vec![0.0; TD],
                mid: vec![0.0; TD],
                sample_rate: 48_000.0,
                bass: 0.0,
                level: 0.0,
                peak: 0.0,
                music: Music::default(),
            },
        }
    }

    /// `l` and `r` hold the newest `FFT` frames, oldest first.
    pub fn process(&mut self, l: &[f32], r: &[f32], sample_rate: f32, dt: f32) {
        let dt = dt.clamp(0.0, 0.1);
        let f = &mut self.frame;
        f.sample_rate = sample_rate;
        for (i, c) in self.buf.iter_mut().enumerate() {
            *c = Complex::new((l[i] + r[i]) * 0.5 * self.window[i], 0.0);
        }
        self.fft.process_with_scratch(&mut self.buf, &mut self.scratch);
        let norm = 1.0 / FFT as f32;
        let keep = SMOOTHING.powf(dt * 60.0);
        for k in 0..BINS {
            let m = self.buf[k].norm() * norm;
            self.mag[k] = keep * self.mag[k] + (1.0 - keep) * m;
            let db = 20.0 * self.mag[k].max(1e-12).log10();
            f.spec[k] = ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0);
        }
        let off = FFT - TD;
        let mut peak = 0.0f32;
        for i in 0..TD {
            let (a, b) = (l[off + i], r[off + i]);
            f.left[i] = a;
            f.right[i] = b;
            f.mid[i] = (a + b) * 0.5;
            peak = peak.max(a.abs()).max(b.abs());
        }
        f.peak = peak;
        f.bass = ((f.band_avg(30.0, 150.0) - 0.4) / 0.6).clamp(0.0, 1.0);
        f.level = ((f.band_avg(30.0, 8000.0) - 0.25) / 0.75).clamp(0.0, 1.0);
        let keep_average = (-dt / AVERAGE_SECONDS).exp();
        let mut bass = (0.0, 0.0);
        for (i, (lo, hi)) in [(30.0, 180.0), (180.0, 2500.0), (2500.0, 16000.0)].into_iter().enumerate() {
            let hz = sample_rate / FFT as f32;
            let start = ((lo / hz).ceil() as usize).min(BINS - 1);
            let end = ((hi / hz).ceil() as usize).min(BINS).max(start + 1);
            // Integrated linear power avoids diluting a narrow musical note across a wide band.
            let power: f32 = self.buf[start..end].iter().map(|v| v.norm_sqr() * norm * norm).sum();
            let db = 10.0 * (power * (16.0 / 3.0)).max(1e-12).log10();
            let target = ((db + 60.0) / 48.0).clamp(0.0, 1.0);
            follow(&mut f.music.bands[i], target, 28.0, 5.0, dt);
            follow(&mut f.music.slow[i], target, 1.4, 0.8, dt);
            if power > FLOOR {
                self.quiet[i] = 0.0;
                self.sum[i] = self.sum[i] * keep_average + power * (1.0 - keep_average);
                self.weight[i] = self.weight[i] * keep_average + (1.0 - keep_average);
            } else {
                self.quiet[i] += dt;
                if self.quiet[i] > FORGET_SECONDS {
                    self.sum[i] = 0.0;
                    self.weight[i] = 0.0;
                }
            }
            let average = if self.weight[i] > 0.0 { self.sum[i] / self.weight[i] } else { power };
            let ratio = (power / average.max(FLOOR)).min(4.0);
            follow(&mut f.music.rel[i], ratio, 30.0, 6.0, dt);
            if i == 0 { bass = (power, ratio); }
        }
        let hit = ((f.music.bands[0] - f.music.slow[0] - 0.06) * 2.5).clamp(0.0, 1.0);
        follow(&mut f.music.onset, hit, 35.0, 8.0, dt);
        // A beat is bass that is loud for this song and rising fast.
        let (power, ratio) = bass;
        let rising = power > self.recent * BEAT_RISE;
        self.recent = power + (self.recent - power) * (-dt / 0.125).exp();
        self.since_beat += dt;
        f.music.beat = 0.0;
        if rising && ratio > BEAT_LEVEL && self.since_beat >= BEAT_GAP {
            f.music.beat = ((ratio - 1.0) / 2.0).clamp(0.1, 1.0);
            self.since_beat = 0.0;
        }
        f.music.pulse = (f.music.pulse * (-dt / 0.12).exp()).max(f.music.beat);
        let mid_power: f32 = l.iter().zip(r).map(|(l, r)| (l + r).powi(2)).sum();
        let side_power: f32 = l.iter().zip(r).map(|(l, r)| (l - r).powi(2)).sum();
        let width = side_power / (mid_power + side_power).max(1e-9);
        follow(&mut f.music.width, width, 5.0, 3.0, dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn musical_bands_isolate_tones_and_release_in_silence() {
        for (band, hz) in [80.0, 900.0, 6500.0].into_iter().enumerate() {
            let mut a = Analyzer::new();
            let samples: Vec<_> = (0..FFT).map(|i| (i as f32 * std::f32::consts::TAU * hz / 48000.0).sin() * 0.2).collect();
            for _ in 0..60 { a.process(&samples, &samples, 48000.0, 1.0 / 60.0); }
            assert!(a.frame.music.bands[band] > 0.8);
            for other in 0..3 {
                if other != band { assert!(a.frame.music.bands[other] < 0.15); }
            }
            assert!(a.frame.music.width < 0.01);
            let zero = vec![0.0; FFT];
            for _ in 0..300 { a.process(&zero, &zero, 48000.0, 1.0 / 60.0); }
            assert!(a.frame.music.bands.iter().all(|v| *v < 0.001));
            assert!(a.frame.music.slow.iter().all(|v| *v < 0.03));
            assert!(a.frame.music.rel.iter().all(|v| *v < 0.01));
        }
    }

    const RATE: f32 = 48_000.0;
    const KICK_EVERY: f32 = 0.5;

    /// Kicks every half second over a quiet pad, scaled by `gain`.
    fn kicks(seconds: f32, gain: f32) -> Vec<f32> {
        (0..(seconds * RATE) as usize).map(|i| {
            let t = i as f32 / RATE;
            let age = t % KICK_EVERY;
            let kick = (age * 52.0 * std::f32::consts::TAU).sin() * (-age / 0.1).exp() * 0.8;
            let pad = (t * 900.0 * std::f32::consts::TAU).sin() * 0.05;
            (kick + pad) * gain
        }).collect()
    }

    /// Plays `signal` at 60 frames a second. Returns each frame's time and music controls.
    fn play(signal: &[f32]) -> Vec<(f32, Music)> {
        let mut a = Analyzer::new();
        let mut window = vec![0.0; FFT];
        (1..signal.len() / 800).map(|k| {
            let end = k * 800;
            for (i, w) in window.iter_mut().enumerate() {
                *w = (end + i).checked_sub(FFT).map_or(0.0, |j| signal[j]);
            }
            a.process(&window, &window, RATE, 1.0 / 60.0);
            (end as f32 / RATE, a.frame.music)
        }).collect()
    }

    #[test]
    fn beats_follow_kicks_and_ignore_steady_sound() {
        let frames = play(&kicks(8.0, 0.5));
        let beats: Vec<f32> = frames.iter().filter(|(_, m)| m.beat > 0.0).map(|(t, _)| *t).collect();
        assert!((12..=16).contains(&beats.len()), "{} beats for 16 kicks: {beats:?}", beats.len());
        for t in &beats {
            let late = t % KICK_EVERY;
            assert!(late < 0.12, "A beat at {t:.3} s is {late:.3} s after its kick");
        }
        assert!(frames.iter().all(|(_, m)| (0.0..=1.0).contains(&m.beat) && m.pulse >= m.beat));
        let steady: Vec<f32> = (0..(4.0 * RATE) as usize).map(|i| (i as f32 / RATE * 80.0 * std::f32::consts::TAU).sin() * 0.4).collect();
        let frames = play(&steady);
        assert!(frames.iter().filter(|(t, _)| *t > 0.5).all(|(_, m)| m.beat == 0.0), "A steady tone has no beats");
        let (_, last) = frames.last().unwrap();
        assert!((last.rel[0] - 1.0).abs() < 0.1, "A steady tone sits at its own average: {}", last.rel[0]);
    }

    #[test]
    fn relative_levels_ignore_overall_loudness() {
        let quiet = play(&kicks(8.0, 0.05));
        let loud = play(&kicks(8.0, 0.5));
        let tail = |f: &[(f32, Music)], pick: fn(&Music) -> f32| -> f32 {
            let late: Vec<f32> = f.iter().filter(|(t, _)| *t > 6.0).map(|(_, m)| pick(m)).collect();
            late.iter().sum::<f32>() / late.len() as f32
        };
        let (q, l) = (tail(&quiet, |m| m.bands[0]), tail(&loud, |m| m.bands[0]));
        assert!(l - q > 0.3, "The absolute bass level follows loudness: {q} against {l}");
        for band in 0..2 {
            let pick: fn(&Music) -> f32 = [|m: &Music| m.rel[0], |m: &Music| m.rel[1]][band];
            let (q, l) = (tail(&quiet, pick), tail(&loud, pick));
            assert!((q - l).abs() < 0.15 * l.max(0.1), "Band {band} relative level: {q} against {l}");
        }
        let count = |f: &[(f32, Music)]| f.iter().filter(|(_, m)| m.beat > 0.0).count();
        assert_eq!(count(&quiet), count(&loud));
    }
}
