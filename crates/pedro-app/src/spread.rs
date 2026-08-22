//! How pages are grouped into the rows the reader scrolls through.
//!
//! One page to a row, or two side by side the way the book was printed. A
//! printed spread is not "pages 1 and 2": page 1 is a right-hand page with
//! nothing facing it, and the pairs run 2–3, 4–5 from there. Getting that wrong
//! puts every spread half a book out of step with the paper it is a picture of.
//!
//! Everything else in the reader counts in pages and the scrolling list counts
//! in rows, so this is the one place that knows how to turn one into the other.

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

    /// How many rows a book of `pages` pages has.
    pub fn rows(self, pages: u32) -> usize {
        match self {
            Layout::Single => pages as usize,
            // The cover has a row of its own, and the rest pair off after it.
            Layout::Spread => match pages {
                0 => 0,
                _ => 1 + (pages as usize - 1).div_ceil(2),
            },
        }
    }

    /// The pages drawn in a row, in the order they are drawn.
    ///
    /// Empty past the end of the book, which is what lets a caller ask about a
    /// row the list has not caught up with yet.
    pub fn pages(self, row: usize, pages: u32) -> Vec<u32> {
        if pages == 0 {
            return Vec::new();
        }

        let wanted: Vec<u32> = match self {
            Layout::Single => vec![row as u32 + 1],
            Layout::Spread if row == 0 => vec![1],
            Layout::Spread => vec![row as u32 * 2, row as u32 * 2 + 1],
        };

        wanted.into_iter().filter(|page| *page <= pages).collect()
    }

    /// The row a page is drawn in.
    pub fn row(self, page: u32) -> usize {
        match self {
            Layout::Single => page.max(1) as usize - 1,
            Layout::Spread => match page {
                0 | 1 => 0,
                page => page as usize / 2,
            },
        }
    }

    /// The first page of a row, which is the page the reader is taken to be on.
    ///
    /// `None` past the end of the book. The scrolling list asks about rows that
    /// are not there — it measures with ranges beyond the last row — and
    /// answering "page 1" for those is not a smaller mistake than answering
    /// nothing: it tells the reader they are back at the cover, which moves the
    /// place, which saves it, which draws again.
    pub fn first_page(self, row: usize, pages: u32) -> Option<u32> {
        self.pages(row, pages).first().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::Layout;

    #[test]
    fn one_page_to_a_row_is_the_page_itself() {
        assert_eq!(Layout::Single.rows(30), 30);
        assert_eq!(Layout::Single.pages(0, 30), vec![1]);
        assert_eq!(Layout::Single.pages(16, 30), vec![17]);
        assert_eq!(Layout::Single.row(17), 16);
    }

    /// The cover faces nothing, which is what makes every later pair even-odd.
    #[test]
    fn the_cover_is_alone_and_the_rest_pair_off() {
        assert_eq!(Layout::Spread.pages(0, 30), vec![1]);
        assert_eq!(Layout::Spread.pages(1, 30), vec![2, 3]);
        assert_eq!(Layout::Spread.pages(2, 30), vec![4, 5]);
    }

    #[test]
    fn a_page_knows_which_row_it_is_in() {
        assert_eq!(Layout::Spread.row(1), 0);
        assert_eq!(Layout::Spread.row(2), 1);
        assert_eq!(Layout::Spread.row(3), 1);
        assert_eq!(Layout::Spread.row(4), 2);

        // And a row's first page is the page that row is at.
        for page in 1..=30 {
            let row = Layout::Spread.row(page);
            assert!(Layout::Spread.pages(row, 30).contains(&page), "page {page}");
        }
    }

    /// A book with an even number of pages ends on a half-empty spread rather
    /// than on a page that is not there.
    #[test]
    fn the_last_spread_may_hold_one_page() {
        assert_eq!(Layout::Spread.rows(4), 3);
        assert_eq!(Layout::Spread.pages(2, 4), vec![4]);

        assert_eq!(Layout::Spread.rows(5), 3);
        assert_eq!(Layout::Spread.pages(2, 5), vec![4, 5]);
    }

    #[test]
    fn a_book_of_one_page_has_one_row_either_way() {
        assert_eq!(Layout::Single.rows(1), 1);
        assert_eq!(Layout::Spread.rows(1), 1);
        assert_eq!(Layout::Spread.pages(0, 1), vec![1]);
    }

    /// The list measures itself with ranges past the last row, so this is asked
    /// every frame — and answering "page 1" instead of "no page" is what turned
    /// a flutter at startup into a loop that never settled.
    #[test]
    fn a_row_past_the_end_holds_nothing() {
        assert!(Layout::Spread.pages(50, 30).is_empty());
        assert!(Layout::Single.pages(50, 30).is_empty());
        assert!(Layout::Spread.pages(0, 0).is_empty());

        assert_eq!(Layout::Single.first_page(50, 30), None);
        assert_eq!(Layout::Spread.first_page(50, 30), None);
        assert_eq!(Layout::Single.first_page(16, 30), Some(17));
        assert_eq!(Layout::Spread.first_page(1, 30), Some(2));
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
