//! The caption (title bar) icon.
//!
//! The taskbar finds its icon in pa-ui.exe's resources on its own, but the caption
//! has no such fallback: winit registers its window class with no icon at all, and
//! Slint 1.18 exposes no `Window::set_icon` (only `SystemTray::set_icon`), so nothing
//! ever sets one and the title bar draws an empty square.
//!
//! This posts the icon onto the window once it exists. The
//! Win32 entry points are declared here rather than pulled in via `windows-sys`,
//! which is not in the dependency graph at all - the same reasoning that keeps
//! rc.exe hand-run in build.rs.
//!
//! Best effort by design: if the window cannot be found or the resource is missing,
//! the caption simply stays as it was.

/// Post the exe's icon onto this process's main window. Returns true once done.
pub fn apply() -> bool {
    #[cfg(windows)]
    {
        imp::apply()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

#[cfg(windows)]
mod imp {
    use std::ptr;

    const WM_SETICON: u32 = 0x0080;
    const ICON_SMALL: usize = 0;
    const ICON_BIG: usize = 1;
    const ICON_SMALL2: usize = 2;
    const IMAGE_ICON: u32 = 1;
    const SM_CXICON: i32 = 11;
    const SM_CYICON: i32 = 12;
    const SM_CXSMICON: i32 = 49;
    const SM_CYSMICON: i32 = 50;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: extern "system" fn(isize, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: isize) -> i32;
        fn SendMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn LoadImageW(instance: isize, name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> isize;
        fn GetSystemMetrics(index: i32) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> isize;
        fn GetCurrentProcessId() -> u32;
    }

    /// Stop at the first visible top-level window owned by this process.
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

    pub(super) fn apply() -> bool {
        unsafe {
            let mut hwnd: isize = 0;
            EnumWindows(first_visible, &mut hwnd as *mut isize as isize);
            if hwnd == 0 {
                return false; // no window yet - the caller retries
            }

            // MAKEINTRESOURCE(1): the RT_GROUP_ICON that assets/icon.rc embeds.
            let module = GetModuleHandleW(ptr::null());
            let tile = ptr::dangling::<u16>();
            let small = LoadImageW(
                module, tile, IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON), 0,
            );
            let big = LoadImageW(
                module, tile, IMAGE_ICON,
                GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON), 0,
            );
            if small == 0 || big == 0 {
                return false;
            }

            SendMessageW(hwnd, WM_SETICON, ICON_SMALL, small);
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL2, small);
            SendMessageW(hwnd, WM_SETICON, ICON_BIG, big);
            true
        }
    }
}
