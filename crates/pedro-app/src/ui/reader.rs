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

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement as _, RenderImage, SharedString, StatefulInteractiveElement as _,
    Styled as _, canvas, div, img, px, relative, uniform_list,
};
use gpui_component::input::Input;
use gpui_component::{IconName, Sizable as _, h_flex, spinner::Spinner, v_flex};
use pedro_pdf::Rect;

use crate::app::Pedro;
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

        // A shelf has no pages of its own. What it has is the books on it, and
        // this is where the reader sees which ones a question will be answered
        // from — the same place a book shows them its pages.
        if tab.id.starts_with("shelf:") {
            return self.render_shelf(cx).into_any_element();
        }

        let Some(open) = &tab.document else {
            return render_opening().into_any_element();
        };

        let page_count = open.page_count as usize;
        let Some(scroll) = self.page_scroll().cloned() else {
            return render_empty_state().into_any_element();
        };

        uniform_list(
            // Keyed by the book: gpui keeps an element's state under its id,
            // and two books sharing one would share a scroll position that
            // means a different place in each.
            SharedString::from(format!("pages:{}", tab.id)),
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
        .track_scroll(scroll)
        .size_full()
        .into_any_element()
    }

    /// A shelf: its name, and the books a question to it is answered from.
    fn render_shelf(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let Some(shelf) = self.active_shelf().cloned() else {
            return render_empty_state().into_any_element();
        };

        let books: Vec<_> = self
            .library
            .books()
            .iter()
            .filter(|book| book.folder_id.as_deref() == Some(shelf.id.as_str()))
            .map(|book| self.render_shelf_book(book, &shelf.id, cx))
            .collect();

        let empty = books.is_empty();
        let id = shelf.id.clone();

        v_flex()
            .size_full()
            .overflow_hidden()
            .child(
                h_flex()
                    .px(px(28.))
                    .pt(px(24.))
                    .pb(px(12.))
                    .gap(px(10.))
                    .items_center()
                    .child(icon(IconName::Folder, px(18.), palette::text_muted()))
                    .child(div().flex_1().min_w_0().child(Input::new(&self.shelf_name)))
                    .child(self.render_shelf_delete(&id, cx)),
            )
            .child(
                div()
                    .px(px(28.))
                    .pb(px(14.))
                    .text_size(px(12.))
                    .text_color(palette::text_faint())
                    .child(match empty {
                        true => SharedString::from(
                            "Drag books here from the sidebar, or right-click one to put it on                              this shelf.",
                        ),
                        false => SharedString::from(format!(
                            "{} book{} — a question in the panel is answered from all of them.",
                            books.len(),
                            if books.len() == 1 { "" } else { "s" }
                        )),
                    }),
            )
            .child(
                v_flex()
                    .id("shelf-books")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(20.))
                    .pb(px(20.))
                    .gap(px(6.))
                    .children(books),
            )
            .into_any_element()
    }

    /// One book on a shelf, with the way to take it off again.
    fn render_shelf_book(
        &self,
        book: &pedro_core::model::Book,
        shelf_id: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let book_id = book.id.clone();
        let open_id = book.id.clone();
        let label = SharedString::from(book.file_name.clone());
        let pages = SharedString::from(format!("{} pages", book.page_count));
        let _ = shelf_id;

        h_flex()
            .id(SharedString::from(format!("shelf-book:{}", book.id)))
            .group("shelf-book")
            .h(px(52.))
            .px(px(14.))
            .gap(px(12.))
            .items_center()
            .rounded(px(10.))
            .bg(palette::row())
            .cursor_pointer()
            .hover(|this| this.bg(palette::row_hover()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_book_tab(&open_id, cx);
            }))
            .child(icon(IconName::BookOpen, px(15.), palette::text_faint()))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(palette::text())
                            .child(label),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(palette::text_faint())
                            .child(pages),
                    ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("unshelve:{}", book.id)))
                    .px(px(8.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .invisible()
                    .group_hover("shelf-book", |this| this.visible())
                    .bg(palette::surface())
                    .hover(|this| this.bg(palette::danger().opacity(0.3)))
                    .text_size(px(10.))
                    .text_color(palette::text_muted())
                    .child("Take off")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.shelve_book(&book_id, None, cx);
                    })),
            )
    }

    /// Throws the shelf away, asking first. The books stay.
    fn render_shelf_delete(
        &self,
        shelf_id: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let id = SharedString::from(format!("shelf:{shelf_id}"));
        let confirming = self.is_confirming(&id);
        let shelf_id = shelf_id.to_owned();
        let asked = id.clone();

        div()
            .id("delete-shelf")
            .px(px(10.))
            .h(px(26.))
            .flex()
            .items_center()
            .rounded(px(7.))
            .cursor_pointer()
            .when(confirming, |this| this.bg(palette::danger().opacity(0.24)))
            .hover(|this| this.bg(palette::danger().opacity(0.36)))
            .text_size(px(11.))
            .text_color(if confirming {
                palette::danger()
            } else {
                palette::text_muted()
            })
            .child(if confirming {
                "Delete the shelf? The books stay."
            } else {
                "Delete shelf"
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                match this.is_confirming(&asked) {
                    true => this.delete_shelf(&shelf_id, cx),
                    false => this.confirm(asked.clone(), cx),
                }
            }))
    }

    /// One page, centred in a row of its own.
    fn render_page_row(&self, page: u32, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let height = self.page_height();
        let Some(open) = self.open_document() else {
            return div().h(px(height + GAP)).into_any_element();
        };

        // The page's own shape where it is known. Books whose pages differ in
        // size — a cover, a fold-out — would otherwise be stretched into the
        // first page's proportions, and a mark drawn in fractions of a
        // stretched page does not sit on its words.
        let width = open.width_of(page, height);
        let sheet = match open.page(page) {
            Some(held) => {
                let marks: Vec<Rect> = open
                    .highlights_on(page)
                    .flat_map(|highlight| highlight.rects.iter().copied())
                    .collect();

                render_sheet(
                    width,
                    height,
                    held.image.clone(),
                    marks,
                    open.selection_rects(page),
                )
                .into_any_element()
            }
            None => render_pending(width, height).into_any_element(),
        };

        h_flex()
            .h(px(height + GAP))
            .w_full()
            .justify_center()
            .items_start()
            .child(self.selectable(page, sheet, cx))
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
                    |_, _, _| {},
                    // Recorded when the page is painted rather than when it is
                    // laid out. A scrolling list lays its rows out in its own
                    // space and translates them on the way to the screen, so
                    // prepaint bounds are in neither the space the mouse is
                    // reported in nor the space the page is drawn in.
                    move |drawn, _, _, _| {
                        bounds.borrow_mut().insert(page, drawn);
                    },
                )
                // Without a size of its own a canvas is laid out as nothing,
                // and the bounds it reports are nothing too. Without a corner
                // to sit in it takes its static position — which, as the second
                // child of a block, is one whole page *below* the page, and
                // every drag then lands above the top of the sheet.
                .absolute()
                .top(px(0.))
                .left(px(0.))
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.begin_selection(page, event.position, cx)
                }),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                this.extend_selection(page, event.pressed_button, event.position, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_selection(cx)),
            )
    }
}

