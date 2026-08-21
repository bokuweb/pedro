//! The visual shell, split by region. Each module adds an `impl Pedro` block
//! with one `render_*` method.

mod chat;
mod composer;
mod reader;
mod sidebar;
mod tab_bar;
mod window_drag;

pub(crate) use chat::CHAT_WIDTH;
pub(crate) use sidebar::SIDEBAR_WIDTH;

use gpui::{Hsla, Pixels, Styled as _};
use gpui_component::{Icon, IconName};

/// An icon at an explicit pixel size, which reads better than the theme's
/// size scale for a layout this dense.
pub(crate) fn icon(name: IconName, size: Pixels, color: Hsla) -> Icon {
    Icon::new(name).size(size).text_color(color)
}
