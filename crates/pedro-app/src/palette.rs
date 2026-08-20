//! The colors the interface is built from.
//!
//! One violet family, dark, with the panels translucent so the desktop behind
//! the window shows through a blur (see `main.rs`). That is why the surfaces
//! carry alpha rather than being flat: a fully opaque panel over a blurred
//! window looks like a mistake, not a choice.
//!
//! Everything lives here so the whole surface can be re-tuned without hunting
//! through the view code. [`apply_to_theme`] pushes it into `gpui-component` so
//! its widgets (inputs, buttons, scrollbars) match the hand-rolled parts.

use gpui::{App, Hsla, px, rgb};
use gpui_component::{Theme, ThemeMode};

/// A colour with the window's blur allowed through it.
fn veiled(hex: u32, alpha: f32) -> Hsla {
    Hsla::from(rgb(hex)).opacity(alpha)
}

/// Behind everything, and the reader canvas.
pub fn canvas() -> Hsla {
    veiled(0x1d1229, 0.72)
}

/// The title strip, top bar and tab bar.
pub fn chrome() -> Hsla {
    veiled(0x180f22, 0.66)
}

/// The narrow icon column on the far left.
pub fn rail() -> Hsla {
    veiled(0x140c1d, 0.7)
}

/// The panel between the rail and the reader.
pub fn sidebar() -> Hsla {
    veiled(0x1a1024, 0.66)
}

/// A row in the sidebar list.
///
/// Transparent: rows are told apart by the space between them, not by a fill,
/// so only the one under the pointer and the one you are on carry any.
pub fn row() -> Hsla {
    gpui::transparent_black()
}

pub fn row_hover() -> Hsla {
    veiled(0x7c5cc4, 0.14)
}

pub fn row_active() -> Hsla {
    veiled(0x8b6ad4, 0.22)
}

/// Raised controls: the search field and the composer.
pub fn surface() -> Hsla {
    veiled(0x2a1b3a, 0.66)
}

pub fn surface_hover() -> Hsla {
    veiled(0x362344, 0.8)
}

/// Hairlines between rows and panels.
pub fn separator() -> Hsla {
    veiled(0x5a3f78, 0.22)
}

pub fn border() -> Hsla {
    veiled(0x5a3f78, 0.28)
}

pub fn text() -> Hsla {
    rgb(0xf2ecf8).into()
}

pub fn text_muted() -> Hsla {
    rgb(0xa695bb).into()
}

pub fn text_faint() -> Hsla {
    rgb(0x7c6c93).into()
}

/// The brand violet, used for the logo mark and focus rings.
pub fn accent() -> Hsla {
    rgb(0x8b5cf6).into()
}

/// Inline code and other places the text should read as machinery.
pub fn code() -> Hsla {
    rgb(0xc9a4ff).into()
}

/// Something is happening: the dot beside a row that is still working.
pub fn working() -> Hsla {
    rgb(0xe879c0).into()
}

pub fn danger() -> Hsla {
    rgb(0xf07a7a).into()
}

pub fn success() -> Hsla {
    rgb(0x63d6a4).into()
}

/// Ink for the few places something light sits under something dark: the
/// arrow on the send button, text on a page. Opaque, unlike the panels — a
/// translucent glyph on white reads as a rendering fault.
pub fn ink() -> Hsla {
    rgb(0x1d1229).into()
}

/// The paper of a rendered page. Stays light: a PDF page is white, and the
/// point of the reader is to show it as it is.
pub fn page() -> Hsla {
    rgb(0xf7f7f5).into()
}

/// Placeholder bars drawn on a page we cannot render yet.
pub fn page_placeholder() -> Hsla {
    rgb(0xdcdcd8).into()
}

/// Points `gpui-component`'s theme at the palette above.
pub fn apply_to_theme(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);

    let theme = Theme::global_mut(cx);
    theme.font_size = px(15.);
    theme.radius = px(10.);
    theme.radius_lg = px(16.);

    theme.background = canvas();
    theme.foreground = text();
    theme.muted = surface();
    theme.muted_foreground = text_muted();
    theme.border = border();
    theme.input = surface();
    theme.ring = accent();
    theme.selection = accent().opacity(0.35);
    theme.caret = code();

    theme.primary = accent();
    theme.primary_foreground = text();
    theme.primary_hover = accent().opacity(0.85);
    theme.primary_active = accent().opacity(0.7);

    theme.secondary = surface();
    theme.secondary_foreground = text();
    theme.secondary_hover = surface_hover();
    theme.secondary_active = surface_hover();

    theme.popover = veiled(0x241733, 0.96);
    theme.popover_foreground = text();

    theme.sidebar = sidebar();
    theme.sidebar_foreground = text();
    theme.sidebar_border = border();
    theme.sidebar_accent = row_active();
    theme.sidebar_accent_foreground = text();

    theme.tab_bar = chrome();
    theme.tab = chrome();
    theme.tab_foreground = text_muted();
    theme.tab_active = canvas();
    theme.tab_active_foreground = text();

    theme.title_bar = chrome();
    theme.title_bar_border = border();

    theme.scrollbar = gpui::transparent_black();
    theme.scrollbar_thumb = row_active();
    theme.scrollbar_thumb_hover = surface_hover();

    theme.danger = danger();
    theme.success = success();
}
