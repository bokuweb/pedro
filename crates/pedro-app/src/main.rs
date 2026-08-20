//! Pedro — a native reader that talks to the coding agents you already have
//! installed.

mod app;
mod chat;
mod document;
mod library;
mod palette;
mod state;
mod ui;

use gpui::{
    AnyView, App, AppContext as _, Application, Bounds, KeyBinding, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, point, px, size,
};
use gpui_component::Root;
use tracing_subscriber::EnvFilter;

use crate::app::{FocusSearch, NextPage, Pedro, PreviousPage};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);
            palette::apply_to_theme(cx);
            cx.bind_keys([
                KeyBinding::new("cmd-k", FocusSearch, None),
                // Bound in the shell's own context so that the text field's
                // bindings, which sit deeper, keep the arrows for the caret
                // while the reader is typing a question.
                KeyBinding::new("right", NextPage, Some("Pedro")),
                KeyBinding::new("left", PreviousPage, Some("Pedro")),
            ]);

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
                    // There is no title bar and no strip standing in for one.
                    // The panels run to the top edge of the window and the
                    // traffic lights sit in the sidebar's first row, which is
                    // indented to clear them and dragged to move the window.
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(18.), px(17.))),
                }),
                ..Default::default()
            };

            cx.open_window(options, |window, cx| {
                // Named rather than inferred: `Root::new` takes anything that
                // converts into an `AnyView`, which leaves `.into()` with
                // nothing to pick from.
                let pedro: AnyView = cx.new(|cx| Pedro::new(window, cx)).into();
                cx.new(|cx| Root::new(pedro, window, cx))
            })
            .expect("failed to open window");

            cx.activate(true);
        });
}
