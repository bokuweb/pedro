//! Moving a window that has no title bar to grab.
//!
//! The window is opened with a transparent titlebar and the shell draws no
//! strip standing in for one: a band whose only job is to be dragged costs the
//! reader 34 pixels of every screen, and in fullscreen it cannot even do that.
//!
//! So the rows that are already there do it. Any row can be made draggable,
//! and the two that are — the sidebar's header and the tab bar — are the two
//! that run along the top edge where a title bar would have been.

use gpui::{Context, Div, InteractiveElement as _, MouseButton, Stateful, Window};
use gpui_component::InteractiveElementExt as _;

use crate::app::Pedro;

impl Pedro {
    /// Makes `element` move the window when it is dragged, and zoom it when it
    /// is double-clicked, the way a title bar would.
    pub(crate) fn draggable(
        &self,
        element: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        element
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.window_drag_armed = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.window_drag_armed = false),
            )
            .on_mouse_move(cx.listener(|this, _, window: &mut Window, _| {
                if this.window_drag_armed {
                    // A move with the button still held is a drag, not a click.
                    this.window_drag_armed = false;
                    window.start_window_move();
                }
            }))
            .on_double_click(|_, window: &mut Window, _| {
                #[cfg(target_os = "macos")]
                window.titlebar_double_click();
                #[cfg(not(target_os = "macos"))]
                window.zoom_window();
            })
    }
}
