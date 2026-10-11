//! Audio input. The default source is loopback: a copy of whatever the computer is playing,
//! taken after the mixer. That is how the visualizer sees Spotify, a browser tab, or any
//! other app, without touching the stream itself.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

/// Ring size in frames. Larger than the FFT window, so a read never overtakes a write.
const RING: usize = 16384;
/// Loopback capture stops delivering packets when nothing plays. After this long with
/// no new samples, the ring reads back as silence instead of a frozen picture.
const STALE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Loopback,
    Mic,
    Test,
}

impl Source {
    pub fn next(self) -> Source {
        match self {
            Source::Loopback => Source::Mic,
            Source::Mic => Source::Test,
            Source::Test => Source::Loopback,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Source::Loopback => "LOOPBACK",
            Source::Mic => "MIC",
            Source::Test => "TEST",
        }
    }
}

/// A stereo ring buffer of the most recent samples.
pub struct Ring {
    l: Vec<f32>,
    r: Vec<f32>,
    pos: usize,
    last_write: Option<Instant>,
    /// Frames received since the start, and their sample peak. Used by `--probe`.
    pub total: u64,
    pub peak: f32,
}

impl Ring {
    pub fn new() -> Self {
        Ring { l: vec![0.0; RING], r: vec![0.0; RING], pos: 0, last_write: None, total: 0, peak: 0.0 }
    }

    #[inline]
    pub fn push(&mut self, l: f32, r: f32) {
        self.l[self.pos] = l;
        self.r[self.pos] = r;
        self.pos = (self.pos + 1) % RING;
        self.total += 1;
        self.peak = self.peak.max(l.abs()).max(r.abs());
    }

    pub fn touch(&mut self) {
        self.last_write = Some(Instant::now());
    }

    /// Copies the newest `out_l.len()` frames, oldest first.
    pub fn latest(&self, out_l: &mut [f32], out_r: &mut [f32]) {
        let fresh = self.last_write.is_some_and(|t| t.elapsed() < STALE);
        if !fresh {
            out_l.fill(0.0);
            out_r.fill(0.0);
            return;
        }
        let n = out_l.len().min(RING);
        let start = (self.pos + RING - n) % RING;
        for i in 0..n {
            let j = (start + i) % RING;
            out_l[i] = self.l[j];
            out_r[i] = self.r[j];
        }
    }
}

pub struct Capture {
    _stream: cpal::Stream,
    pub ring: Arc<Mutex<Ring>>,
    pub failed: Arc<AtomicBool>,
    pub device_id: Option<cpal::DeviceId>,
    pub device_name: String,
    /// The device's own rate, for display.
    pub device_rate: u32,
    /// The rate of the samples in the ring, after high rates are brought down near 48 kHz.
    pub sample_rate: u32,
}

impl Capture {
    pub fn start(source: Source) -> Result<Capture, String> {
        let host = cpal::default_host();
        let device = match source {
            // On WASAPI, opening an input stream on an output device turns on loopback capture.
            Source::Loopback => host.default_output_device().ok_or("No audio output device found.")?,
            Source::Mic => host.default_input_device().ok_or("No microphone or line-in device found.")?,
            Source::Test => return Err("The test signal does not use a device.".into()),
        };
        let supported = match source {
            Source::Loopback => device.default_output_config(),
            _ => device.default_input_config(),
        }
        .map_err(|e| format!("Could not read the device format: {e}"))?;

        let channels = supported.channels() as usize;
        let device_rate = supported.sample_rate();
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        // A 4096-point FFT at 192 kHz gives 47 Hz bins and smears the bass. Averaging
        // blocks of 2 or 4 frames brings 88.2 to 192 kHz down to 44.1 or 48 kHz.
        let factor = ((device_rate as f32 / 48_000.0).round() as usize).max(1);

        let ring = Arc::new(Mutex::new(Ring::new()));
        let failed = Arc::new(AtomicBool::new(false));
        let tap = Tap { channels: channels.max(1), factor, ring: ring.clone(), failed: failed.clone() };
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, config, tap),
            SampleFormat::I16 => build::<i16>(&device, config, tap),
            SampleFormat::I32 => build::<i32>(&device, config, tap),
            SampleFormat::U16 => build::<u16>(&device, config, tap),
            SampleFormat::U8 => build::<u8>(&device, config, tap),
            SampleFormat::F64 => build::<f64>(&device, config, tap),
            other => return Err(format!("Unsupported sample format: {other}")),
        }
        .map_err(|e| format!("Could not open the audio stream: {e}"))?;
        stream.play().map_err(|e| format!("Could not start the audio stream: {e}"))?;

        Ok(Capture {
            _stream: stream,
            ring,
            failed,
            device_id: device.id().ok(),
            device_name: device.to_string(),
            device_rate,
            sample_rate: device_rate / factor as u32,
        })
    }

    /// True when the stream died or the system default device moved to another endpoint,
    /// for example when headphones are plugged in.
    pub fn needs_restart(&self, source: Source) -> bool {
        if self.failed.load(Ordering::Relaxed) {
            return true;
        }
        let host = cpal::default_host();
        let current = match source {
            Source::Loopback => host.default_output_device(),
            _ => host.default_input_device(),
        };
        match (current.and_then(|d| d.id().ok()), &self.device_id) {
            (Some(now), Some(ours)) => &now != ours,
            _ => false,
        }
    }
}

/// What the audio callback needs: the channel layout, the averaging factor, and where to write.
struct Tap {
    channels: usize,
    factor: usize,
    ring: Arc<Mutex<Ring>>,
    failed: Arc<AtomicBool>,
}

fn build<T>(device: &cpal::Device, config: cpal::StreamConfig, tap: Tap) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + Sample + Send + 'static,
    f32: FromSample<T>,
{
    let Tap { channels, factor, ring, failed } = tap;
    let scale = 1.0 / factor as f32;
    let (mut sum_l, mut sum_r, mut count) = (0.0f32, 0.0f32, 0usize);
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let Ok(mut ring) = ring.lock() else { return };
            for frame in data.chunks_exact(channels) {
                let l = f32::from_sample(frame[0]);
                let r = if channels > 1 { f32::from_sample(frame[1]) } else { l };
                sum_l += l;
                sum_r += r;
                count += 1;
                if count == factor {
                    ring.push(sum_l * scale, sum_r * scale);
                    (sum_l, sum_r, count) = (0.0, 0.0, 0);
                }
            }
            ring.touch();
        },
        move |err: cpal::Error| {
            eprintln!("audio stream error: {err}");
            if !matches!(err.kind(), cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied) {
                failed.store(true, Ordering::Relaxed);
            }
        },
        None,
    )
}
