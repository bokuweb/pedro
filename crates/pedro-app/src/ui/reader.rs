//! The document canvas: every page of the book, in one scroll.
//!
//! A thousand-page book cannot be rasterised, so the list only builds the rows
//! it is about to draw and only those pages are asked of pdfium. Every page is
//! laid out against the first one's size, which is what makes the rows a uniform
//! height and the scrollbar honest before a single page has been read.
//!
//! Each page records where it was drawn, because a drag arrives in window
//! coordinates and characters are known as fractions of a page. It is recorded
//! into a shared cell rather than into the view because this happens while the
//! frame is being laid out, and asking the view to change then would ask for
//! another frame, every frame.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement as _, RenderImage, SharedString, Styled as _, canvas, div, img,
    px, relative, uniform_list,
};
use gpui_component::{IconName, h_flex, v_flex};
use pedro_pdf::Rect;

use crate::app::{PAGE_HEIGHT, Pedro};
use crate::palette;
use crate::ui::icon;

/// The space between one page and the next.
const GAP: f32 = 20.;

impl Pedro {
    pub(crate) fn render_reader(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let Some(tab) = self.active_tab() else {
            return render_empty_state().into_any_element();
        };

        if let Some(why) = &tab.error {
            return render_failure(tab.label.clone(), why.clone()).into_any_element();
        }

        let Some(open) = &tab.document else {
            return render_opening().into_any_element();
        };

        let page_count = open.page_count as usize;

        uniform_list(
            "pages",
            page_count,
            cx.processor(|this, range: Range<usize>, _window, cx| {
                // The list asks for exactly the rows it is about to draw, which
                // makes this the one place that knows what to rasterise.
                this.pages_in_view(&range, cx);

                range
                    .map(|index| this.render_page_row(index as u32 + 1, cx))
                    .collect()
            }),
        )
        .track_scroll(self.page_scroll.clone())
        .size_full()
        .into_any_element()
    }

    /// One page, centred in a row of its own.
    fn render_page_row(&self, page: u32, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let Some(open) = self.open_document() else {
            return div().h(px(PAGE_HEIGHT + GAP)).into_any_element();
        };

        let width = open.width_at(PAGE_HEIGHT);
        let marks: Vec<Rect> = open
            .highlights_on(page)
            .flat_map(|highlight| highlight.rects.iter().copied())
            .collect();

        let sheet = render_sheet(
            width,
            open.page(page).map(|held| held.image.clone()),
            marks,
            open.selection_rects(page),
        );

        h_flex()
            .h(px(PAGE_HEIGHT + GAP))
            .w_full()
            .justify_center()
            .items_start()
            .child(self.selectable(page, sheet.into_any_element(), cx))
            .into_any_element()
    }

    /// Makes a page answer a drag with a passage.
    fn selectable(
        &self,
        page: u32,
        sheet: AnyElement,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let bounds = self.page_bounds.clone();

        div()
            .id(("page", page as usize))
            .relative()
            .cursor_text()
            .child(sheet)
            .child(
                canvas(
                    move |drawn, _, _| {
                        bounds.borrow_mut().insert(page, drawn);
                    },
                    |_, _, _, _| {},
                )
                // Without a size of its own a canvas is laid out as nothing,
                // and the bounds it reports are nothing too — which is what
                // made every drag fall outside the page.
                .absolute()
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.begin_selection(page, event.position, cx)
                }),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                this.extend_selection(page, event.position, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_selection(cx)),
            )
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
fn render_over_page(rect: Rect, tint: Hsla) -> impl IntoElement {
    div()
        .absolute()
        .left(relative(rect.left))
        .top(relative(rect.top))
        .w(relative(rect.width().max(0.0)))
        .h(relative(rect.height().max(0.0)))
        .rounded(px(2.))
        .bg(tint)
}

/// A book still being read off disk.
///
/// An empty sheet rather than a spinner: the first page lands in a space of the
/// right shape instead of pushing the layout around when it arrives.
fn render_opening() -> impl IntoElement {
    h_flex()
        .size_full()
        .justify_center()
        .items_start()
        .pt(px(GAP))
        .child(render_sheet(
            PAGE_HEIGHT * 0.75,
            None,
            Vec::new(),
            Vec::new(),
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
