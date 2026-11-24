mod activity_sheet;
mod app;
mod assets;
mod composer;
mod composer_model;
mod daemon;
mod dev_errors;
mod diff_git;
mod diff_model;
mod diff_view;
mod popups;
mod providers;
mod right_sidebar;
mod roster;
mod settings;
mod settings_harness;
mod sidebar;
mod workspace_ui;
mod slash;
mod theme;
mod transcript;
mod transcript_view;

use app::PaApp;
use assets::AppAssets;
use gpui_kit::component::{ThemeMode, TitleBar};
use gpui_kit::*;
use settings::AppSettings;

fn main() {
    // On Windows multi-refresh-rate setups (e.g. 200Hz primary + 60Hz virtual/secondary display),
    // GPUI's DirectComposition + WS_EX_NOREDIRECTIONBITMAP path locks DWM window dragging to 60fps.
    // Using the standard HWND flip-model swap chain restores full native monitor refresh rate dragging.
    #[cfg(target_os = "windows")]
    if std::env::var_os("GPUI_DISABLE_DIRECT_COMPOSITION").is_none() {
        // SAFETY: Called at the very start of main() before any worker threads are spawned.
        unsafe {
            std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "1");
        }
    }

    let app = gpui_kit::application().with_assets(AppAssets);

    app.run(move |cx| {
        gpui_kit::init(cx);
        let initial_settings = AppSettings::load();
        theme::apply_theme(
            if initial_settings.dark_mode {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            None,
            cx,
        );

        let mut window_options = TitleBar::window_options();
        window_options.window_bounds = Some(WindowBounds::centered(size(px(1240.), px(840.)), cx));
        window_options.window_min_size = Some(size(px(244.0 + composer::CHAT_COLUMN_MIN_WIDTH + 4.0), px(520.0)));
        window_options.inactive_frame_interval = None;
        window_options.low_power_frame_interval = None;
        window_options.thermal_frame_interval = None;

        gpui_kit::open_window(window_options, cx, |window, cx| {
            #[cfg(target_os = "windows")]
            win_drag::install(window);
            cx.new(|cx| PaApp::new(initial_settings, window, cx))
        })
        .expect("Failed to open Prime Agent GPUI window");
    });
}

