//! Pedro — a native reader that talks to the coding agents you already have
//! installed.

mod app;
mod chat;
mod document;
mod library;
mod palette;
mod panes;
mod state;
mod ui;

use gpui::{
    AnyView, App, AppContext as _, Application, Bounds, KeyBinding, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, point, px, size,
};
use gpui_component::Root;
use gpui_component::input::{Backspace, Enter};
use tracing_subscriber::EnvFilter;

use crate::app::{
    FocusSearch, NextPage, NextTab, Pedro, PreviousPage, PreviousTab, ToggleChat, ToggleSidebar,
    ZoomIn, ZoomOut, ZoomReset,
};

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
                // Both spellings of the same key: the plus is a shifted equals
                // on most layouts, and readers press whichever they think of.
                KeyBinding::new("cmd-=", ZoomIn, None),
                KeyBinding::new("cmd-+", ZoomIn, None),
                KeyBinding::new("cmd--", ZoomOut, None),
                KeyBinding::new("cmd-0", ZoomReset, None),
                // The keys every tabbed application uses for this.
                // What every editor with two side panels uses.
                KeyBinding::new("cmd-b", ToggleSidebar, None),
                KeyBinding::new("cmd-alt-b", ToggleChat, None),
                KeyBinding::new("cmd-shift-]", NextTab, None),
                KeyBinding::new("cmd-shift-[", PreviousTab, None),
                // Registered after `gpui_component::init`, so these shadow the
                // ones it binds for the same keys in the same context.
                //
                // Enter sends the question and shift-enter breaks the line,
                // the way every chat does it. Both still insert a newline
                // first — a multi-line field is what they are for — and the
                // one that sends throws the field away anyway. `secondary` is
                // how the two are told apart afterwards.
                KeyBinding::new("enter", Enter { secondary: true }, Some("Input")),
                KeyBinding::new("shift-enter", Enter { secondary: false }, Some("Input")),
                // Emacs' backspace. macOS turns ctrl-h into one for native
                // text views, and a view that draws its own text has to say so
                // itself.
                KeyBinding::new("ctrl-h", Backspace, Some("Input")),
                KeyBinding::new("ctrl-d", gpui_component::input::Delete, Some("Input")),
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
