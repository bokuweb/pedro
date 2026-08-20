//! An open book: the pages that have been rasterised, and what the reader has
//! marked on them.
//!
//! Pages are rasterised by pdfium on a background thread and handed to GPUI as
//! images. The bytes cross that boundary untouched: pdfium renders BGRA, and a
//! [`RenderImage`] is BGRA, so the one format conversion this would obviously
//! need does not exist.
//!
//! Only the pages near the one being read are kept. A thousand-page book at two
//! pixels per point is several gigabytes; what the reader can see is a dozen
//! megabytes, and the rest is a rasterise away.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{RenderImage, SharedString};
use image::{Frame, RgbaImage};
use pedro_core::model::Highlight;
use pedro_pdf::{Document, PageImage, PageSize, PageText, PixelFormat, Rect};

/// How much detail to render per logical pixel.
///
/// Fixed at 2 rather than read from the display: a page rendered for a Retina
/// screen is merely oversampled on a screen that is not one, while the reverse
/// is a blurry page, and a book is a thing people look at closely.
const OVERSAMPLE: f32 = 2.0;

/// How many pages to keep rasterised on each side of the one being read.
///
/// Enough to cover a screen of a scrolling reader in both directions, so that
/// scrolling back a page never waits for pdfium.
const KEEP: u32 = 4;

/// A page that has been rasterised, and the text that is on it.
pub struct Page {
    pub image: Arc<RenderImage>,
    /// The page's own size in points. Pages of one book are not all the same
    /// shape, and a page drawn in another page's proportions is a page whose
    /// character boxes no longer land on its words.
    pub size: PageSize,
    /// Every character and where it sits, which is what turns a drag into a
    /// passage.
    pub text: PageText,
}

/// A run of characters the reader has dragged across.
///
/// Stored as indices rather than as the text itself so that the highlight and
/// the quotation cannot disagree: both are read back out of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub page: u32,
    pub from: usize,
    pub to: usize,
}

/// A book open in a tab.
pub struct OpenDocument {
    /// Shared with the background thread that rasterises pages.
    pub document: Arc<Document>,
    pub page_count: u32,
    /// The size of the first page, in points, which every page is laid out
    /// against. A book whose pages differ in size is rare enough that measuring
    /// each one would cost more than it is worth.
    pub size: PageSize,
    /// The page at the top of the viewport, one-based, the way the reader
    /// counts. What the composer quotes and where the place is saved.
    pub page: u32,
    /// The pages that have been rasterised, by page number.
    pub pages: HashMap<u32, Page>,
    /// The pages pdfium is working on, so the same one is not asked for twice.
    pub requested: HashSet<u32>,
    pub selection: Option<Selection>,
    /// Every passage marked in this book, so the ones on a page can be drawn and
    /// the conversation behind one can be reopened by pressing it.
    pub highlights: Vec<Highlight>,
}

impl OpenDocument {
    pub fn new(document: Document, size: PageSize, page: u32) -> Self {
        let page_count = document.page_count();

        Self {
            document: Arc::new(document),
            page_count,
            size,
            page: page.clamp(1, page_count.max(1)),
            pages: HashMap::new(),
            requested: HashSet::new(),
            selection: None,
            highlights: Vec::new(),
        }
    }

    pub fn page(&self, page: u32) -> Option<&Page> {
        self.pages.get(&page)
    }

    /// Whether `page` has to be rasterised before it can be drawn.
    pub fn wants(&self, page: u32) -> bool {
        page >= 1
            && page <= self.page_count
            && !self.pages.contains_key(&page)
            && !self.requested.contains(&page)
    }

    /// Files a rasterised page, and forgets the ones the reader has left far
    /// enough behind.
    pub fn store(&mut self, page: u32, rasterised: Page) {
        self.requested.remove(&page);
        self.pages.insert(page, rasterised);

        let here = self.page;
        self.pages.retain(|number, _| number.abs_diff(here) <= KEEP);
    }

    /// How wide `page` is when drawn `height` logical pixels tall: its own
    /// shape once it has been read, the first page's before that — which is
    /// what the rows are sized by before anything has been read at all.
    pub fn width_of(&self, page: u32, height: f32) -> f32 {
        match self.page(page) {
            Some(held) => width_at(held.size, height),
            None => width_at(self.size, height),
        }
    }

    /// The scale to rasterise at so a page drawn `height` pixels tall has a
    /// pixel of its own for every pixel of screen.
    pub fn scale_for(&self, height: f32) -> f32 {
        scale_for(self.size, height)
    }

    /// Moves the page in view by `by`, stopping at either cover. Returns the
    /// page that lands on, or `None` if it did not move.
    pub fn turn(&mut self, by: i64) -> Option<u32> {
        let page = turned(self.page, self.page_count, by);
        let moved = page != self.page;
        self.page = page;

        moved.then_some(page)
    }

    /// What the composer's context line says about where we are.
    pub fn position(&self) -> SharedString {
        format!("p. {} of {}", self.page, self.page_count).into()
    }

    /// Starts a selection on `page`, at the character nearest `(x, y)`.
    pub fn begin_selection(&mut self, page: u32, x: f32, y: f32) {
        let Some(index) = self.page(page).and_then(|held| held.text.char_near(x, y)) else {
            self.selection = None;
            return;
        };

        self.selection = Some(Selection {
            page,
            from: index,
            to: index,
        });
    }