#[cfg(target_os = "windows")]
mod win_drag {
    use gpui_kit::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::cell::Cell;
    use std::ffi::c_void;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTSIZE, LR_SHARED,
        WM_DPICHANGED, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_MOVE, WM_NCDESTROY, WM_SETICON,
        WM_TIMER,
    };

    // Matches `SIZE_MOVE_LOOP_TIMER_ID` in `gpui-pre-windows/src/events.rs`.
    const SIZE_MOVE_LOOP_TIMER_ID: usize = 1;
    const SUBCLASS_ID: usize = 0x5041_4452;

    // Cell fields allow re-entrant Win32 dispatch without creating aliased `&mut` references.
    #[derive(Default)]
    struct DragThrottleState {
        in_size_move: Cell<bool>,
        deferred_move: Cell<Option<isize>>,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Action {
        Forward,
        Swallow,
        ForwardThenFlushMove(isize),
    }

    impl DragThrottleState {
        fn handle(&self, msg: u32, wparam: usize, lparam: isize) -> Action {
            match msg {
                WM_ENTERSIZEMOVE => {
                    self.in_size_move.set(true);
                    self.deferred_move.set(None);
                    Action::Forward
                }
                WM_TIMER if self.in_size_move.get() && wparam == SIZE_MOVE_LOOP_TIMER_ID => {
                    Action::Swallow
                }
                WM_MOVE if self.in_size_move.get() => {
                    self.deferred_move.set(Some(lparam));
                    Action::Swallow
                }
                WM_DPICHANGED if self.in_size_move.get() => match self.deferred_move.take() {
                    Some(lp) => Action::ForwardThenFlushMove(lp),
                    None => Action::Forward,
                },
                WM_EXITSIZEMOVE => {
                    self.in_size_move.set(false);
                    match self.deferred_move.take() {
                        Some(lp) => Action::ForwardThenFlushMove(lp),
                        None => Action::Forward,
                    }
                }
                _ => Action::Forward,
            }
        }
    }

    pub fn install(window: &Window) {
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return;
        };
        let hwnd = HWND(win32.hwnd.get() as *mut c_void);
        // SAFETY: `GetModuleHandleW(None)` retrieves the current executable's HMODULE,
        // `MAKEINTRESOURCEW(1)` references icon resource ID 1 embedded by `build.rs`, and
        // `SendMessageW(WM_SETICON)` sets the HWND's small and large icons for taskbar and Alt-Tab.
        unsafe {
            if let Ok(hmod) = GetModuleHandleW(None) {
                let res_id = windows::core::PCWSTR(1usize as *const u16);
                if let Ok(big) =
                    LoadImageW(Some(hmod.into()), res_id, IMAGE_ICON, 0, 0, LR_DEFAULTSIZE | LR_SHARED)
                {
                    let _ = SendMessageW(
                        hwnd,
                        WM_SETICON,
                        Some(WPARAM(ICON_BIG as usize)),
                        Some(LPARAM(big.0 as isize)),
                    );
                }
                if let Ok(small) =
                    LoadImageW(Some(hmod.into()), res_id, IMAGE_ICON, 16, 16, LR_SHARED)
                {
                    let _ = SendMessageW(
                        hwnd,
                        WM_SETICON,
                        Some(WPARAM(ICON_SMALL as usize)),
                        Some(LPARAM(small.0 as isize)),
                    );
                }
            }
        }
        let state_ptr = Box::into_raw(Box::new(DragThrottleState::default()));
        // SAFETY: `hwnd` is a live Win32 window created on the UI thread, `drag_subclass_proc`
        // matches `SUBCLASSPROC`, and `state_ptr` is uniquely owned by this subclass until freed
        // in `WM_NCDESTROY` (or immediately below if registration fails).
        let ok = unsafe {
            SetWindowSubclass(hwnd, Some(drag_subclass_proc), SUBCLASS_ID, state_ptr as usize)
        };
        if !ok.as_bool() {
            // SAFETY: Registration failed so `drag_subclass_proc` will never read `state_ptr`.
            unsafe {
                drop(Box::from_raw(state_ptr));
            }
        }
    }

    unsafe extern "system" fn drag_subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        uid_subclass: usize,
        dw_ref_data: usize,
    ) -> LRESULT {
        if msg == WM_NCDESTROY {
            // SAFETY: Removing the subclass before freeing `dw_ref_data` ensures no future
            // callback can dereference it, and `dw_ref_data` came from `Box::into_raw`.
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(drag_subclass_proc), uid_subclass);
                if dw_ref_data != 0 {
                    drop(Box::from_raw(dw_ref_data as *mut DragThrottleState));
                }
                return DefSubclassProc(hwnd, msg, wparam, lparam);
            }
        }

        let action = if dw_ref_data != 0 {
            // SAFETY: `dw_ref_data` is valid for the lifetime of this subclass until `WM_NCDESTROY`,
            // Win32 dispatches messages for `hwnd` strictly on its creating UI thread, and
            // `DragThrottleState` uses `Cell` interior mutability so shared borrows are sound
            // even under re-entrant message dispatch.
            let state = unsafe { &*(dw_ref_data as *const DragThrottleState) };
            state.handle(msg, wparam.0, lparam.0)
        } else {
            Action::Forward
        };

        match action {
            Action::Swallow => LRESULT(0),
            Action::Forward => {
                // SAFETY: Forwarding unmodified message parameters to the next subclass handler.
                unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
            }
            Action::ForwardThenFlushMove(move_lparam) => {
                // SAFETY: Forwarding the current message then replaying the latest deferred
                // `WM_MOVE` payload so GPUI reconciles final window origin and display state.
                unsafe {
                    let res = DefSubclassProc(hwnd, msg, wparam, lparam);
                    let _ = DefSubclassProc(hwnd, WM_MOVE, WPARAM(0), LPARAM(move_lparam));
                    res
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::UI::WindowsAndMessaging::WM_SIZE;

        #[::core::prelude::v1::test]
        fn defers_moves_and_swallows_sizemove_timer_until_exit() {
            let s = DragThrottleState::default();

            assert_eq!(s.handle(WM_MOVE, 0, 100), Action::Forward);
            assert_eq!(s.handle(WM_TIMER, SIZE_MOVE_LOOP_TIMER_ID, 0), Action::Forward);

            assert_eq!(s.handle(WM_ENTERSIZEMOVE, 0, 0), Action::Forward);
            assert_eq!(s.handle(WM_TIMER, SIZE_MOVE_LOOP_TIMER_ID, 0), Action::Swallow);
            assert_eq!(s.handle(WM_TIMER, 99, 0), Action::Forward);
            assert_eq!(s.handle(WM_MOVE, 0, 111), Action::Swallow);
            assert_eq!(s.handle(WM_MOVE, 0, 222), Action::Swallow);
            assert_eq!(s.handle(WM_SIZE, 0, 333), Action::Forward);

            assert_eq!(
                s.handle(WM_EXITSIZEMOVE, 0, 0),
                Action::ForwardThenFlushMove(222)
            );
            assert_eq!(s.handle(WM_MOVE, 0, 444), Action::Forward);
            assert_eq!(s.handle(WM_EXITSIZEMOVE, 0, 0), Action::Forward);
        }

        #[::core::prelude::v1::test]
        fn dpi_change_mid_drag_flushes_pending_move() {
            let s = DragThrottleState::default();
            assert_eq!(s.handle(WM_ENTERSIZEMOVE, 0, 0), Action::Forward);
            assert_eq!(s.handle(WM_MOVE, 0, 555), Action::Swallow);
            assert_eq!(
                s.handle(WM_DPICHANGED, 0, 0),
                Action::ForwardThenFlushMove(555)
            );
            assert_eq!(s.handle(WM_EXITSIZEMOVE, 0, 0), Action::Forward);
        }
    }
}
