//! The strip above everything else.
//!
//! The window is opened with a transparent titlebar so the shell can run edge
//! to edge, which means this strip is responsible for the two things the
//! system titlebar would otherwise do: leave room for the traffic lights and
//! let the window be dragged.

use gpui::{Context, InteractiveElement as _, IntoElement, MouseButton, Styled as _, Window, px};
use gpui_component::{InteractiveElementExt as _, h_flex};

use crate::app::Pedro;
use crate::palette;

/// Tall enough to clear the macOS traffic lights.
const HEIGHT: f32 = 34.;

impl Pedro {
    pub(crate) fn render_title_strip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        h_flex()
            .id("title-strip")
            .h(px(HEIGHT))
            .w_full()
            .flex_shrink_0()
            .bg(palette::chrome())
            .border_b_1()
            .border_color(palette::border())
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
