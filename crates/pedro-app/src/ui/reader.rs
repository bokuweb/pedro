//! The document canvas.
//!
//! A page is a rasterised image of a real PDF page, drawn at its own aspect
//! ratio on white paper. Until it arrives — a book is read off disk and its
//! first page rasterised in the background — the same sheet is drawn empty,
//! which keeps the layout from jumping when the page lands in it.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, img, px,
};
use gpui_component::{IconName, h_flex, v_flex};

use crate::app::{PAGE_HEIGHT, Pedro};
use crate::palette;
use crate::state::PageLayout;
use crate::ui::icon;

impl Pedro {
    pub(crate) fn render_reader(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let Some(tab) = self.active_tab() else {
            return render_empty_state().into_any_element();
        };

        if let Some(why) = &tab.error {
            return render_failure(tab.label.clone(), why.clone()).into_any_element();
        }

        let Some(open) = &tab.document else {
            return render_sheet(PAGE_HEIGHT * 0.75, None).into_any_element();
        };

        let width = open.width_at(PAGE_HEIGHT);
        let spread = matches!(self.layout, PageLayout::Spread) && open.page_count > 1;

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(px(16.))
            .child(
                h_flex()
                    .gap(px(18.))
                    .child(render_sheet(width, open.visible().cloned()))
                    // The facing page is drawn as an empty sheet until spreads
                    // rasterise two pages rather than one: an empty sheet is at
                    // least honest about the shape of what is coming.
                    .when(spread, |this| this.child(render_sheet(width, None))),
            )
            .child(self.render_page_controls(open.page, open.page_count, cx))
            .into_any_element()
    }

    /// Where the reader is, and the two ways to move.
    fn render_page_controls(
        &self,
        page: u32,
        page_count: u32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        h_flex()
            .gap(px(4.))
            .items_center()
            .child(render_step(
                "previous-page",
                IconName::ChevronLeft,
                page > 1,
                cx.listener(|this, _, _, cx| this.turn_page(-1, cx)),
            ))
            .child(
                div()
                    .min_w(px(110.))
                    .text_center()
                    .text_size(px(12.))
                    .text_color(palette::text_muted())
                    .child(format!("{page} / {page_count}")),
            )
            .child(render_step(
                "next-page",
                IconName::ChevronRight,
                page < page_count,
                cx.listener(|this, _, _, cx| this.turn_page(1, cx)),
            ))
    }
}

/// One sheet of paper, with a page on it or without.
fn render_sheet(width: f32, image: Option<std::sync::Arc<gpui::RenderImage>>) -> impl IntoElement {
    div()
        .w(px(width))
        .h(px(PAGE_HEIGHT))
        .rounded(px(3.))
        .bg(palette::page())
        .shadow_lg()
        .overflow_hidden()
        .children(image.map(|image| img(image).w(px(width)).h(px(PAGE_HEIGHT))))
}

fn render_step(
    id: &'static str,
    name: IconName,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .size(px(28.))
        .rounded(px(8.))
        .flex()
        .items_center()
        .justify_center()
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(palette::row_hover()))
                .on_click(on_click)
        })
        .child(icon(
            name,
            px(15.),
            if enabled {
                palette::text_muted()
            } else {
                palette::text_faint()
            },
        ))
}

fn render_failure(title: SharedString, why: SharedString) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .child(icon(IconName::TriangleAlert, px(26.), palette::danger()))
        .child(
            div()
                .text_size(px(14.))
                .text_color(palette::text())
                .child(title),
        )
        .child(
            div()
                .max_w(px(420.))
                .text_center()
                .text_size(px(12.))
                .text_color(palette::text_muted())
                .child(why),
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