    /// Drags the far end of the selection to `(x, y)` on `page`.
    ///
    /// A drag that leaves the page it started on is ignored rather than
    /// restarted: passages that span a page break are not selectable yet, and
    /// silently moving the selection to another page would lose the one the
    /// reader was making.
    pub fn extend_selection(&mut self, page: u32, x: f32, y: f32) {
        let Some(selection) = self.selection else {
            return;
        };
        if selection.page != page {
            return;
        }
        let Some(index) = self.page(page).and_then(|held| held.text.char_near(x, y)) else {
            return;
        };

        self.selection = Some(Selection {
            to: index,
            ..selection
        });
    }

    /// The selection, if it covers anything at all.
    ///
    /// A selection of one character is a click, not a passage: dropping it is
    /// what makes pressing the page clear the last one.
    pub fn selection(&self) -> Option<Selection> {
        self.selection
            .filter(|selection| selection.from != selection.to)
    }

    /// The passage the reader has selected.
    pub fn selected_text(&self) -> Option<String> {
        let selection = self.selection()?;
        let text = self
            .page(selection.page)?
            .text
            .slice(selection.from, selection.to);

        (!text.trim().is_empty()).then_some(text)
    }

    /// One rectangle per line of the selection, if it is on `page`.
    pub fn selection_rects(&self, page: u32) -> Vec<Rect> {
        match self.selection().filter(|selection| selection.page == page) {
            Some(selection) => match self.page(page) {
                Some(held) => held.text.line_rects(selection.from, selection.to),
                None => Vec::new(),
            },
            None => Vec::new(),
        }
    }

    /// The passages marked on `page`.
    pub fn highlights_on(&self, page: u32) -> impl DoubleEndedIterator<Item = &Highlight> {
        self.highlights
            .iter()
            .filter(move |highlight| highlight.page_number == page)
    }

    /// The marked passage under `(x, y)` on `page`.
    ///
    /// The most recent wins where two overlap: a reader who marks the same
    /// sentence twice means the question they asked about it last.
    pub fn highlight_at(&self, page: u32, x: f32, y: f32) -> Option<&Highlight> {
        self.highlights_on(page)
            .filter(|highlight| highlight.rects.iter().any(|rect| rect.contains(x, y)))
            .next_back()
    }
}

/// How wide a page of `size` is when drawn `height` logical pixels tall.
fn width_at(size: PageSize, height: f32) -> f32 {
    match size.height {
        0.0 => height,
        page_height => height * size.width / page_height,
    }
}

/// The scale a page of `size` has to be rasterised at to fill `height` logical
/// pixels with real detail.
fn scale_for(size: PageSize, height: f32) -> f32 {
    match size.height {
        0.0 => OVERSAMPLE,
        page_height => (height * OVERSAMPLE / page_height).max(0.01),
    }
}

/// The page `by` pages from `page`, stopping at either cover.
fn turned(page: u32, page_count: u32, by: i64) -> u32 {
    (page as i64 + by).clamp(1, page_count.max(1) as i64) as u32
}

/// Wraps a rasterised page for GPUI.
///
/// The buffer is an `RgbaImage` holding BGRA, which is not a mistake: it is the
/// container `RenderImage` is built from, and what it wants inside it is BGRA.
/// Taking the bytes by value means the pixels are never copied.
pub fn as_render_image(page: PageImage) -> Option<Arc<RenderImage>> {
    debug_assert_eq!(
        page.format,
        PixelFormat::Bgra8,
        "gpui draws BGRA; anything else would need converting first"
    );

    let buffer = RgbaImage::from_raw(page.width, page.height, page.bytes)?;

    Some(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A4-ish, in points.
    fn a4() -> PageSize {
        PageSize {
            width: 595.0,
            height: 842.0,
        }
    }

    #[test]
    fn a_page_keeps_its_aspect_ratio() {
        let width = width_at(a4(), 842.0);
        assert!((width - 595.0).abs() < 1e-3, "{width}");
    }

    #[test]
    fn a_page_is_rasterised_at_two_pixels_per_pixel() {
        assert!((scale_for(a4(), 842.0) - 2.0).abs() < 1e-6);
        assert!((scale_for(a4(), 421.0) - 1.0).abs() < 1e-6);
    }

    /// A page with no height cannot be scaled to one, and must not divide by it
    /// either: a malformed PDF should still open.
    #[test]
    fn a_page_without_height_is_not_divided_by_zero() {
        let flat = PageSize {
            width: 100.0,
            height: 0.0,
        };

        assert_eq!(width_at(flat, 640.0), 640.0);
        assert_eq!(scale_for(flat, 640.0), OVERSAMPLE);
    }

    #[test]
    fn turning_stops_at_both_covers() {
        assert_eq!(turned(1, 10, -1), 1);
        assert_eq!(turned(10, 10, 1), 10);
        assert_eq!(turned(5, 10, 3), 8);
        assert_eq!(turned(5, 10, -3), 2);
    }

    /// A book that reports no pages still has a page one to sit on.
    #[test]
    fn an_empty_book_stays_on_page_one() {
        assert_eq!(turned(1, 0, 1), 1);
    }

    #[test]
    fn a_render_image_needs_four_bytes_per_pixel() {
        let short = PageImage {
            width: 2,
            height: 2,
            format: PixelFormat::Bgra8,
            bytes: vec![0; 8],
        };

        assert!(as_render_image(short).is_none());
    }

    #[test]
    fn a_page_becomes_an_image_of_the_same_size() {
        let page = PageImage {
            width: 3,
            height: 2,
            format: PixelFormat::Bgra8,
            bytes: vec![0; 3 * 2 * 4],
        };

        let image = as_render_image(page).expect("a full buffer");
        let size = image.size(0);
        assert_eq!((u32::from(size.width), u32::from(size.height)), (3, 2));
    }
}
