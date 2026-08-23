//! How pages are grouped into the rows the reader scrolls through.
//!
//! One page to a row, or two side by side the way the book was printed. A
//! printed spread is not "pages 1 and 2": page 1 is a right-hand page with
//! nothing facing it, and the pairs run 2–3, 4–5 from there. Getting that wrong
//! puts every spread half a book out of step with the paper it is a picture of.
//!
//! Everything else in the reader counts in pages and the scrolling list counts
//! in rows, so this is the one place that knows how to turn one into the other.

use pedro_pdf::PageSize;

/// How many pages are drawn side by side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    #[default]
    Single,
    /// Two facing pages, as the book was printed.
    Spread,
}

impl Layout {
    pub fn from_stored(spread: Option<bool>) -> Self {
        match spread {
            Some(true) => Layout::Spread,
            _ => Layout::Single,
        }
    }

    pub fn is_spread(self) -> bool {
        self == Layout::Spread
    }

    pub fn toggled(self) -> Self {
        match self {
            Layout::Single => Layout::Spread,
            Layout::Spread => Layout::Single,
        }
    }
}

/// Which pages are drawn in which row, worked out once for a book.
///
/// The arithmetic version of this — the cover alone, then pairs — is right
/// only for a book that is all one shape. A page turned sideways among upright
/// ones is a fold-out: it is the spread, and pairing it with the page after it
/// gives the reader half a plan beside an unrelated page of text. So a sideways
/// page takes a row to itself, and so does the page it would have faced, which
/// keeps every later pair on the same side of the book as before.
///
/// Built from every page's size rather than from the first page's, because a
/// row that consults the pages as they arrive moves while the reader is reading
/// it. Every size is known before the first page is drawn — the whole table
/// costs about seven milliseconds for a five-hundred-page book.
pub struct Rows {
    layout: Layout,
    rows: Vec<Vec<u32>>,
    /// The row each page is in, indexed by page number minus one.
    of_page: Vec<usize>,
}

