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

use crate::spread::{Layout, Rows};

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

impl Page {
    /// How many characters of this page have a box of their own.
    pub fn chars_len(&self) -> usize {
        self.text.chars.len()
    }
}

/// A page that has been rasterised, and the text that is on it.
pub struct Page {
    pub image: Arc<RenderImage>,
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

/// The passage a jump was aimed at, drawn until the reader looks elsewhere.
///
/// A jump that only scrolls leaves the reader to find the sentence again on the
/// page it is on, which for a search hit is the search a second time. What is
/// known of the passage depends on where the jump came from: a mark carries the
/// rectangles it was drawn from, while a search hit carries only the words it
/// matched — and words become rectangles only once the page has been read.
#[derive(Debug, Clone)]
pub struct Spotlight {
    pub page: u32,
    /// The passage as text, for a target that arrived as words.
    pub needle: Option<String>,
    /// One rectangle per line of it, once they are known.
    pub rects: Vec<Rect>,
}

impl Spotlight {
    /// A page to land on with nothing on it to point at, which is all a jump
    /// knows when the passage it was made from has gone.
    pub fn page(page: u32) -> Self {
        Self {
            page,
            needle: None,
            rects: Vec::new(),
        }
    }

    /// A passage whose geometry is already known, the way a mark's is.
    pub fn marked(page: u32, rects: Vec<Rect>) -> Self {
        Self {
            page,
            needle: None,
            rects,
        }
    }

