//! The document canvas.
//!
//! Page rendering is not wired up yet — pedro has not committed to a PDF
//! backend — so pages are drawn as sheets with placeholder text blocks. The
//! layout, spread toggle and empty state are real.

use gpui::{Context, IntoElement, ParentElement as _, Styled as _, div, px, relative};
use gpui_component::{IconName, h_flex, v_flex};

use crate::app::Pedro;
use crate::palette;
use crate::state::PageLayout;
use crate::ui::icon;

const PAGE_WIDTH: f32 = 420.;
const PAGE_HEIGHT: f32 = 560.;

/// Relative widths of the placeholder lines on a page, so the block reads as
/// prose rather than a uniform grid.
const LINE_WIDTHS: [f32; 12] = [
    0.95, 0.88, 0.92, 0.7, 0.94, 0.86, 0.9, 0.62, 0.93, 0.89, 0.84, 0.45,
];

impl Pedro {
    pub(crate) fn render_reader(&self, _cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let Some(tab) = self.active_tab() else {
            return render_empty_state().into_any_element();
        };

        let pages = match self.layout {
            PageLayout::Single => 1,
            PageLayout::Spread => 2,
        };

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(px(18.))
            .child(
                h_flex()
                    .gap(px(20.))
                    .children((0..pages).map(|offset| render_page(offset + 1))),
            )
            .child(
                v_flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(palette::text_muted())
                            .child(tab.label.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette::text_faint())
                            .child("Page rendering backend not connected yet"),
                    ),
            )
            .into_any_element()
    }
}

fn render_page(number: usize) -> impl IntoElement {
    v_flex()
        .w(px(PAGE_WIDTH))
        .h(px(PAGE_HEIGHT))
        .p(px(38.))
        .gap(px(13.))
        .rounded(px(4.))
        .bg(palette::page())
        .shadow_lg()
        .children(LINE_WIDTHS.iter().map(|width| {
            div()
                .h(px(9.))
                .w(relative(*width))
                .rounded(px(3.))
                .bg(palette::page_placeholder())
        }))
        .child(div().flex_1())
        .child(
            div()
                .w_full()
                .text_center()
                .text_size(px(11.))
                .text_color(palette::text_faint())
                .child(format!("{number}")),
        )
}

fn render_empty_state() -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .child(icon(IconName::BookOpen, px(28.), palette::text_faint()))
        .child(
            div()
                .text_size(px(14.))
                .text_color(palette::text_muted())
                .child("No document open"),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(palette::text_faint())
                .child("Pick something from the library on the left."),
        )
}
