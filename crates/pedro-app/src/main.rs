//! Pedro — a native reader that talks to the coding agents you already have
//! installed.

mod app;
mod palette;
mod state;
mod ui;

use gpui::{
    App, Application, Bounds, KeyBinding, TitlebarOptions, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, point, px, size,
};
use gpui_component::Root;
use tracing_subscriber::EnvFilter;

use crate::app::{FocusSearch, Pedro};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);
            palette::apply_to_theme(cx);
            cx.bind_keys([KeyBinding::new("cmd-k", FocusSearch, None)]);

            let bounds = Bounds::centered(None, size(px(1280.), px(860.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(920.), px(600.))),
                // The panels are translucent (see `palette`), so what is behind
                // the window is part of the design rather than something to
                // paint over. Blurred rather than plain transparency: reading
                // is the point, and unblurred desktop under body text is not
                // something anyone can read against.
                window_background: WindowBackgroundAppearance::Blurred,
                titlebar: Some(TitlebarOptions {
                    title: Some("Pedro".into()),
                    // The shell draws its own title strip so the panels can run
                    // to the top edge of the window.
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(16.), px(11.))),
                }),
                ..Default::default()
            };

            cx.open_window(options, |window, cx| {
                let pedro = cx.new(|cx| Pedro::new(window, cx));
                cx.new(|cx| Root::new(pedro.into(), window, cx))
            })
            .expect("failed to open window");

            cx.activate(true);
        });
}