impl Rows {
    pub fn build(layout: Layout, sizes: &[PageSize]) -> Self {
        let count = sizes.len() as u32;
        let sideways = |page: u32| {
            sizes
                .get(page as usize - 1)
                .is_some_and(|size| size.width > size.height)
        };

        let mut rows: Vec<Vec<u32>> = Vec::new();
        match layout {
            Layout::Single => rows.extend((1..=count).map(|page| vec![page])),
            Layout::Spread => {
                if count >= 1 {
                    rows.push(vec![1]);
                }

                let mut page = 2;
                while page <= count {
                    let facing = (page < count).then_some(page + 1);
                    match facing {
                        // A sideways page anywhere in the pair breaks it, and
                        // both halves take a row of their own.
                        Some(next) if sideways(page) || sideways(next) => {
                            rows.push(vec![page]);
                            rows.push(vec![next]);
                        }
                        Some(next) => rows.push(vec![page, next]),
                        None => rows.push(vec![page]),
                    }

                    page += 2;
                }
            }
        }

        let mut of_page = vec![0; count as usize];
        for (index, row) in rows.iter().enumerate() {
            for page in row {
                of_page[*page as usize - 1] = index;
            }
        }

        Self {
            layout,
            rows,
            of_page,
        }
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// The pages in a row, empty past the end of the book.
    pub fn pages(&self, row: usize) -> &[u32] {
        self.rows.get(row).map(Vec::as_slice).unwrap_or_default()
    }

    /// The first page of a row, which is the page the reader is taken to be on.
    ///
    /// `None` past the end of the book. The scrolling list asks about rows that
    /// are not there — it measures with ranges beyond the last row — and
    /// answering "page 1" for those is not a smaller mistake than answering
    /// nothing: it tells the reader they are back at the cover, which moves the
    /// place, which saves it, which draws again.
    pub fn first_page(&self, row: usize) -> Option<u32> {
        self.pages(row).first().copied()
    }

    /// The row a page is drawn in.
    pub fn row_of(&self, page: u32) -> usize {
        self.of_page
            .get(page.max(1) as usize - 1)
            .copied()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::{Layout, Rows};
    use pedro_pdf::PageSize;

    fn upright() -> PageSize {
        PageSize {
            width: 595.,
            height: 842.,
        }
    }

    fn sideways() -> PageSize {
        PageSize {
            width: 1191.,
            height: 842.,
        }
    }

    fn book(pages: usize) -> Vec<PageSize> {
        vec![upright(); pages]
    }

    #[test]
    fn one_page_to_a_row_is_the_page_itself() {
        let rows = Rows::build(Layout::Single, &book(30));

        assert_eq!(rows.len(), 30);
        assert_eq!(rows.pages(0), [1]);
        assert_eq!(rows.pages(16), [17]);
        assert_eq!(rows.row_of(17), 16);
    }

    /// The cover faces nothing, which is what makes every later pair even-odd.
    #[test]
    fn the_cover_is_alone_and_the_rest_pair_off() {
        let rows = Rows::build(Layout::Spread, &book(30));

        assert_eq!(rows.pages(0), [1]);
        assert_eq!(rows.pages(1), [2, 3]);
        assert_eq!(rows.pages(2), [4, 5]);
    }

    #[test]
    fn a_page_knows_which_row_it_is_in() {
        let rows = Rows::build(Layout::Spread, &book(30));

        assert_eq!(rows.row_of(1), 0);
        assert_eq!(rows.row_of(2), 1);
        assert_eq!(rows.row_of(3), 1);
        assert_eq!(rows.row_of(4), 2);

        for page in 1..=30 {
            let row = rows.row_of(page);
            assert!(rows.pages(row).contains(&page), "page {page}");
        }
    }

    /// A fold-out is the spread. Pairing it with the page after it gives the
    /// reader half a plan beside an unrelated page of text.
    #[test]
    fn a_sideways_page_takes_a_row_to_itself() {
        let mut sizes = book(12);
        sizes[3] = sideways(); // page 4

        let rows = Rows::build(Layout::Spread, &sizes);

        assert_eq!(rows.pages(0), [1]);
        assert_eq!(rows.pages(1), [2, 3]);
        assert_eq!(rows.pages(2), [4], "the fold-out shared its row");
        assert_eq!(rows.pages(3), [5], "the page it would have faced");
        // And the pairs afterwards are on the same side of the book as before.
        assert_eq!(rows.pages(4), [6, 7]);
    }

    /// Two fold-outs facing each other are still a row each: they are two
    /// sheets, not one spread.
    #[test]
    fn two_sideways_pages_do_not_pair_with_each_other() {
        let mut sizes = book(8);
        sizes[3] = sideways();
        sizes[4] = sideways();

        let rows = Rows::build(Layout::Spread, &sizes);

        assert_eq!(rows.pages(2), [4]);
        assert_eq!(rows.pages(3), [5]);
    }

    /// A book with an even number of pages ends on a half-empty spread rather
    /// than on a page that is not there.
    #[test]
    fn the_last_spread_may_hold_one_page() {
        assert_eq!(Rows::build(Layout::Spread, &book(4)).len(), 3);
        assert_eq!(Rows::build(Layout::Spread, &book(4)).pages(2), [4]);

        assert_eq!(Rows::build(Layout::Spread, &book(5)).len(), 3);
        assert_eq!(Rows::build(Layout::Spread, &book(5)).pages(2), [4, 5]);
    }

    #[test]
    fn a_book_of_one_page_has_one_row_either_way() {
        assert_eq!(Rows::build(Layout::Single, &book(1)).len(), 1);
        assert_eq!(Rows::build(Layout::Spread, &book(1)).len(), 1);
        assert_eq!(Rows::build(Layout::Spread, &book(1)).pages(0), [1]);
    }

    /// The list measures itself with ranges past the last row, so this is asked
    /// every frame — and answering "page 1" instead of "no page" is what turned
    /// a flutter at startup into a loop that never settled.
    #[test]
    fn a_row_past_the_end_holds_nothing() {
        let rows = Rows::build(Layout::Spread, &book(30));

        assert!(rows.pages(50).is_empty());
        assert_eq!(rows.first_page(50), None);
        assert_eq!(rows.first_page(1), Some(2));
        assert!(Rows::build(Layout::Spread, &[]).pages(0).is_empty());
    }

    #[test]
    fn what_is_stored_is_what_comes_back() {
        assert_eq!(Layout::from_stored(Some(true)), Layout::Spread);
        assert_eq!(Layout::from_stored(Some(false)), Layout::Single);
        assert_eq!(Layout::from_stored(None), Layout::Single);
        assert_eq!(Layout::Single.toggled(), Layout::Spread);
        assert_eq!(Layout::Spread.toggled(), Layout::Single);
    }
}
