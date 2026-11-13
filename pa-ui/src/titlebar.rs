//! The custom title bar's Windows side.
//!
//! A frame that is not removed but made invisible: `WS_CAPTION|WS_THICKFRAME`
//! still gives DWM its shadow, rounded corners, Aero Snap and Snap Layouts.
//! `WM_NCCALCSIZE` reports no non-client area; `WM_NCHITTEST` then says which
//! pixels are which - the top strip is HTCAPTION, the outer few are resize
//! borders, maximise answers HTMAXBUTTON for Snap Layouts, and the other two
//! caption buttons answer HTCLIENT so Slint draws and handles them.
//!
//! The bar itself is drawn in Slint (`TitleBar` in ui/app.slint) and has to agree
//! with the numbers here: `HEIGHT`, `BUTTON`, and the two collapse buttons the
//! columns draw into the strip.

/// Logical height of the bar, matching the Slint `TitleBar` component.
pub const HEIGHT: i32 = 32;
/// Logical width of one caption button (Windows 11 uses 46).
pub const BUTTON: i32 = 46;
/// How many caption buttons sit at the right edge.
const BUTTONS: i32 = 3;
/// Logical geometry of the right column's collapse button, which is drawn in this
/// strip and is a Slint button: measured from the window's right edge to its left
/// side, and its width. These must agree with the TitleBar in `ui/app.slint` - a
/// mark left inside HTCAPTION is a window drag, and its click never arrives.
const COLLAPSE_INSET: i32 = 202;
const COLLAPSE: i32 = 56;

/// The sidebar when it is folded away: a strip of controls whose top mark is the
/// way back open, so the whole strip is Slint's to answer.
const STRIP: i32 = 48;

/// Logical geometry of the two controls at the right of the sidebar's header
/// band, inside this strip: left edge and width, from the window's left. The
/// column is 232 wide and its band is inset 8 either side, so the 30px collapse
/// button starts at `232 - 8 - 30`, and the 30px magnifier sits one gap to its
/// left. The band's own right edge is the span the client area has to answer
/// for, and both buttons sit inside it. These must agree with the column in
/// `ui/app.slint` for the same reason the right column's do.
const SIDEBAR_BAND_RIGHT: i32 = 224;
const SIDEBAR_CONTROLS_LEFT: i32 = 160;

/// Attach to this process's main window. Returns false until the window exists.
pub fn init(ui: slint::Weak<crate::AppWindow>) -> bool {
    #[cfg(windows)]
    {
        imp::init(ui)
    }
    #[cfg(not(windows))]
    {
        let _ = ui;
        true
    }
}

