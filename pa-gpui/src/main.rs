mod app;
mod assets;
mod daemon;
mod settings;
mod theme;
mod transcript;

use app::PaApp;
use assets::AppAssets;
use gpui_kit::component::{Theme, ThemeMode, TitleBar};
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
        Theme::change(
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
        window_options.inactive_frame_interval = None;
        window_options.low_power_frame_interval = None;
        window_options.thermal_frame_interval = None;

        gpui_kit::open_window(window_options, cx, |window, cx| {
            cx.new(|cx| PaApp::new(initial_settings, window, cx))
        })
        .expect("Failed to open Prime Agent GPUI window");
    });
}
