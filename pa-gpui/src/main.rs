mod app;
mod daemon;
mod transcript;

use app::PaApp;
use gpui_kit::assets::Assets;
use gpui_kit::component::{Theme, ThemeMode, TitleBar};
use gpui_kit::*;

fn main() {
    let app = gpui_kit::application().with_assets(Assets);

    app.run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);

        let mut window_options = TitleBar::window_options();
        window_options.window_bounds = Some(WindowBounds::centered(size(px(1320.), px(860.)), cx));

        gpui_kit::open_window(window_options, cx, |window, cx| {
            cx.new(|cx| PaApp::new(window, cx))
        })
        .expect("Failed to open Prime Agent GPUI window");
    });
}