pub fn has_window() -> bool {
    #[cfg(windows)]
    {
        imp::has_window()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Close the way the native close button would, so anything listening for a
/// close request still sees one.
pub fn close() -> Result<(), String> {
    #[cfg(windows)]
    {
        imp::close()
    }
    #[cfg(not(windows))]
    {
        Err("native window close is only wired on Windows".to_string())
    }
}

#[cfg(windows)]
mod imp {
    use crate::AppWindow;
    use slint::Weak;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use std::sync::{Mutex, OnceLock};

    static HWND: AtomicIsize = AtomicIsize::new(0);
    static UI: OnceLock<Weak<AppWindow>> = OnceLock::new();
    static HOVER: AtomicBool = AtomicBool::new(false);
    static PUBLISHED: Mutex<(bool, bool)> = Mutex::new((false, false));

    const WM_SIZE: u32 = 0x0005;
    const WM_NCCALCSIZE: u32 = 0x0083;
    const WM_NCHITTEST: u32 = 0x0084;
    const WM_MOUSELEAVE: u32 = 0x02A3;
    const WM_NCLBUTTONDOWN: u32 = 0x00A1;
    const WM_SYSCOMMAND: u32 = 0x0112;
    const WM_CLOSE: u32 = 0x0010;
    const SC_MAXIMIZE: usize = 0xF030;
    const SC_RESTORE: usize = 0xF120;
    const WM_DPICHANGED: u32 = 0x02E0;

    const HTCLIENT: isize = 1;
    const HTCAPTION: isize = 2;
    const HTMAXBUTTON: isize = 9;
    const HTLEFT: isize = 10;
    const HTRIGHT: isize = 11;
    const HTTOP: isize = 12;
    const HTTOPLEFT: isize = 13;
    const HTTOPRIGHT: isize = 14;
    const HTBOTTOM: isize = 15;
    const HTBOTTOMLEFT: isize = 16;
    const HTBOTTOMRIGHT: isize = 17;

    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_FRAMECHANGED: u32 = 0x0020;

    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_NCRENDERING_POLICY: u32 = 2;
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMNCRP_ENABLED: i32 = 2;
    const DWMWCP_ROUND: i32 = 2;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct TrackMouse {
        size: u32,
        flags: u32,
        hwnd: isize,
        hover_time: u32,
    }

    /// POINT ptReserved, ptMaxSize, ptMaxPosition, ptMinTrackSize, ptMaxTrackSize
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct NcCalcSizeParams {
        rgrc: [Rect; 3],
        lppos: *mut isize,
    }

    /// Resize-border thickness Windows adds around a frame window.
    fn frame() -> (i32, i32) {
        unsafe {
            let padded = GetSystemMetrics(92 /* SM_CXPADDEDBORDER */);
            (
                GetSystemMetrics(32 /* SM_CXSIZEFRAME */) + padded,
                GetSystemMetrics(33 /* SM_CYSIZEFRAME */) + padded,
            )
        }
    }


    #[link(name = "user32")]
    extern "system" {
        fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
        fn GetDpiForWindow(hwnd: isize) -> u32;
        fn IsZoomed(hwnd: isize) -> i32;
        fn SetWindowPos(hwnd: isize, after: isize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
        fn PostMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> i32;
        fn EnumWindows(callback: extern "system" fn(isize, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: isize) -> i32;
        fn GetSystemMetrics(index: i32) -> i32;
        fn TrackMouseEvent(event: *mut TrackMouse) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcessId() -> u32;
    }

    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(hwnd: isize, attribute: u32, value: *const i32, size: u32) -> i32;
    }

    #[link(name = "comctl32")]
    extern "system" {
        fn SetWindowSubclass(
            hwnd: isize,
            proc_: extern "system" fn(isize, u32, usize, isize, usize, usize) -> isize,
            id: usize,
            refdata: usize,
        ) -> i32;
        fn DefSubclassProc(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
    }

    extern "system" fn first_visible(hwnd: isize, lparam: isize) -> i32 {
        unsafe {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 {
                *(lparam as *mut isize) = hwnd;
                return 0;
            }
            1
        }
    }

    fn publish(hwnd: isize) {
        let maximized = unsafe { IsZoomed(hwnd) != 0 };
        let hover = HOVER.load(Ordering::Relaxed);
        let mut published = PUBLISHED.lock().expect("titlebar state");
        if *published == (maximized, hover) {
            return;
        }
        *published = (maximized, hover);
        drop(published);
        let weak = UI.get().cloned();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.and_then(|weak| weak.upgrade()) {
                ui.set_titlebar_maximized(maximized);
                ui.set_titlebar_max_hover(hover);
            }
        });
    }

    pub(super) fn init(ui: Weak<AppWindow>) -> bool {
        let _ = UI.set(ui);
        unsafe {
            let mut hwnd: isize = 0;
            EnumWindows(first_visible, &mut hwnd as *mut isize as isize);
            if hwnd == 0 {
                return false;
            }
            if HWND.swap(hwnd, Ordering::SeqCst) == hwnd {
                return true; // already attached
            }

            // The frame is invisible, but the compositor's treatment of it is not:
            // without these the window falls back to square corners with no shadow,
            // which is what every application would look like if it only removed the
            // non-client area and stopped there. Ask for all three rather than relying
            // on the defaults a normal window happens to get.
            let dark: i32 = 1;
            DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark, 4);

            let nc_enabled: i32 = DWMNCRP_ENABLED;
            DwmSetWindowAttribute(hwnd, DWMWA_NCRENDERING_POLICY, &nc_enabled, 4);

            let rounded: i32 = DWMWCP_ROUND;
            DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &rounded, 4);

            SetWindowSubclass(hwnd, subclass, 1, 0);
            // Make Windows re-run WM_NCCALCSIZE with the new rules.
            SetWindowPos(hwnd, 0, 0, 0, 0, 0,
                SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
            publish(hwnd);
            true
        }
    }

    pub(super) fn has_window() -> bool {
        HWND.load(Ordering::SeqCst) != 0
    }

    pub(super) fn close() -> Result<(), String> {
        let hwnd = HWND.load(Ordering::SeqCst);
        if hwnd == 0 {
            return Err("window handle is not initialized".to_string());
        }
        if unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) == 0 } {
            return Err(format!("PostMessageW(WM_CLOSE) failed: {}", std::io::Error::last_os_error()));
        }
        Ok(())
    }

    fn scale(hwnd: isize) -> f64 {
        unsafe {
            let dpi = GetDpiForWindow(hwnd);
            if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 }
        }
    }

    /// Where the pointer is, in the same terms WM_NCHITTEST uses.
    fn hit_test(hwnd: isize, screen_x: i32, screen_y: i32) -> isize {
        unsafe {
            let mut window = Rect::default();
            if GetWindowRect(hwnd, &mut window) == 0 {
                return HTCLIENT;
            }
            let width = window.right - window.left;
            let height = window.bottom - window.top;
            let x = screen_x - window.left;
            let y = screen_y - window.top;
            let scale = scale(hwnd);

            // Maximised, the client is inset inside a window that overhangs the work
            // area, so every caption measurement shifts by that inset.
            let maximized = IsZoomed(hwnd) != 0;
            let (inset_x, inset_y) = if maximized { frame() } else { (0, 0) };

            if !maximized {
                let border = (6.0 * scale).round() as i32;
                let left = x < border;
                let right = x >= width - border;
                let top = y < border;
                let bottom = y >= height - border;
                match (left, right, top, bottom) {
                    (true, _, true, _) => return HTTOPLEFT,
                    (_, true, true, _) => return HTTOPRIGHT,
                    (true, _, _, true) => return HTBOTTOMLEFT,
                    (_, true, _, true) => return HTBOTTOMRIGHT,
                    (true, _, _, _) => return HTLEFT,
                    (_, true, _, _) => return HTRIGHT,
                    (_, _, true, _) => return HTTOP,
                    (_, _, _, true) => return HTBOTTOM,
                    _ => {}
                }
            }

            let caption = inset_y + (super::HEIGHT as f64 * scale).round() as i32;
            if y >= inset_y && y < caption {
                // The folded strip's top mark and the sidebar's two controls live
                // in the top strip and are Slint controls.
                if x < inset_x + (super::STRIP as f64 * scale).round() as i32 {
                    return HTCLIENT;
                }
                let sb_left = inset_x + (super::SIDEBAR_CONTROLS_LEFT as f64 * scale).round() as i32;
                let sb_right = inset_x + (super::SIDEBAR_BAND_RIGHT as f64 * scale).round() as i32;
                if x >= sb_left && x < sb_right {
                    return HTCLIENT;
                }
                // The right column's collapse button sits further along the same
                // strip and is also Slint's to answer; without this carve-out the
                // press is taken as a caption drag and the mark never fires.
                let button_w = (super::COLLAPSE as f64 * scale).round() as i32;
                let collapse_left =
                    width - inset_x - (super::COLLAPSE_INSET as f64 * scale).round() as i32;
                if x >= collapse_left && x < collapse_left + button_w {
                    return HTCLIENT;
                }
                let button = (super::BUTTON as f64 * scale).round() as i32;
                let buttons_left = width - inset_x - button * super::BUTTONS;
                if x >= buttons_left {
                    // 0 = minimise, 1 = maximise/restore, 2 = close
                    return if (x - buttons_left) / button == 1 { HTMAXBUTTON } else { HTCLIENT };
                }
                return HTCAPTION;
            }
            HTCLIENT
        }
    }

    extern "system" fn subclass(
        hwnd: isize,
        msg: u32,
        wparam: usize,
        lparam: isize,
        _id: usize,
        _refdata: usize,
    ) -> isize {
        unsafe {
            match msg {
                // No non-client area at all: the frame stays for Windows' own
                // behaviour but stops being drawn. When maximised, Windows sizes the
                // window to the work area *plus* the frame thickness (that is how a
                // normal window's border ends up off-screen) - with nothing drawn
                // there the client would hang past the edge, so take that thickness
                // back out of the client rect.
                WM_NCCALCSIZE if wparam != 0 => {
                    if IsZoomed(hwnd) != 0 {
                        let (border_x, border_y) = frame();
                        let params = &mut *(lparam as *mut NcCalcSizeParams);
                        let rect = &mut params.rgrc[0];
                        rect.left += border_x;
                        rect.top += border_y;
                        rect.right -= border_x;
                        rect.bottom -= border_y;
                    }
                    0
                }

                WM_NCHITTEST => {
                    let x = (lparam & 0xffff) as u16 as i16 as i32;
                    let y = ((lparam >> 16) & 0xffff) as u16 as i16 as i32;
                    let hit = hit_test(hwnd, x, y);
                    let hover = hit == HTMAXBUTTON;
                    if HOVER.swap(hover, Ordering::Relaxed) != hover {
                        publish(hwnd);
                    }
                    if hover {
                        let mut track = TrackMouse {
                            size: std::mem::size_of::<TrackMouse>() as u32,
                            flags: 2, // TME_LEAVE
                            hwnd,
                            hover_time: 0,
                        };
                        TrackMouseEvent(&mut track);
                    }
                    hit
                }

                WM_MOUSELEAVE => {
                    if HOVER.swap(false, Ordering::Relaxed) {
                        publish(hwnd);
                    }
                    DefSubclassProc(hwnd, msg, wparam, lparam)
                }

                WM_SIZE => {
                    let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                    publish(hwnd);
                    result
                }

                // The Snap Layouts flyout needs this button to belong to Windows, so
                // the click is forwarded rather than handled in Slint. Doing it here
                // keeps the toggle working even if the shell declines the default.
                WM_NCLBUTTONDOWN if wparam as isize == HTMAXBUTTON => {
                    let command = if IsZoomed(hwnd) == 0 { SC_MAXIMIZE } else { SC_RESTORE };
                    PostMessageW(hwnd, WM_SYSCOMMAND, command, 0);
                    0
                }

                // A DPI change moves the caption height; ask for a fresh frame pass.
                WM_DPICHANGED => {
                    let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                    SetWindowPos(hwnd, 0, 0, 0, 0, 0,
                        SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
                    publish(hwnd);
                    result
                }

                _ => DefSubclassProc(hwnd, msg, wparam, lparam),
            }
        }
    }


}
