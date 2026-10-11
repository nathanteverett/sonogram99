//! The Sonograph 99 renderer. It turns stereo samples into pictures in a software
//! framebuffer, and has no window, audio device, or platform code. The desktop app, the
//! offline renderer, and the WebAssembly build all draw through it.

pub mod analysis;
pub mod canvas;
pub mod feedback;
pub mod fluid;
pub mod font;
pub mod geometry;
pub mod glow;
pub mod gravity;
pub mod interaction;
pub mod kaleidoscope;
pub mod modes;
pub mod offline;
pub mod palette;
pub mod post;
pub mod preset;
pub mod show;
pub mod synthwave;
pub mod testsignal;
pub mod transition;
