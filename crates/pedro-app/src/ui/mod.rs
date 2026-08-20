//! The visual shell, split by region. Each module adds an `impl Pedro` block
//! with one `render_*` method.

mod chat;
mod composer;
mod reader;
mod sidebar;
mod tab_bar;
mod window_drag;

use gpui::prelude::FluentBuilder as _;
use gpui::{Div, Hsla, InteractiveElement as _, Pixels, Styled as _, div, px};
use gpui_component::{Icon, IconName};

use crate::palette;

/// An icon at an explicit pixel size, which reads better than the theme's
/// size scale for a layout this dense.
pub(crate) fn icon(name: IconName, size: Pixels, color: Hsla) -> Icon {
    Icon::new(name).size(size).text_color(color)
}

/// A one pixel vertical rule, for separating groups inside a horizontal bar.
pub(crate) fn vertical_rule(height: Pixels) -> Div {
    div().w(px(1.)).h(height).bg(palette::border())
}

/// The square hit target shared by the rail and the top bar toggles. The
/// caller supplies the id, click handler and icon.
pub(crate) fn square_button(size: Pixels, active: bool) -> Div {
    div()
        .size(size)
        .rounded(px(9.))
        .flex()
        .items_center()
        .justify_center()
        .when(active, |this| this.bg(palette::row_active()))
        .hover(|this| this.bg(palette::row_hover()))
}
