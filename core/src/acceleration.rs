//! Optional acceleration of pixel-heavy operations. No graphics API is required by the
//! core or the WebAssembly build. The desktop installs a backend on its rendering thread.
//! CPU buffers remain authoritative, so a failed operation can be retried in software
//! without losing trails, audio state, or the current transition.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use crate::{
    glow::{Flow, Grid},
    palette::Palette,
    post::Look,
    preset::Mirror,
};

pub type Light = [f32; 2];

pub enum Warp<'a> {
    Flow {
        from: (f32, f32),
        to: (f32, f32),
        zoom: f32,
        rot: f32,
        keep: f32,
        flow: Flow,
    },
    Grid(&'a Grid),
}

pub enum Operation<'a> {
    Warp {
        src: &'a [Light],
        dst: &'a mut [Light],
        w: usize,
        h: usize,
        warp: Warp<'a>,
        spread: bool,
    },
    Resolve {
        src: &'a [Light],
        dst: &'a mut [u32],
        w: usize,
        h: usize,
        palette: &'a Palette,
        brightness: f32,
    },
    Post {
        pixels: &'a mut [u32],
        w: usize,
        h: usize,
        look: &'a Look,
        mirror: Mirror,
    },
    Orbs {
        dst: &'a mut [Light],
        w: usize,
        h: usize,
        xs: &'a [(f32, f32)],
        ys: &'a [(f32, f32)],
        lobes: &'a [(f32, f32, f32, f32)],
        hand: [f32; 3],
        hue: f32,
        energy: f32,
        detail: f32,
    },
    Mandala {
        src: &'a [Light],
        side: usize,
        dst: &'a mut [u32],
        palette: &'a Palette,
        w: usize,
        h: usize,
        mid: f32,
        window: f32,
        scale: f32,
        turn: f32,
        folds: usize,
    },
    /// Lens layout: mirror kind, cosine, sine, inverse scale, pan x/y, two reserved,
    /// followed by four window corners (x/y) in source pixels.
    Kaleidoscope {
        src: &'a [Light],
        side: usize,
        dst: &'a mut [u32],
        palette: &'a Palette,
        w: usize,
        h: usize,
        center: (f32, f32),
        shift_x: &'a [f32],
        shift_y: &'a [f32],
        current: [f32; 16],
        previous: Option<([f32; 16], f32)>,
    },
    /// Ring layout: phase cosine/sine, amplitude, source center x/y.
    Gravity {
        dst: &'a mut [Light],
        w: usize,
        h: usize,
        rings: &'a [[f32; 5]],
        lenses: &'a [(f32, f32, f32)],
        now: (f32, f32),
        stretch: f32,
        moved: bool,
        inner: f32,
        fade: f32,
        far: f32,
        haze: f32,
        haze_u: f32,
        haze_r: f32,
    },
}

pub trait Accelerator {
    fn name(&self) -> &str;
    /// Ok(true) completed the operation. Ok(false) selects the CPU for this workload.
    /// On false or error, the destination MUST be untouched so software can retry it.
    fn run(&mut self, operation: Operation<'_>) -> Result<bool, String>;
    fn software_time(&mut self, _elapsed: Duration) {}
    fn projection_software_time(&mut self, _elapsed: Duration) {}
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub adapter: Option<String>,
    pub fallback: Option<String>,
    pub operations: u64,
}

#[derive(Default)]
struct State {
    backend: Option<Box<dyn Accelerator>>,
    status: Status,
    cpu_started: Option<Instant>,
}

thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }

pub fn install(backend: Box<dyn Accelerator>) {
    let adapter = Some(backend.name().to_owned());
    STATE.with(|state| {
        *state.borrow_mut() = State {
            backend: Some(backend),
            status: Status {
                adapter,
                ..Status::default()
            },
            cpu_started: None,
        }
    });
}

pub fn software(reason: Option<String>) {
    STATE.with(|state| {
        *state.borrow_mut() = State {
            backend: None,
            status: Status {
                fallback: reason,
                ..Status::default()
            },
            cpu_started: None,
        }
    });
}

pub fn status() -> Status {
    STATE.with(|state| state.borrow().status.clone())
}

pub fn run(operation: Operation<'_>) -> bool {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(backend) = &mut state.backend else {
            return false;
        };
        match backend.run(operation) {
            Ok(true) => {
                state.status.operations += 1;
                true
            }
            Ok(false) => {
                state.cpu_started = Some(Instant::now());
                false
            }
            Err(reason) => {
                state.backend = None;
                state.status.adapter = None;
                state.status.fallback = Some(reason);
                false
            }
        }
    })
}

/// Finish a software alternative so Automatic compares transfer-inclusive GPU time
/// with the same CPU workload. No timing is collected without a backend.
pub fn finish_software() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if let Some(start) = state.cpu_started.take() {
            if let Some(backend) = &mut state.backend {
                backend.software_time(start.elapsed());
            }
        }
    });
}

/// Avoid reading the native clock in software-only builds, including browser WASM.
pub fn start_projection() -> Option<Instant> {
    STATE.with(|state| state.borrow().backend.as_ref().map(|_| Instant::now()))
}

pub fn finish_projection(started: Option<Instant>) {
    let Some(started) = started else {
        return;
    };
    STATE.with(|state| {
        if let Some(backend) = &mut state.borrow_mut().backend {
            backend.projection_software_time(started.elapsed());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{canvas::Canvas, glow::Field};

    struct LostDevice;
    impl Accelerator for LostDevice {
        fn name(&self) -> &str {
            "Lost test device"
        }
        fn run(&mut self, _: Operation<'_>) -> Result<bool, String> {
            Err("Device removed".into())
        }
    }

    #[test]
    fn failed_gpu_operation_retries_in_software_without_losing_the_image() {
        software(None);
        let mut field = Field::default();
        field.fit(40, 30);
        field.dab(20.0, 15.0, 10.0, 0.4, 1.0);
        let pal = Palette::new(1);
        let mut expected = Canvas::new(40, 30);
        field.resolve(&mut expected, &pal);
        install(Box::new(LostDevice));
        let mut actual = Canvas::new(40, 30);
        field.resolve(&mut actual, &pal);
        assert_eq!(actual.px, expected.px);
        assert_eq!(status().fallback.as_deref(), Some("Device removed"));
        assert!(status().adapter.is_none());
        field.resolve(&mut actual, &pal);
        assert_eq!(actual.px, expected.px);
        software(None);
    }
}
