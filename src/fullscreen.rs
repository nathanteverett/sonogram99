//! Borderless fullscreen on the window's current monitor, preserving its placement.

#[cfg(windows)]
mod platform {
    use windows::Win32::{
        Foundation::{HWND, SetLastError, WIN32_ERROR},
        Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        UI::WindowsAndMessaging::*,
    };

    struct SavedWindow {
        style: isize,
        placement: WINDOWPLACEMENT,
    }

    #[derive(Default)]
    pub struct Fullscreen {
        saved: Option<SavedWindow>,
    }

    fn set_style(hwnd: HWND, style: isize) -> windows::core::Result<()> {
        // A zero previous style can be valid; clear and inspect the error separately.
        unsafe {
            SetLastError(WIN32_ERROR(0));
            if SetWindowLongPtrW(hwnd, GWL_STYLE, style) == 0 {
                let error = windows::core::Error::from_thread();
                if error.code().is_err() { return Err(error); }
            }
        }
        Ok(())
    }

    impl Fullscreen {
        pub fn active(&self) -> bool { self.saved.is_some() }

        pub fn set(&mut self, window: &minifb::Window, enabled: bool) -> Result<(), String> {
            if enabled == self.active() { return Ok(()); }
            self.apply(HWND(window.get_window_handle()), enabled)
                .map_err(|e| format!("Could not change fullscreen: {e}"))
        }

        fn apply(&mut self, hwnd: HWND, enabled: bool) -> windows::core::Result<()> {
            // The HWND belongs to the live minifb window, on this UI thread.
            unsafe {
                if enabled {
                    let mut placement = WINDOWPLACEMENT {
                        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                        ..Default::default()
                    };
                    GetWindowPlacement(hwnd, &mut placement)?;
                    let mut monitor = MONITORINFO {
                        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                        ..Default::default()
                    };
                    GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut monitor).ok()?;
                    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                    set_style(hwnd, style & !(WS_OVERLAPPEDWINDOW.0 as isize))?;
                    let r = monitor.rcMonitor;
                    if let Err(error) = SetWindowPos(hwnd, Some(HWND_TOP), r.left, r.top,
                        r.right - r.left, r.bottom - r.top, SWP_FRAMECHANGED | SWP_NOOWNERZORDER)
                    {
                        let _ = set_style(hwnd, style);
                        let _ = SetWindowPlacement(hwnd, &placement);
                        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0,
                            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                        return Err(error);
                    }
                    self.saved = Some(SavedWindow { style, placement });
                } else if let Some(saved) = &self.saved {
                    set_style(hwnd, saved.style)?;
                    SetWindowPlacement(hwnd, &saved.placement)?;
                    SetWindowPos(hwnd, None, 0, 0, 0, 0,
                        SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE)?;
                    self.saved = None;
                }
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::Foundation::{RECT, WPARAM, LPARAM};

        fn press_f11(window: &mut minifb::Window, app: &mut crate::App) {
            let hwnd = HWND(window.get_window_handle());
            // Exercise minifb's Windows scan-code path as well as saved bindings.
            unsafe { PostMessageW(Some(hwnd), WM_KEYDOWN, WPARAM(0x7a), LPARAM(0x0057_0001)).unwrap(); }
            window.update();
            // Windows minifb advances pressed-key durations before pumping events,
            // so the following frame exposes the newly delivered key press.
            window.update();
            let keys = window.get_keys_pressed(minifb::KeyRepeat::No);
            assert!(keys.contains(&minifb::Key::F11));
            for key in keys { app.key(key); }
            unsafe { PostMessageW(Some(hwnd), WM_KEYUP, WPARAM(0x7a), LPARAM(0xc057_0001u32 as isize)).unwrap(); }
            window.update();
            window.update();
        }

        #[test]
        #[ignore = "Creates a desktop window to verify native monitor bounds and restoration"]
        fn native_fullscreen_round_trip() {
            let mut window = minifb::Window::new("Sonograph fullscreen check", 640, 360,
                minifb::WindowOptions { resize: true, ..Default::default() }).unwrap();
            let hwnd = HWND(window.get_window_handle());
            let mut fullscreen = Fullscreen::default();
            let mut app = crate::App::new(&crate::Options {
                source: crate::audio::Source::Test, ..Default::default()
            }, false);
            app.catalogue.bindings = crate::catalogue::Bindings::parse("F11 = none").unwrap();
            unsafe {
                for maximized in [false, true] {
                    if maximized { let _ = ShowWindow(hwnd, SW_MAXIMIZE); }
                    window.update();
                    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                    let mut before = RECT::default();
                    GetWindowRect(hwnd, &mut before).unwrap();
                    let mut monitor = MONITORINFO {
                        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                        ..Default::default()
                    };
                    GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut monitor).ok().unwrap();
                    for _ in 0..2 {
                        press_f11(&mut window, &mut app);
                        assert!(app.fullscreen);
                        fullscreen.set(&window, app.fullscreen).unwrap();
                        window.update();
                        assert!(fullscreen.active());
                        let mut actual = RECT::default();
                        GetWindowRect(hwnd, &mut actual).unwrap();
                        assert_eq!(actual, monitor.rcMonitor);
                        assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE) & WS_OVERLAPPEDWINDOW.0 as isize, 0);
                        let (w, h) = window.get_size();
                        app.frame(w / 2, h / 2, 1.0 / 60.0);
                        assert_eq!(app.screen.px, app.show.composite.px);
                        window.update_with_buffer(&app.screen.px, app.screen.w, app.screen.h).unwrap();
                        press_f11(&mut window, &mut app);
                        assert!(!app.fullscreen);
                        fullscreen.set(&window, app.fullscreen).unwrap();
                        window.update();
                        assert!(!fullscreen.active());
                        GetWindowRect(hwnd, &mut actual).unwrap();
                        assert_eq!(actual, before);
                        assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), style);
                        assert_eq!(IsZoomed(hwnd).as_bool(), maximized);
                    }
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    #[derive(Default)]
    pub struct Fullscreen;
    impl Fullscreen {
        pub fn active(&self) -> bool { false }
        pub fn set(&mut self, _: &minifb::Window, enabled: bool) -> Result<(), String> {
            if enabled { Err("Desktop fullscreen is currently available on Windows.".into()) }
            else { Ok(()) }
        }
    }
}

pub use platform::Fullscreen;
