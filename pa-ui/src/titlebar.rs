//! The custom title bar's Windows side.
//!
//! The frame is not removed - it is made invisible. The window keeps
//! `WS_CAPTION|WS_THICKFRAME`, so Windows still gives us the DWM shadow, rounded
//! corners, Aero Snap, the resize loop and Snap Layouts. `WM_NCCALCSIZE` reports
//! no non-client area, which is what leaves no caption to draw; `WM_NCHITTEST`
//! then tells Windows which pixels are which:
//!
//! * the top strip is `HTCAPTION`, so dragging, double-click-to-maximise and
//!   drag-to-edge snapping are all handled natively;
//! * the outer few pixels are the resize borders;
//! * the maximise button answers `HTMAXBUTTON`, which is what makes Windows 11
//!   offer its Snap Layouts flyout on hover;
//! * the other two buttons answer `HTCLIENT`, so Slint draws and handles them.
//!
//! The title bar itself is drawn in Slint (`TitleBar` in ui/app.slint) and has to
//! agree with the numbers here: \`HEIGHT\` and \`BUTTON\`.

/// Logical height of the bar, matching the Slint `TitleBar` component.
pub const HEIGHT: i32 = 32;
/// Logical width of one caption button (Windows 11 uses 46).
pub const BUTTON: i32 = 46;
/// How many caption buttons sit at the right edge.
const BUTTONS: i32 = 3;
/// Logical geometry of the right column's fold mark, which is drawn in this strip
/// and is a Slint button: measured from the window's right edge to its left side,
/// and its width. These must agree with the TitleBar in `ui/app.slint` - a mark
/// left inside HTCAPTION is a window drag, and its click never arrives.
const FOLD_INSET: i32 = 172;
const FOLD: i32 = 26;

/// Width of the clickable icon block at the left of the bar. Without this the icon
/// would fall inside HTCAPTION, Windows would treat the press as a window drag, and
/// the click would never reach Slint to open the system menu.
const ICON: i32 = 34;

/// Attach to this process's main window. Returns false until the window exists.
pub fn init() -> bool {
    #[cfg(windows)]
    {
        imp::init()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Close the way the native close button would, so anything listening for a
/// close request still sees one.
pub fn close() {
    #[cfg(windows)]
    imp::close();
}

/// Open the window's own system menu under the caption icon, as the native
/// caption does on left-click.
pub fn show_system_menu() {
    #[cfg(windows)]
    imp::show_system_menu();
}

/// True while the pointer is over the maximise button. Windows owns that button
/// through `HTMAXBUTTON`, so Slint never sees the mouse enter or leave it and the
/// hover state has to be sampled.
pub fn cursor_over_maximize() -> bool {
    #[cfg(windows)]
    {
        imp::cursor_over_maximize()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicIsize, Ordering};

    static HWND: AtomicIsize = AtomicIsize::new(0);

    const WM_NCCALCSIZE: u32 = 0x0083;
    const WM_NCHITTEST: u32 = 0x0084;
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
    #[derive(Default, Clone, Copy)]
    struct Point {
        x: i32,
        y: i32,
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
        fn GetCursorPos(point: *mut Point) -> i32;
        fn GetSystemMenu(hwnd: isize, revert: i32) -> isize;
        fn TrackPopupMenu(menu: isize, flags: u32, x: i32, y: i32, reserved: i32, hwnd: isize, rect: *const isize) -> i32;
        fn SetForegroundWindow(hwnd: isize) -> i32;
        fn PostMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> i32;
        fn EnumWindows(callback: extern "system" fn(isize, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: isize) -> i32;
        fn GetSystemMetrics(index: i32) -> i32;
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

    pub(super) fn init() -> bool {
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
            true
        }
    }

    /// The native caption's left-click behaviour: drop the window's system menu
    /// under the icon. Windows owns that menu, so the entries and their state are
    /// the real ones - Restore/Move/Size/Minimise/Maximise/Close, enablement and all.
    pub(super) fn show_system_menu() {
        const TPM_RIGHTBUTTON: u32 = 0x0002;
        const TPM_RETURNCMD: u32 = 0x0100;
        unsafe {
            let hwnd = HWND.load(Ordering::SeqCst);
            if hwnd == 0 {
                return;
            }
            let menu = GetSystemMenu(hwnd, 0);
            if menu == 0 {
                return;
            }
            let mut window = Rect::default();
            if GetWindowRect(hwnd, &mut window) == 0 {
                return;
            }
            let caption = (super::HEIGHT as f64 * scale(hwnd)).round() as i32;
            let x = window.left + 4;
            let y = window.top + caption;

            // A menu opened without the window foreground would not dismiss on the
            // first click elsewhere, which is the classic stuck-menu bug.
            SetForegroundWindow(hwnd);
            let chosen = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD, x, y, 0, hwnd, std::ptr::null());
            PostMessageW(hwnd, 0 /* WM_NULL */, 0, 0);
            if chosen != 0 {
                PostMessageW(hwnd, WM_SYSCOMMAND, chosen as usize, 0);
            }
        }
    }

    pub(super) fn close() {
        let hwnd = HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
        }
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
                // Collapsed sidebar expand button (left 48px) and expanded sidebar
                // collapse button (194..228px) live in the top strip and are Slint controls.
                if x < inset_x + (48.0 * scale).round() as i32 {
                    return HTCLIENT;
                }
                let sb_fold_left = inset_x + (194.0 * scale).round() as i32;
                let sb_fold_right = inset_x + (228.0 * scale).round() as i32;
                if x >= sb_fold_left && x < sb_fold_right {
                    return HTCLIENT;
                }
                // The right column's fold sits further along the same strip and is
                // also Slint's to answer; without this carve-out the press is taken
                // as a caption drag and the mark never fires.
                let fold_w = (super::FOLD as f64 * scale).round() as i32;
                let fold_left = width - inset_x - (super::FOLD_INSET as f64 * scale).round() as i32;
                if x >= fold_left && x < fold_left + fold_w {
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
                    hit_test(hwnd, x, y)
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
                    result
                }

                _ => DefSubclassProc(hwnd, msg, wparam, lparam),
            }
        }
    }

    pub(super) fn cursor_over_maximize() -> bool {
        unsafe {
            let hwnd = HWND.load(Ordering::SeqCst);
            if hwnd == 0 || GetSystemMetrics(23 /* SM_SWAPBUTTON */) == -1 {
                return false;
            }
            let mut cursor = Point::default();
            if GetCursorPos(&mut cursor) == 0 {
                return false;
            }
            let mut window = Rect::default();
            if GetWindowRect(hwnd, &mut window) == 0 {
                return false;
            }
            let scale = scale(hwnd);
            let (inset_x, inset_y) = if IsZoomed(hwnd) != 0 { frame() } else { (0, 0) };
            let width = window.right - window.left;
            let button = (super::BUTTON as f64 * scale).round() as i32;
            let caption = inset_y + (super::HEIGHT as f64 * scale).round() as i32;
            let buttons_left = width - inset_x - button * super::BUTTONS;
            let x = cursor.x - window.left;
            let y = cursor.y - window.top;
            y >= inset_y && y < caption && x >= buttons_left + button && x < buttons_left + button * 2
        }
    }
}