/// One sheet of paper, and whatever of it the reader has marked.
fn render_sheet(
    width: f32,
    height: f32,
    image: Arc<RenderImage>,
    marks: Vec<Rect>,
    selection: Vec<Rect>,
) -> impl IntoElement {
    div()
        .relative()
        .w(px(width))
        .h(px(height))
        .rounded(px(3.))
        .bg(palette::page())
        .shadow_lg()
        .overflow_hidden()
        .child(img(image).w(px(width)).h(px(height)))
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

/// A page whose image has not arrived yet.
///
/// It takes the row's space, so the list does not move when the page lands, but
/// draws no sheet: until a page has been rasterised its width is only the first
/// page's width, and paper of the wrong width is paper that visibly changes
/// shape when the real page arrives.
fn render_pending(width: f32, height: f32) -> impl IntoElement {
    h_flex()
        .w(px(width))
        .h(px(height))
        .items_center()
        .justify_center()
        .child(
            Spinner::new()
                .with_size(px(20.))
                .color(palette::text_faint()),
        )
}

/// A book still being read off disk.
///
/// A spinner rather than a blank sheet: the sheet would have to guess a page
/// size, and every guess is wrong for some book — a scan wider than it is tall,
/// a plan drawn at A0. A wrongly shaped sheet moves the layout further when the
/// first page arrives than no sheet at all does.
fn render_opening() -> impl IntoElement {
    h_flex().size_full().items_center().justify_center().child(
        Spinner::new()
            .with_size(px(22.))
            .color(palette::text_faint()),
    )
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
