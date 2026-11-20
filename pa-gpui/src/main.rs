mod app;
mod daemon;
mod transcript;

use app::PaApp;
use gpui_kit::assets::Assets;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

fn main() {
    let app = gpui_kit::application().with_assets(Assets);

    app.run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(820.)), cx)),
            ..Default::default()
        };

        gpui_kit::open_window(window_options, cx, |window, cx| {
            cx.new(|cx| PaApp::new(window, cx))
        })
        .expect("Failed to open Prime Agent GPUI window");
    });
}
