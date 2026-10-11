//! Music-reactive desktop Halo with a separate, circular input surface.
//!
//! The visual window is always click-through, including its bright spokes. Only the
//! center window takes input. Both use per-pixel alpha, so there is no black rectangle.

use sonograph_core::{analysis::Frame, canvas::{unpack, Canvas}, palette::{Palette, SCREEN}, preset::Preset, show::Show};

const SIDE: usize = 280;
const CENTER: usize = 56;
const INSET: i32 = ((SIDE - CENTER) / 2) as i32;

struct Avatar {
    show: Show,
    halo: Vec<u32>,
    center: Vec<u32>,
}

impl Avatar {
    fn new() -> Self {
        let mut show = Show::new(3, Preset::default(), false, 0x5EED_0099);
        show.resize(SIDE, SIDE);
        let mut avatar = Self { show, halo: vec![0; SIDE * SIDE], center: vec![0; CENTER * CENTER] };
        alpha_pixels(&avatar.show.viz, &mut avatar.halo, &mut avatar.center, SCREEN);
        avatar
    }

    fn frame(&mut self, audio: &Frame, palette: &Palette, dt: f32, rate: usize, pulse: bool) {
        self.show.set_rate(rate);
        self.show.pulse = pulse;
        self.show.frame(audio, palette, dt);
        alpha_pixels(&self.show.composite, &mut self.halo, &mut self.center, palette.skin.screen);
    }
}