    /// A passage known only by its words, the way a search hit is.
    pub fn found(page: u32, needle: impl Into<String>) -> Self {
        Self {
            page,
            needle: Some(needle.into()),
            rects: Vec::new(),
        }
    }
}

/// A book open in a tab.
pub struct OpenDocument {
    /// Shared with the background thread that rasterises pages.
    pub document: Arc<Document>,
    pub page_count: u32,
    /// The size of the first page, in points, and what a page of unknown
    /// number is measured against.
    pub size: PageSize,
    /// The page at the top of the viewport, one-based, the way the reader
    /// counts. What the composer quotes and where the place is saved.
    pub page: u32,
    /// Whether the pages are drawn one at a time or two facing. A property of
    /// the book rather than of the reader — a scanned spread wants it and a
    /// slide deck does not — so it is stored and restored per book.
    pub layout: Layout,
    /// Every page's size, read in one pass when the book was opened.
    ///
    /// Held for the whole book rather than looked up as pages arrive: a row
    /// laid out around the pages in it moves every time one of them turns out
    /// to be a different shape from the first page.
    pub sizes: Vec<PageSize>,
    /// Which pages share a row, for the layout it was built for.
    pub rows: Rows,
    /// The pages that have been rasterised, by page number.
    pub pages: HashMap<u32, Page>,
    /// The pages pdfium is working on, so the same one is not asked for twice.
    pub requested: HashSet<u32>,
    /// The pages that came back unusable, so they are not asked for forever.
    ///
    /// Without this a page that cannot be drawn is asked for again on the very
    /// next frame, and the answer is no again — which is not a blank page, it
    /// is a core spinning for as long as the book is open.
    pub unusable: HashSet<u32>,
    /// Bumped whenever the size a page is drawn at changes. Work already in
    /// flight was started for the old size, and is dropped when it lands rather
    /// than being drawn at a size it was not made for.
    pub generation: u64,
    pub selection: Option<Selection>,
    /// Every passage marked in this book, so the ones on a page can be drawn and
    /// the conversation behind one can be reopened by pressing it.
    pub highlights: Vec<Highlight>,
    /// Where the reader was last sent from the panel, so the page can point at
    /// the passage rather than merely arrive at it.
    pub spotlight: Option<Spotlight>,
}

impl OpenDocument {
    pub fn new(
        document: Document,
        size: PageSize,
        sizes: Vec<PageSize>,
        page: u32,
        layout: Layout,
    ) -> Self {
        let page_count = document.page_count();
        let rows = Rows::build(layout, &sizes);

        Self {
            document: Arc::new(document),
            page_count,
            size,
            page: page.clamp(1, page_count.max(1)),
            layout,
            sizes,
            rows,
            pages: HashMap::new(),
            requested: HashSet::new(),
            unusable: HashSet::new(),
            generation: 0,
            selection: None,
            highlights: Vec::new(),
            spotlight: None,
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
            && !self.unusable.contains(&page)
    }

    /// Throws away every page, because the size they were drawn for has
    /// changed. Pages still being rasterised are left to finish and dropped on
    /// arrival.
    pub fn resized(&mut self) {
        self.pages.clear();
        self.requested.clear();
        // Given another chance at the new size: a page that could not be drawn
        // one way may be drawable another, and the reader has just asked for
        // something to change.
        self.unusable.clear();
        self.generation += 1;
    }

    /// Notes that a page cannot be drawn, so it is not asked for again.
    pub fn cannot_draw(&mut self, page: u32) {
        self.requested.remove(&page);
        self.unusable.insert(page);
    }

    /// Files a rasterised page, and forgets the ones the reader has left far
    /// enough behind.
    pub fn store(&mut self, page: u32, rasterised: Page) {
        self.requested.remove(&page);
        self.pages.insert(page, rasterised);

        let here = self.page;
        self.pages.retain(|number, _| number.abs_diff(here) <= KEEP);

        // A jump lands long before the page it aimed at, so the passage is
        // looked for here rather than where the jump was made.
        self.find_spotlight();
    }

    /// How wide `page` is when drawn `height` logical pixels tall: its own
    /// shape once it has been read, the first page's before that — which is
    /// what the rows are sized by before anything has been read at all.
    pub fn width_of(&self, page: u32, height: f32) -> f32 {
        width_at(self.size_of(page), height)
    }

    /// A page's own size, known from the moment the book was opened.
    pub fn size_of(&self, page: u32) -> PageSize {
        self.sizes
            .get(page.max(1) as usize - 1)
            .copied()
            .unwrap_or(self.size)
    }

    /// Lays the book out again, for a layout it was not built for.
    pub fn lay_out(&mut self, layout: Layout) {
        if self.rows.layout() != layout {
            self.rows = Rows::build(layout, &self.sizes);
        }
    }

    /// How large a page is drawn inside a box `width` by `height`.
    ///
    /// Fitted rather than scaled by height alone, because a book is not all one
    /// shape: a plan or a scanned spread turns up sideways in a book of upright
    /// pages, and a sideways A3 drawn to the height of an A4 is half again as
    /// wide as the column it is in. Fitting keeps every page inside its column
    /// whatever shape it is, and a page that does not fill the column is
    /// centred in it rather than stretched to it.
    pub fn drawn_size(&self, page: u32, height: f32, width: f32) -> (f32, f32) {
        fitted(self.size_of(page), height, width)
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

    /// Points at a passage, and finds it if the page it is on has been read.
    pub fn spotlight(&mut self, spotlight: Spotlight) {
        self.spotlight = Some(spotlight);
        self.find_spotlight();
    }

    /// Stops pointing at anything. The reader touching the page is a reader who
    /// has found what they were sent to, and the pointer has outstayed it.
    pub fn clear_spotlight(&mut self) {
        self.spotlight = None;
    }

    /// Turns a spotlight known only by its words into rectangles.
    ///
    /// A passage that is not on the page it claims puts the spotlight out
    /// rather than leaving it looking again at every page that lands: the page
    /// has been read, so the answer will not change.
    fn find_spotlight(&mut self) {
        let Some(spotlight) = &self.spotlight else {
            return;
        };
        let Some(needle) = spotlight
            .needle
            .as_deref()
            .filter(|_| spotlight.rects.is_empty())
        else {
            return;
        };
        let Some(held) = self.pages.get(&spotlight.page) else {
            return;
        };

        match held.text.locate(needle) {
            Some((from, to)) => {
                let rects = held.text.line_rects(from, to);
                if let Some(spotlight) = &mut self.spotlight {
                    spotlight.rects = rects;
                }
            }
            None => self.spotlight = None,
        }
    }

    /// The lines to point at on `page`, if that is where the reader was sent.
    pub fn spotlight_rects(&self, page: u32) -> &[Rect] {
        match &self.spotlight {
            Some(spotlight) if spotlight.page == page => &spotlight.rects,
            _ => &[],
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
            .rfind(|highlight| highlight.rects.iter().any(|rect| rect.contains(x, y)))
    }
}

/// How wide a page of `size` is when drawn `height` logical pixels tall.
/// A page of `size` drawn as large as it can be inside a box.
fn fitted(size: PageSize, height: f32, width: f32) -> (f32, f32) {
    if size.width <= 0. || size.height <= 0. {
        return (width, height);
    }

    let scale = (height / size.height).min(width / size.width);

    (size.width * scale, size.height * scale)
}

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

#[cfg(test)]
mod fitting {
    use super::{PageSize, fitted};

    fn sized(width: f32, height: f32) -> PageSize {
        PageSize { width, height }
    }

    /// An upright page in a column wider than it needs is decided by the
    /// height, which is what makes every upright page in a book one size.
    #[test]
    fn an_upright_page_is_as_tall_as_it_is_allowed() {
        let (width, height) = fitted(sized(595., 842.), 640., 490.);

        assert!((height - 640.).abs() < 0.01, "{height}");
        assert!((width - 452.3).abs() < 0.5, "{width}");
    }

    /// A sideways A3 among upright A4s is the case this exists for: by height
    /// alone it would be 905 wide in a column of 490.
    #[test]
    fn a_sideways_page_is_held_to_the_width_of_its_column() {
        let a3_landscape = sized(1191., 842.);
        let (width, height) = fitted(a3_landscape, 640., 490.);

        assert!(width <= 490.01, "{width} spilled out of its column");
        assert!(height < 640., "{height} should be short, not tall");
        // And it keeps its shape.
        assert!(
            (width / height - 1191. / 842.).abs() < 0.01,
            "{width}x{height}"
        );
    }

    /// Every upright page of the same shape comes out the same size, whatever
    /// else is in the book — which is what stops a row moving as its
    /// neighbours load.
    #[test]
    fn pages_of_one_shape_are_one_size() {
        let a4 = fitted(sized(595., 842.), 640., 490.);
        let also_a4 = fitted(sized(1190., 1684.), 640., 490.);

        assert!((a4.0 - also_a4.0).abs() < 0.01);
        assert!((a4.1 - also_a4.1).abs() < 0.01);
    }

    #[test]
    fn a_page_of_no_size_fills_what_it_is_given() {
        assert_eq!(fitted(sized(0., 0.), 640., 490.), (490., 640.));
    }
}

#[cfg(test)]
mod pages_that_will_not_draw {
    use super::*;

    /// A real document, because there is no making one without pdfium — and
    /// the sizes it reports are the ones the layout is built from.
    fn a_document() -> OpenDocument {
        let path = std::env::temp_dir().join("pedro-app-unusable.pdf");
        std::fs::write(
            &path,
            pedro_pdf::fixtures::pdf_with_pages(&["one", "two", "three", "four"]),
        )
        .expect("a writable file");

        let document = Document::open(&path).expect("a readable pdf");
        let size = document.page_size(0).expect("a size");
        let sizes = document.page_sizes().expect("every size");

        OpenDocument::new(document, size, sizes, 1, Layout::Single)
    }

    /// A page that cannot be drawn is not asked for again. Asking again gets
    /// the same answer on every frame for as long as the book is open, which is
    /// not a blank page but a core spinning.
    #[test]
    fn a_page_that_cannot_be_drawn_is_not_asked_for_again() {
        let mut open = a_document();
        assert!(open.wants(3));

        open.cannot_draw(3);
        assert!(!open.wants(3), "it would have been asked for again");

        // And only that page.
        assert!(open.wants(4));
    }

    /// Drawing at another size is another chance: the reader has just asked for
    /// something to change.
    #[test]
    fn a_new_size_gives_every_page_another_chance() {
        let mut open = a_document();
        open.cannot_draw(3);

        open.resized();
        assert!(open.wants(3));
    }
}
