mod app;
mod daemon;
mod settings;
mod transcript;

use app::PaApp;
use gpui_kit::assets::Assets;
use gpui_kit::component::{Theme, ThemeMode, TitleBar};
use gpui_kit::*;
use settings::AppSettings;

fn main() {
    let app = gpui_kit::application().with_assets(Assets);

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
        window_options.window_bounds = Some(WindowBounds::centered(size(px(1320.), px(860.)), cx));

        gpui_kit::open_window(window_options, cx, |window, cx| {
            cx.new(|cx| PaApp::new(initial_settings, window, cx))
        })
        .expect("Failed to open Prime Agent GPUI window");
    });
}
