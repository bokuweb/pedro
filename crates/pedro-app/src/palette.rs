//! The colors the interface is built from.
//!
//! These are kept in one place so the whole surface can be re-tuned without
//! hunting through the view code. [`apply_to_theme`] pushes them into
//! `gpui-component` so its widgets (inputs, buttons, scrollbars) match the
//! hand-rolled parts of the shell.

use gpui::{App, Hsla, px, rgb};
use gpui_component::{Theme, ThemeMode};

/// Behind everything, and the reader canvas.
pub fn canvas() -> Hsla {
    rgb(0x050506).into()
}

/// The title strip, top bar and tab bar.
pub fn chrome() -> Hsla {
    rgb(0x0e0e11).into()
}

/// The narrow icon column on the far left.
pub fn rail() -> Hsla {
    rgb(0x141417).into()
}

/// The panel between the rail and the reader.
pub fn sidebar() -> Hsla {
    rgb(0x1a1a1e).into()
}

/// The header block at the top of the sidebar panel.
pub fn sidebar_header() -> Hsla {
    rgb(0x1e1e23).into()
}

/// A row in the sidebar list.
pub fn row() -> Hsla {
    rgb(0x232328).into()
}

pub fn row_hover() -> Hsla {
    rgb(0x2b2b31).into()
}

pub fn row_active() -> Hsla {
    rgb(0x313139).into()
}

/// Raised controls: the search field and the status pill.
pub fn surface() -> Hsla {
    rgb(0x2c2c32).into()
}

pub fn surface_hover() -> Hsla {
    rgb(0x35353c).into()
}

/// Hairlines between rows and panels.
pub fn separator() -> Hsla {
    rgb(0x2e2e34).into()
}

pub fn border() -> Hsla {
    rgb(0x26262b).into()
}

pub fn text() -> Hsla {
    rgb(0xececef).into()
}

pub fn text_muted() -> Hsla {
    rgb(0x8e8e97).into()
}

pub fn text_faint() -> Hsla {
    rgb(0x5e5e66).into()
}

/// The brand blue, used for the logo mark and focus rings.
pub fn accent() -> Hsla {
    rgb(0x4b4be6).into()
}

pub fn danger() -> Hsla {
    rgb(0xe05252).into()
}

pub fn success() -> Hsla {
    rgb(0x4cc38a).into()
}

/// The paper of a rendered page.
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
    theme.radius = px(8.);
    theme.radius_lg = px(12.);

    theme.background = canvas();
    theme.foreground = text();
    theme.muted = surface();
    theme.muted_foreground = text_muted();
    theme.border = border();
    theme.input = surface();
    theme.ring = accent();
    theme.selection = accent().opacity(0.35);
    theme.caret = text();

    theme.primary = accent();
    theme.primary_foreground = text();
    theme.primary_hover = accent().opacity(0.85);
    theme.primary_active = accent().opacity(0.7);

    theme.secondary = surface();
    theme.secondary_foreground = text();
    theme.secondary_hover = surface_hover();
    theme.secondary_active = surface_hover();

    theme.popover = sidebar_header();
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
    theme.scrollbar_thumb = surface_hover();
    theme.scrollbar_thumb_hover = text_faint();

    theme.danger = danger();
    theme.success = success();
}
