//! Offline rendering: reads a WAV file and draws one picture per video frame at a fixed
//! frame rate, faster or slower than real time. The same file, options and recipe always
//! give the same pictures.

use crate::analysis::{Analyzer, FFT};
use crate::canvas::Canvas;
use crate::palette::Palette;
use crate::preset::Preset;
use crate::show::Show;

/// Stereo samples. Mono files have the same samples in both channels.
pub struct Audio {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub rate: u32,
}

impl Audio {
    pub fn seconds(&self) -> f64 {
        self.left.len() as f64 / self.rate.max(1) as f64
    }
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Reads a WAV file: 8, 16, 24 or 32-bit whole numbers, or 32 or 64-bit floating point,
/// with any number of channels. Only the first two channels are used. Rates above 50 kHz
/// are averaged down near 48 kHz, as the live capture does, so the bass keeps its detail.
pub fn read_wav(bytes: &[u8]) -> Result<Audio, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("This is not a WAV file (no RIFF/WAVE header).".into());
    }
    let mut format = None;
    let mut data = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let (id, size) = (&bytes[at..at + 4], u32_at(bytes, at + 4) as usize);
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        match id {
            b"fmt " if body.len() >= 16 => {
                let mut tag = u16_at(body, 0);
                // WAVE_FORMAT_EXTENSIBLE keeps the real format in its sub-format.
                if tag == 0xFFFE && body.len() >= 26 {
                    tag = u16_at(body, 24);
                }
                format = Some((tag, u16_at(body, 2) as usize, u32_at(body, 4), u16_at(body, 14) as usize));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    let (tag, channels, rate, bits) = format.ok_or("The WAV file has no format chunk.")?;
    let data = data.ok_or("The WAV file has no data chunk.")?;
    if channels == 0 || rate == 0 {
        return Err("The WAV file has no channels or no sample rate.".into());
    }
    let read: fn(&[u8]) -> f32 = match (tag, bits) {
        (1, 8) => |b| (b[0] as f32 - 128.0) / 128.0,
        (1, 16) => |b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32_768.0,
        (1, 24) => |b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
        (1, 32) => |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
        (3, 32) => |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        (3, 64) => |b| f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32,
        _ => return Err(format!("WAV format {tag} with {bits} bits is not supported. Use PCM or floating point.")),
    };
    let size = bits / 8;
    let frame = size * channels;
    let factor = ((rate as f32 / 48_000.0).round() as usize).max(1);
    let n = data.len() / frame / factor;
    let (mut left, mut right) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for block in data.chunks_exact(frame * factor) {
        let (mut l, mut r) = (0.0, 0.0);
        for f in block.chunks_exact(frame) {
            let a = read(&f[..size]);
            l += a;
            r += if channels > 1 { read(&f[size..2 * size]) } else { a };
        }
        let clean = |v: f32| if v.is_finite() { (v / factor as f32).clamp(-1.0, 1.0) } else { 0.0 };
        left.push(clean(l));
        right.push(clean(r));
    }
    Ok(Audio { left, right, rate: rate / factor as u32 })
}

/// What to render.
pub struct Job {
    pub fps: f32,
    pub width: usize,
    pub height: usize,
    pub mode: usize,
    pub preset: Preset,
    pub skin: usize,
    pub auto: bool,
}

impl Job {
    pub fn frames(&self, audio: &Audio) -> usize {
        (audio.seconds() * self.fps as f64).ceil() as usize
    }
}

/// Draws every frame of `audio` and hands each picture to `out`, in order. Returns the
/// number of frames.
pub fn render(audio: &Audio, job: &Job, mut out: impl FnMut(usize, &Canvas) -> Result<(), String>) -> Result<usize, String> {
    if !(job.fps >= 1.0 && job.fps <= 240.0) {
        return Err("The frame rate must be from 1 to 240.".into());
    }
    // The recipe's seed also sets the transition patterns and auto mode's timing.
    let mut show = Show::new(job.mode, job.preset.clone(), job.auto, job.preset.seed);
    show.resize(job.width, job.height);
    let pal = Palette::new(job.skin);
    let mut analyzer = Analyzer::new();
    let (mut l, mut r) = (vec![0.0; FFT], vec![0.0; FFT]);
    let dt = 1.0 / job.fps;
    let frames = job.frames(audio);
    for k in 0..frames {
        // The window ends at the frame's own time.
        let end = (k as f64 * audio.rate as f64 / job.fps as f64).round() as usize;
        for i in 0..FFT {
            let at = (end + i).checked_sub(FFT).filter(|&j| j < audio.left.len());
            l[i] = at.map_or(0.0, |j| audio.left[j]);
            r[i] = at.map_or(0.0, |j| audio.right[j]);
        }
        analyzer.process(&l, &r, audio.rate as f32, dt);
        show.frame(&analyzer.frame, &pal, dt);
        out(k, &show.composite)?;
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WAV file holding `samples` (interleaved), in the given format.
    fn wav(tag: u16, channels: u16, rate: u32, bits: u16, samples: &[f64]) -> Vec<u8> {
        let mut data = Vec::new();
        for &s in samples {
            match (tag, bits) {
                (1, 16) => data.extend_from_slice(&((s * 32_767.0) as i16).to_le_bytes()),
                (1, 24) => data.extend_from_slice(&((s * 8_388_607.0) as i32).to_le_bytes()[..3]),
                (3, 32) => data.extend_from_slice(&(s as f32).to_le_bytes()),
                (3, 64) => data.extend_from_slice(&s.to_le_bytes()),
                _ => unreachable!(),
            }
        }
        let extensible = tag == 3 && bits == 64;
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&(if extensible { 0xFFFEu16 } else { tag }).to_le_bytes());
        fmt.extend_from_slice(&channels.to_le_bytes());
        fmt.extend_from_slice(&rate.to_le_bytes());
        fmt.extend_from_slice(&(rate * channels as u32 * bits as u32 / 8).to_le_bytes());
        fmt.extend_from_slice(&(channels * bits / 8).to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());
        if extensible {
            fmt.extend_from_slice(&22u16.to_le_bytes());
            fmt.extend_from_slice(&bits.to_le_bytes());
            fmt.extend_from_slice(&3u32.to_le_bytes());
            fmt.extend_from_slice(&tag.to_le_bytes());
            fmt.extend_from_slice(&[0; 14]);
        }
        let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
        for (id, body) in [(b"fmt ", &fmt), (b"LIST", &vec![1u8; 3]), (b"data", &data)] {
            out.extend_from_slice(id);
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(body);
            if body.len() % 2 == 1 {
                out.push(0);
            }
        }
        out
    }

    #[test]
    fn reads_common_wav_formats() {
        let stereo = [0.5, -0.25, 0.0, 1.0, -1.0, 0.125];
        for (tag, bits) in [(1, 16), (1, 24), (3, 32), (3, 64)] {
            let a = read_wav(&wav(tag, 2, 44_100, bits, &stereo)).unwrap();
            assert_eq!(a.rate, 44_100);
            for (got, want) in a.left.iter().zip([0.5, 0.0, -1.0]).chain(a.right.iter().zip([-0.25, 1.0, 0.125])) {
                assert!((got - want).abs() < 1e-3, "format {tag}/{bits}: {got} for {want}");
            }
        }
        let mono = read_wav(&wav(1, 1, 22_050, 16, &[0.5, -0.5])).unwrap();
        assert_eq!(mono.left, mono.right, "Mono plays in both channels");
        assert!((mono.left[1] + 0.5).abs() < 1e-3);
        let high = read_wav(&wav(3, 2, 96_000, 32, &[0.2, 0.4, 0.4, 0.8, 0.6, 0.0, 0.2, 0.0])).unwrap();
        assert_eq!((high.rate, high.left.len()), (48_000, 2), "96 kHz averages pairs down to 48 kHz");
        assert!((high.left[0] - 0.3).abs() < 1e-6 && (high.left[1] - 0.4).abs() < 1e-6);
        assert!(read_wav(b"RIFF\0\0\0\0AVI ").is_err());
        assert!(read_wav(&wav(1, 2, 44_100, 16, &[])[..30]).is_err());
    }

    #[test]
    fn renders_every_frame_the_same_way_twice() {
        let rate = 48_000;
        let samples: Vec<f64> = (0..rate / 2).flat_map(|i| {
            let t = i as f64 / rate as f64;
            let kick = (t * 55.0 * std::f64::consts::TAU).sin() * (-(t % 0.25) / 0.06).exp();
            [kick * 0.6, kick * 0.5]
        }).collect();
        let audio = read_wav(&wav(1, 2, rate, 16, &samples)).unwrap();
        let job = Job { fps: 30.0, width: 64, height: 40, mode: 5, preset: Preset::builtin(4), skin: 1, auto: false };
        let run = || {
            let mut frames = Vec::new();
            let n = render(&audio, &job, |k, c| {
                assert_eq!((c.w, c.h, k), (64, 40, frames.len()));
                frames.push(c.px.clone());
                Ok(())
            }).unwrap();
            (n, frames)
        };
        let (n, first) = run();
        assert_eq!(n, 15);
        assert_eq!(first.len(), 15);
        assert_eq!(run().1, first, "The same input gives the same frames");
        assert_ne!(first[2], first[14], "The picture moves");
        assert!(render(&audio, &Job { fps: 0.0, ..job }, |_, _| Ok(())).is_err());
    }
}
