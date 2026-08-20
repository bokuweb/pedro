//! The document canvas.
//!
//! A page is a rasterised image of a real PDF page, drawn at its own aspect
//! ratio on white paper. Until it arrives — a book is read off disk and its
//! first page rasterised in the background — the same sheet is drawn empty,
//! which keeps the layout from jumping when the page lands in it.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    RenderImage, SharedString, StatefulInteractiveElement as _, Styled as _, canvas, div, img, px,
    relative,
};
use gpui_component::{IconName, h_flex, v_flex};

use pedro_pdf::Rect;

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
            return render_sheet(PAGE_HEIGHT * 0.75, None, Vec::new(), Vec::new())
                .into_any_element();
        };

        let width = open.width_at(PAGE_HEIGHT);
        let spread = matches!(self.layout, PageLayout::Spread) && open.page_count > 1;
        // Marked passages sit under the live selection: the reader is dragging
        // out one of them right now, and what is under the pointer has to be
        // the brighter of the two.
        let marks: Vec<Rect> = open
            .highlights_here()
            .flat_map(|highlight| highlight.rects.iter().copied())
            .collect();
        let page = render_sheet(
            width,
            open.visible().cloned(),
            marks,
            open.selection_rects(),
        )
        .into_any_element();

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(px(16.))
            .child(
                h_flex()
                    .gap(px(18.))
                    .child(self.selectable(page, cx))
                    // The facing page is drawn as an empty sheet until spreads
                    // rasterise two pages rather than one: an empty sheet is at
                    // least honest about the shape of what is coming.
                    .when(spread, |this| {
                        this.child(render_sheet(width, None, Vec::new(), Vec::new()))
                    }),
            )
            .child(self.render_page_controls(open.page, open.page_count, cx))
            .into_any_element()
    }

    /// Makes a page answer a drag with a passage.
    ///
    /// The page also reports where it was drawn, because a drag arrives in
    /// window coordinates and the characters are known as fractions of the
    /// page; a `canvas` is the only element that is told its own bounds.
    fn selectable(&self, page: AnyElement, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let this = cx.entity();

        div()
            .id("page")
            .relative()
            .cursor_text()
            .child(page)
            .child(canvas(
                move |bounds, _, cx| {
                    this.update(cx, |this, _| this.page_drawn_at(bounds));
                },
                |_, _, _, _| {},
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    this.begin_selection(event.position, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                this.extend_selection(event.position, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_selection(cx)),
            )
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

/// One sheet of paper, with a page on it or without, and whatever of it the
/// reader has marked.
fn render_sheet(
    width: f32,
    image: Option<Arc<RenderImage>>,
    marks: Vec<Rect>,
    selection: Vec<Rect>,
) -> impl IntoElement {
    div()
        .relative()
        .w(px(width))
        .h(px(PAGE_HEIGHT))
        .rounded(px(3.))
        .bg(palette::page())
        .shadow_lg()
        .overflow_hidden()
        .children(image.map(|image| img(image).w(px(width)).h(px(PAGE_HEIGHT))))
        .children(
            marks
                .into_iter()
                .map(|rect| render_over_page(rect, palette::working().opacity(0.26))),
        )
        .children(
            selection
                .into_iter()
                .map(|rect| render_over_page(rect, palette::accent().opacity(0.34))),
        )
}

/// One line of a mark, drawn over the page.
///
/// Placed in fractions of the sheet rather than in pixels, which is the same
/// space the character boxes are measured in — so a mark stays on its words at
/// any size the page is drawn.
fn render_over_page(rect: Rect, tint: gpui::Hsla) -> impl IntoElement {
    div()
        .absolute()
        .left(relative(rect.left))
        .top(relative(rect.top))
        .w(relative(rect.width().max(0.0)))
        .h(relative(rect.height().max(0.0)))
        .rounded(px(2.))
        .bg(tint)
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