/// Convert the existing opaque renderer to premultiplied BGRA for Windows composition.
/// Removing the skin background makes silence transparent; a feathered rim hides square trail edges.
fn alpha_pixels(canvas: &Canvas, halo: &mut [u32], center: &mut [u32], background: sonograph_core::canvas::Rgb) {
    for (i, pixel) in halo.iter_mut().enumerate() {
        let x = (i % SIDE) as f32 + 0.5 - SIDE as f32 / 2.0;
        let y = (i / SIDE) as f32 + 0.5 - SIDE as f32 / 2.0;
        let radius = x.hypot(y);
        let feather = ((SIDE as f32 / 2.0 - radius) / 22.0).clamp(0.0, 1.0);
        let rgb = unpack(canvas.px[i]);
        let rgb = std::array::from_fn::<_, 3, _>(|c| (rgb[c].saturating_sub(background[c]) as f32 * feather) as u32);
        let alpha = *rgb.iter().max().unwrap();
        *pixel = if radius < CENTER as f32 / 2.0 { 0 } else {
            alpha << 24 | rgb[0] << 16 | rgb[1] << 8 | rgb[2]
        };
    }
    for (i, pixel) in center.iter_mut().enumerate() {
        let x = (i % CENTER) as f32 + 0.5 - CENTER as f32 / 2.0;
        let y = (i / CENTER) as f32 + 0.5 - CENTER as f32 / 2.0;
        let coverage = (CENTER as f32 / 2.0 - x.hypot(y)).clamp(0.0, 1.0);
        let rgb = unpack(canvas.px[(i / CENTER + INSET as usize) * SIDE + i % CENTER + INSET as usize]);
        // A subtle central disc stays visible and draggable even during silence.
        let alpha = (rgb.iter().copied().max().unwrap().max(110) as f32 * coverage) as u32;
        let rgb = rgb.map(|c| (c as u32 * alpha) / 255);
        *pixel = alpha << 24 | rgb[0] << 16 | rgb[1] << 8 | rgb[2];
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::cell::Cell;
    use std::sync::OnceLock;
    use windows::{core::w, Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
        Graphics::Gdi::*,
        UI::{Input::KeyboardAndMouse::{ReleaseCapture, SetCapture}, WindowsAndMessaging::*},
    }};

    struct Input {
        restore: Cell<bool>,
        drag: Cell<Option<POINT>>,
        halo: HWND,
    }

    unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Input;
        if let Some(input) = ptr.as_ref() {
            match message {
                WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
                WM_LBUTTONDBLCLK | WM_RBUTTONUP | WM_CLOSE => {
                    input.restore.set(true);
                    let _ = ReleaseCapture();
                    input.drag.set(None);
                    return LRESULT(0);
                }
                WM_KEYDOWN if wparam.0 == 0x1b || wparam.0 == 0x7b => {
                    input.restore.set(true);
                    return LRESULT(0);
                }
                WM_LBUTTONDOWN => {
                    let mut cursor = POINT::default();
                    let mut rect = RECT::default();
                    if GetCursorPos(&mut cursor).is_ok() && GetWindowRect(hwnd, &mut rect).is_ok() {
                        input.drag.set(Some(POINT { x: cursor.x - rect.left, y: cursor.y - rect.top }));
                        SetCapture(hwnd);
                    }
                    return LRESULT(0);
                }
                WM_MOUSEMOVE => {
                    if let Some(offset) = input.drag.get() {
                        let mut cursor = POINT::default();
                        if GetCursorPos(&mut cursor).is_ok() {
                            let (x, y) = (cursor.x - offset.x, cursor.y - offset.y);
                            let flags = SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
                            let _ = SetWindowPos(hwnd, None, x, y, 0, 0, flags);
                            let _ = SetWindowPos(input.halo, None, x - INSET, y - INSET, 0, 0, flags);
                        }
                    }
                    return LRESULT(0);
                }
                WM_LBUTTONUP => {
                    input.drag.set(None);
                    let _ = ReleaseCapture();
                    return LRESULT(0);
                }
                WM_CAPTURECHANGED => { input.drag.set(None); }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }

    /// One reusable top-down DIB. Re-select before deletion to release every GDI object.
    struct Surface {
        dc: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
        bits: *mut u32,
        side: usize,
    }

    impl Surface {
        fn new(side: usize) -> windows::core::Result<Self> {
            unsafe {
                let dc = CreateCompatibleDC(None);
                if dc.0.is_null() { return Err(windows::core::Error::from_thread()); }
                let info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: side as i32, biHeight: -(side as i32), biPlanes: 1,
                    biBitCount: 32, biCompression: BI_RGB.0, ..Default::default()
                }, ..Default::default() };
                let mut bits = std::ptr::null_mut();
                let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                    Ok(bitmap) => bitmap,
                    Err(error) => { let _ = DeleteDC(dc); return Err(error); }
                };
                let previous = SelectObject(dc, HGDIOBJ(bitmap.0));
                Ok(Self { dc, bitmap, previous, bits: bits.cast(), side })
            }
        }

        fn present(&self, hwnd: HWND, pixels: &[u32], position: POINT) -> windows::core::Result<()> {
            assert_eq!(pixels.len(), self.side * self.side);
            unsafe {
                std::ptr::copy_nonoverlapping(pixels.as_ptr(), self.bits, pixels.len());
                let size = SIZE { cx: self.side as i32, cy: self.side as i32 };
                let origin = POINT::default();
                let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0,
                    SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
                UpdateLayeredWindow(hwnd, None, Some(&position), Some(&size), Some(self.dc),
                    Some(&origin), COLORREF(0), Some(&blend), ULW_ALPHA)
            }
        }
    }

    impl Drop for Surface {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.previous);
                let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
                let _ = DeleteDC(self.dc);
            }
        }
    }

    struct Session {
        halo: HWND,
        center: HWND,
        main: HWND,
        input: Box<Input>,
        visual: Surface,
        handle: Surface,
        avatar: Avatar,
    }

    impl Session {
        fn new(main: HWND, previous: Option<POINT>) -> windows::core::Result<Self> {
            static CLASS: OnceLock<u16> = OnceLock::new();
            unsafe {
                let instance = HINSTANCE(GetWindowLongPtrW(main, GWLP_HINSTANCE) as *mut _);
                let atom = *CLASS.get_or_init(|| RegisterClassW(&WNDCLASSW {
                    style: CS_DBLCLKS, lpfnWndProc: Some(window_proc), hInstance: instance,
                    hCursor: LoadCursorW(None, IDC_HAND).unwrap_or_default(),
                    lpszClassName: w!("Sonograph99Miniaturize"), ..Default::default()
                }));
                if atom == 0 { return Err(windows::core::Error::from_thread()); }
                let mut monitor = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                let monitor_handle = previous.map_or_else(|| MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST),
                    |p| MonitorFromPoint(POINT { x: p.x + SIDE as i32 / 2, y: p.y + SIDE as i32 / 2 }, MONITOR_DEFAULTTONEAREST));
                GetMonitorInfoW(monitor_handle, &mut monitor).ok()?;
                let work = monitor.rcWork;
                let position = previous.unwrap_or(POINT { x: work.right - SIDE as i32 - 24, y: work.bottom - SIDE as i32 - 24 });
                // Keep the handle reachable after a monitor is removed or rearranged.
                let position = POINT {
                    x: position.x.clamp(work.left - INSET, (work.right - INSET - CENTER as i32).max(work.left - INSET)),
                    y: position.y.clamp(work.top - INSET, (work.bottom - INSET - CENTER as i32).max(work.top - INSET)),
                };
                let visual = Surface::new(SIDE)?;
                let handle = Surface::new(CENTER)?;
                let style = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
                let halo = CreateWindowExW(style | WS_EX_TRANSPARENT, w!("Sonograph99Miniaturize"),
                    w!("Sonograph 99 Halo"), WS_POPUP, position.x, position.y,
                    SIDE as i32, SIDE as i32, None, None, Some(instance), None)?;
                let center = match CreateWindowExW(style, w!("Sonograph99Miniaturize"),
                    w!("Halo: drag center; double-click or right-click to restore"), WS_POPUP,
                    position.x + INSET, position.y + INSET, CENTER as i32, CENTER as i32,
                    Some(halo), None, Some(instance), None) {
                    Ok(center) => center,
                    Err(error) => { let _ = DestroyWindow(halo); return Err(error); }
                };
                let session = Self { halo, center, main,
                    input: Box::new(Input { restore: Cell::new(false), drag: Cell::new(None), halo }),
                    visual, handle, avatar: Avatar::new() };
                SetWindowLongPtrW(center, GWLP_USERDATA, session.input.as_ref() as *const Input as isize);
                // Render both surfaces successfully before hiding the player.
                session.visual.present(halo, &session.avatar.halo, position)?;
                session.handle.present(center, &session.avatar.center,
                    POINT { x: position.x + INSET, y: position.y + INSET })?;
                let _ = ShowWindow(main, SW_HIDE);
                let _ = ShowWindow(halo, SW_SHOWNOACTIVATE);
                let _ = ShowWindow(center, SW_SHOWNOACTIVATE);
                Ok(session)
            }
        }

        fn position(&self) -> windows::core::Result<POINT> {
            let mut rect = RECT::default();
            unsafe { GetWindowRect(self.center, &mut rect)?; }
            Ok(POINT { x: rect.left - INSET, y: rect.top - INSET })
        }

        fn frame(&mut self, audio: &Frame, palette: &Palette, dt: f32, rate: usize, pulse: bool) -> windows::core::Result<()> {
            self.avatar.frame(audio, palette, dt, rate, pulse);
            let position = self.position()?;
            self.visual.present(self.halo, &self.avatar.halo, position)?;
            self.handle.present(self.center, &self.avatar.center,
                POINT { x: position.x + INSET, y: position.y + INSET })
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            unsafe {
                SetWindowLongPtrW(self.center, GWLP_USERDATA, 0);
                if self.input.drag.get().is_some() { let _ = ReleaseCapture(); }
                let _ = DestroyWindow(self.center);
                let _ = DestroyWindow(self.halo);
                // Showing without moving or resizing preserves normal/maximized/fullscreen placement.
                let _ = ShowWindow(self.main, SW_SHOW);
                let _ = SetForegroundWindow(self.main);
            }
        }
    }

    #[derive(Default)]
    pub struct Miniaturize {
        session: Option<Session>,
        position: Option<POINT>,
    }

    impl Miniaturize {
        pub fn active(&self) -> bool { self.session.is_some() }
        pub fn restore_requested(&self) -> bool {
            self.session.as_ref().is_some_and(|s| s.input.restore.get())
        }
        pub fn set(&mut self, window: &minifb::Window, enabled: bool) -> Result<(), String> {
            if enabled == self.active() { return Ok(()); }
            if enabled {
                self.session = Some(Session::new(HWND(window.get_window_handle()), self.position)
                    .map_err(|e| format!("Could not miniaturize: {e}"))?);
            } else if let Some(session) = self.session.take() {
                self.position = session.position().ok();
                drop(session);
            }
            Ok(())
        }
        pub fn frame(&mut self, audio: &Frame, palette: &Palette, dt: f32, rate: usize, pulse: bool) -> Result<(), String> {
            if let Some(session) = &mut self.session {
                session.frame(audio, palette, dt, rate, pulse).map_err(|e| format!("Could not draw mini Halo: {e}"))?;
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        #[ignore = "Creates native windows to check overlay input, alpha composition and restoration"]
        fn native_miniaturize_round_trip() {
            let mut window = minifb::Window::new("Sonograph Miniaturize check", 640, 360,
                minifb::WindowOptions { resize: true, ..Default::default() }).unwrap();
            window.set_target_fps(60);
            let main = HWND(window.get_window_handle());
            let mut mini = Miniaturize::default();
            let mut fullscreen = crate::fullscreen::Fullscreen::default();
            let mut app = crate::App::new(&crate::Options {
                source: crate::audio::Source::Test, mode: 6, auto: true, ..Default::default()
            }, false);
            unsafe {
                for state in 0..3 {
                    if state == 1 { let _ = ShowWindow(main, SW_MAXIMIZE); }
                    if state == 2 { fullscreen.set(&window, true).unwrap(); }
                    window.update();
                    let mut before = RECT::default();
                    GetWindowRect(main, &mut before).unwrap();
                    let style = GetWindowLongPtrW(main, GWL_STYLE);
                    let maximized = IsZoomed(main).as_bool();
                    let auto_time = app.show.auto.t;
                    let mode = app.show.mode;
                    mini.set(&window, true).unwrap();
                    for _ in 0..30 {
                        app.pump_audio(1.0 / 60.0);
                        mini.frame(&app.analyzer.frame, &app.pal, 1.0 / 60.0, app.show.rate_index(), app.show.pulse).unwrap();
                        window.update();
                    }
                    assert!(!IsWindowVisible(main).as_bool());
                    assert_eq!(app.show.mode, mode);
                    assert_eq!(app.show.auto.t, auto_time);
                    let session = mini.session.as_ref().unwrap();
                    assert!(IsWindowVisible(session.halo).as_bool());
                    assert!(IsWindowVisible(session.center).as_bool());
                    assert_ne!(GetWindowLongPtrW(session.halo, GWL_EXSTYLE) & WS_EX_TRANSPARENT.0 as isize, 0);
                    assert_ne!(GetWindowLongPtrW(session.halo, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize, 0);
                    assert_eq!(GetWindowLongPtrW(session.center, GWL_EXSTYLE) & WS_EX_TRANSPARENT.0 as isize, 0);
                    let position = session.position().unwrap();
                    let center = POINT { x: position.x + SIDE as i32 / 2, y: position.y + SIDE as i32 / 2 };
                    assert_eq!(WindowFromPoint(center), session.center, "Only the central disc takes input");
                    for point in [POINT { x: position.x + 240, y: center.y },
                        POINT { x: position.x + INSET, y: position.y + INSET }, position] {
                        let hit = WindowFromPoint(point);
                        assert_ne!(hit, session.halo, "Glowing spokes must pass clicks to other applications");
                        assert_ne!(hit, session.center, "Transparent corners must pass clicks through");
                    }
                    // Keep animation running during manual dragging rather than entering a modal move loop.
                    let mut cursor = POINT::default();
                    GetCursorPos(&mut cursor).unwrap();
                    SetCursorPos(center.x, center.y).unwrap();
                    SendMessageW(session.center, WM_LBUTTONDOWN, Some(WPARAM(1)), Some(LPARAM(0)));
                    SetCursorPos(center.x - 43, center.y - 29).unwrap();
                    SendMessageW(session.center, WM_MOUSEMOVE, Some(WPARAM(1)), Some(LPARAM(0)));
                    SendMessageW(session.center, WM_LBUTTONUP, Some(WPARAM(0)), Some(LPARAM(0)));
                    SetCursorPos(cursor.x, cursor.y).unwrap();
                    let moved = session.position().unwrap();
                    assert_eq!((moved.x, moved.y), (position.x - 43, position.y - 29));
                    let mut halo_rect = RECT::default();
                    GetWindowRect(session.halo, &mut halo_rect).unwrap();
                    assert_eq!((halo_rect.left, halo_rect.top), (moved.x, moved.y));
                    if state == 0 {
                        std::fs::create_dir_all("target/miniaturize-review").unwrap();
                        session.avatar.show.composite.save_bmp("target/miniaturize-review/halo-source.bmp").unwrap();
                        // Allow desktop composition to present the move before sampling its pixels.
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        // Capture only the avatar's bounds, showing actual desktop alpha composition.
                        // The player is DPI-unaware: window coordinates are logical, whereas
                        // the desktop DC samples physical pixels. Convert both bounds.
                        let mut physical = moved;
                        let mut end = POINT { x: moved.x + SIDE as i32, y: moved.y + SIDE as i32 };
                        use windows::Win32::UI::HiDpi::LogicalToPhysicalPointForPerMonitorDPI;
                        LogicalToPhysicalPointForPerMonitorDPI(Some(session.halo), &mut physical).ok().unwrap();
                        LogicalToPhysicalPointForPerMonitorDPI(Some(session.halo), &mut end).ok().unwrap();
                        let side = (end.x - physical.x) as usize;
                        let capture = Surface::new(side).unwrap();
                        let desktop = GetDC(None);
                        let copied = BitBlt(capture.dc, 0, 0, side as i32, side as i32,
                            Some(desktop), physical.x, physical.y, SRCCOPY | CAPTUREBLT);
                        ReleaseDC(None, desktop);
                        copied.unwrap();
                        let _ = GdiFlush();
                        let mut canvas = Canvas::new(side, side);
                        canvas.px.copy_from_slice(std::slice::from_raw_parts(capture.bits, side * side));
                        canvas.save_bmp("target/miniaturize-review/native-halo.bmp").unwrap();
                    }
                    let restore_message = if state == 1 { WM_RBUTTONUP } else { WM_LBUTTONDBLCLK };
                    SendMessageW(session.center, restore_message, Some(WPARAM(0)), Some(LPARAM(0)));
                    assert!(mini.restore_requested());
                    mini.set(&window, false).unwrap();
                    window.update();
                    assert!(!mini.active());
                    assert!(IsWindowVisible(main).as_bool());
                    let mut after = RECT::default();
                    GetWindowRect(main, &mut after).unwrap();
                    assert_eq!(before, after);
                    assert_eq!(GetWindowLongPtrW(main, GWL_STYLE), style);
                    assert_eq!(IsZoomed(main).as_bool(), maximized);
                }
                fullscreen.set(&window, false).unwrap();
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    #[derive(Default)]
    pub struct Miniaturize;
    impl Miniaturize {
        pub fn active(&self) -> bool { false }
        pub fn restore_requested(&self) -> bool { false }
        pub fn set(&mut self, _: &minifb::Window, enabled: bool) -> Result<(), String> {
            if enabled { Err("Miniaturize is currently available on Windows.".into()) } else { Ok(()) }
        }
        pub fn frame(&mut self, _: &Frame, _: &Palette, _: f32, _: usize, _: bool) -> Result<(), String> { Ok(()) }
    }
}

pub use platform::Miniaturize;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_halo_and_circular_handle_have_valid_premultiplied_alpha() {
        let mut avatar = Avatar::new();
        let mut canvas = Canvas::new(SIDE, SIDE);
        for skin in sonograph_core::palette::SKINS.iter() {
            canvas.fill(skin.screen);
            alpha_pixels(&canvas, &mut avatar.halo, &mut avatar.center, skin.screen);
            assert!(avatar.halo.iter().all(|p| *p == 0), "{}: silence must stay transparent", skin.name);
            assert_eq!(avatar.center[0], 0, "The handle's corners must pass clicks through");
            assert!(avatar.center[CENTER / 2 * CENTER + CENTER / 2] >> 24 > 0, "Silence must keep the handle usable");
        }
        canvas.fill([250, 100, 30]);
        alpha_pixels(&canvas, &mut avatar.halo, &mut avatar.center, SCREEN);
        assert_eq!(avatar.halo[0], 0);
        assert_eq!(avatar.halo[SIDE / 2 * SIDE + SIDE / 2], 0, "Only the input surface draws the center");
        for &p in avatar.halo.iter().chain(&avatar.center) {
            let alpha = p >> 24;
            for color in unpack(p) { assert!(color as u32 <= alpha); }
        }
    }

    #[test]
    fn avatar_reacts_to_audio_and_always_stays_halo() {
        let mut avatar = Avatar::new();
        let mut analyzer = sonograph_core::analysis::Analyzer::new();
        let palette = Palette::new(0);
        avatar.frame(&analyzer.frame, &palette, 1.0 / 60.0, 2, true);
        let quiet = avatar.halo.clone();
        analyzer.frame.spec.fill(0.7);
        analyzer.frame.bass = 0.8;
        analyzer.frame.level = 0.6;
        for _ in 0..20 { avatar.frame(&analyzer.frame, &palette, 1.0 / 60.0, 2, true); }
        assert_ne!(avatar.halo, quiet);
        assert_eq!(avatar.show.mode, 3);
        assert!(!avatar.show.auto.on);
    }
}
