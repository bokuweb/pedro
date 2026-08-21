//! Choosing how much of the book travels with a question.
//!
//! A port of chatbook's `documentExcerpt.ts`, including its tests. Sending the
//! whole book with every question would be both slow and, on a long book,
//! impossible; sending only the highlighted sentence answers almost nothing.
//! The unit in between is the chapter the highlight sits in, which the book's
//! own outline already names.

use pedro_pdf::OutlineItem;

/// The character pages are joined with in the stored full text.
pub const PAGE_DELIMITER: char = '\u{000C}';

/// Pages taken on each side of the highlight when the book has no usable
/// outline. 10 makes a 21-page excerpt — about one chapter of a typical
/// 200-page, 10-to-15-chapter technical book, which is the unit the chapter
/// path sends when it can.
pub const FALLBACK_WINDOW_PAGES: u32 = 10;

/// The slice of the book a question is given instead of the whole text.
///
/// `text` is always a verbatim run of consecutive pages out of the full text,
/// joined with the same delimiter they were stored with, so a passage the
/// model quotes from it is guaranteed to be found again by
/// [`crate::citation::find_page_number`]'s whole-text scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excerpt {
    pub text: String,
    pub start_page: u32,
    pub end_page: u32,
    pub total_pages: u32,
    pub is_partial: bool,
}

/// Cuts the parts of the book worth sending with a question about several
/// passages at once.
///
/// One excerpt per chapter the passages fall in, in page order, without
/// repeats: two passages from the same chapter are one piece of context, and
/// sending it twice would pay for it twice and tell the model nothing new.
pub fn select_excerpts(full_text: &str, pages: &[u32], outline: &[OutlineItem]) -> Vec<Excerpt> {
    let mut cut: Vec<Excerpt> = Vec::new();

    for page in pages {
        let excerpt = select_excerpt(full_text, *page, outline);
        if !cut
            .iter()
            .any(|held| held.start_page == excerpt.start_page && held.end_page == excerpt.end_page)
        {
            cut.push(excerpt);
        }
    }

    cut.sort_by_key(|excerpt| excerpt.start_page);
    cut
}

/// Cuts the part of the book worth sending with a question about the page the
/// highlight sits on: the chapter holding that page when the outline names
/// one, a [`FALLBACK_WINDOW_PAGES`] window around it otherwise.
///
/// The page count is taken from the text itself — its delimiters — never from
/// a stored column, so a mismatch cannot label a whole text as partial or cut
/// past the last page. A text without delimiters, such as a one-page book, is
/// sent whole.
pub fn select_excerpt(full_text: &str, selection_page: u32, outline: &[OutlineItem]) -> Excerpt {
    let pages: Vec<&str> = full_text.split(PAGE_DELIMITER).collect();
    let total_pages = pages.len() as u32;

    if total_pages <= 1 {
        return Excerpt {
            text: full_text.to_owned(),
            start_page: 1,
            end_page: total_pages,
            total_pages,
            is_partial: false,
        };
    }

    let page = selection_page.clamp(1, total_pages);
    let (start_page, end_page) = chapter_bounds(outline, page, total_pages).unwrap_or((
        page.saturating_sub(FALLBACK_WINDOW_PAGES).max(1),
        (page + FALLBACK_WINDOW_PAGES).min(total_pages),
    ));

    Excerpt {
        text: pages[start_page as usize - 1..end_page as usize].join(&PAGE_DELIMITER.to_string()),
        start_page,
        end_page,
        total_pages,
        is_partial: !(start_page == 1 && end_page == total_pages),
    }
}

/// The chapter interval holding `page`, or `None` when the outline gives no
/// usable bounds.
///
/// Chapter starts outside the book are dropped; two chapters naming the same
/// start page collapse into the first, so no chapter is ever empty. Pages
/// before the first chapter form a front-matter interval of their own.
fn chapter_bounds(outline: &[OutlineItem], page: u32, total_pages: u32) -> Option<(u32, u32)> {
    let mut starts: Vec<u32> = outline
        .iter()
        .map(|chapter| chapter.page_number)
        .filter(|start| *start >= 1 && *start <= total_pages)
        .collect();
    starts.sort_unstable();
    starts.dedup();

    if starts.is_empty() {
        return None;
    }

    let mut bounds = Vec::with_capacity(starts.len() + 2);
    bounds.push(1);
    bounds.extend(starts);
    bounds.push(total_pages + 1);

    let index = bounds.iter().rposition(|start| *start <= page)?;
    Some((bounds[index], bounds[index + 1] - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(title: &str, page_number: u32) -> OutlineItem {
        OutlineItem {
            title: title.to_owned(),
            page_number,
        }
    }

    fn outline() -> Vec<OutlineItem> {
        vec![
            chapter("第1章", 2),
            chapter("第2章", 5),
            chapter("第3章", 9),
        ]
    }

    /// A book of `page_count` pages whose page *n* reads `pn`.
    fn book_of(page_count: u32) -> String {
        (1..=page_count)
            .map(|page| format!("p{page}"))
            .collect::<Vec<_>>()
            .join(&PAGE_DELIMITER.to_string())
    }

    /// The text of pages `start` to `end`, as `select_excerpt` should cut it.
    fn pages_text(start: u32, end: u32) -> String {
        (start..=end)
            .map(|page| format!("p{page}"))
            .collect::<Vec<_>>()
            .join(&PAGE_DELIMITER.to_string())
    }

    fn excerpt(text: &str, start_page: u32, end_page: u32, total_pages: u32) -> Excerpt {
        Excerpt {
            text: text.to_owned(),
            start_page,
            end_page,
            total_pages,
            is_partial: !(start_page == 1 && end_page == total_pages),
        }
    }

    #[test]
    fn sends_only_the_chapter_that_holds_the_highlighted_page() {
        assert_eq!(
            select_excerpt(&book_of(12), 6, &outline()),
            excerpt(&pages_text(5, 8), 5, 8, 12)
        );
    }

    #[test]
    fn counts_a_chapters_opening_page_as_part_of_that_chapter() {
        assert_eq!(
            select_excerpt(&book_of(12), 5, &outline()),
            excerpt(&pages_text(5, 8), 5, 8, 12)
        );
    }

    #[test]
    fn runs_the_last_chapter_to_the_final_page_of_the_book() {
        assert_eq!(
            select_excerpt(&book_of(12), 10, &outline()),
            excerpt(&pages_text(9, 12), 9, 12, 12)
        );
    }

    #[test]
    fn treats_pages_before_the_first_chapter_as_front_matter_of_their_own() {
        assert_eq!(
            select_excerpt(&book_of(12), 1, &outline()),
            excerpt(&pages_text(1, 1), 1, 1, 12)
        );
    }

    #[test]
    fn hands_back_the_whole_book_marked_whole_when_its_one_chapter_spans_it() {
        let whole = vec![chapter("全部", 1)];
        let result = select_excerpt(&book_of(3), 2, &whole);

        assert_eq!(result, excerpt(&pages_text(1, 3), 1, 3, 3));
        assert!(!result.is_partial);
    }

    /// The property the citation lookup rests on: the excerpt sits in the full
    /// text exactly where its first page does. An empty excerpt, or one with
    /// anything injected into it, would not.
    #[test]
    fn keeps_the_excerpt_a_verbatim_slice_of_the_full_text() {
        let full_text = book_of(12);
        let cut = select_excerpt(&full_text, 6, &outline());

        assert_eq!(full_text.find(&cut.text), full_text.find("p5"));
    }

    #[test]
    fn keeps_a_chapter_starting_on_page_one_whole() {
        let outline = vec![chapter("第1章", 1), chapter("第2章", 5)];

        assert_eq!(
            select_excerpt(&book_of(12), 3, &outline),
            excerpt(&pages_text(1, 4), 1, 4, 12)
        );
    }

    #[test]
    fn orders_an_unsorted_outline_before_cutting_chapter_bounds() {
        let shuffled = vec![
            chapter("第3章", 9),
            chapter("第1章", 2),
            chapter("第2章", 5),
        ];

        assert_eq!(
            select_excerpt(&book_of(12), 6, &shuffled),
            excerpt(&pages_text(5, 8), 5, 8, 12)
        );
    }

    #[test]
    fn lets_the_first_of_two_chapters_naming_the_same_start_page_win() {
        let doubled = vec![
            chapter("第1章", 2),
            chapter("第2章", 5),
            chapter("第2章の重複", 5),
            chapter("第3章", 9),
        ];

        assert_eq!(
            select_excerpt(&book_of(12), 5, &doubled),
            excerpt(&pages_text(5, 8), 5, 8, 12)
        );
    }

    #[test]
    fn ignores_chapters_pointing_outside_the_book() {
        let mut stray = outline();
        stray.push(chapter("落丁", 99));

        assert_eq!(
            select_excerpt(&book_of(12), 10, &stray),
            excerpt(&pages_text(9, 12), 9, 12, 12)
        );
    }

    #[test]
    fn falls_back_to_the_page_window_when_every_chapter_points_outside_the_book() {
        let stray = vec![chapter("落丁", 99)];

        assert_eq!(
            select_excerpt(&book_of(30), 15, &stray),
            excerpt(&pages_text(5, 25), 5, 25, 30)
        );
    }

    #[test]
    fn cuts_a_window_around_the_highlight_when_the_book_has_no_outline() {
        assert_eq!(
            select_excerpt(&book_of(30), 15, &[]),
            excerpt(&pages_text(5, 25), 5, 25, 30)
        );
    }

    #[test]
    fn stops_the_window_at_the_front_cover_rather_than_asking_for_page_zero() {
        assert_eq!(
            select_excerpt(&book_of(30), 2, &[]),
            excerpt(&pages_text(1, 12), 1, 12, 30)
        );
    }

    #[test]
    fn stops_the_window_at_the_back_cover_rather_than_past_the_book() {
        assert_eq!(
            select_excerpt(&book_of(30), 29, &[]),
            excerpt(&pages_text(19, 30), 19, 30, 30)
        );
    }

    #[test]
    fn hands_back_a_window_that_covers_a_small_book_whole_marked_whole() {
        let result = select_excerpt(&book_of(12), 6, &[]);

        assert_eq!(result, excerpt(&pages_text(1, 12), 1, 12, 12));
        assert!(!result.is_partial);
    }

    #[test]
    fn hands_back_a_text_without_page_breaks_whole() {
        assert_eq!(
            select_excerpt("切れ目の無い 本文", 3, &outline()),
            excerpt("切れ目の無い 本文", 1, 1, 1)
        );
    }

    #[test]
    fn two_passages_in_one_chapter_are_one_excerpt() {
        let cut = select_excerpts(&book_of(12), &[5, 7], &outline());

        assert_eq!(cut.len(), 1);
        assert_eq!((cut[0].start_page, cut[0].end_page), (5, 8));
    }

    #[test]
    fn passages_in_two_chapters_are_two_excerpts_in_page_order() {
        let cut = select_excerpts(&book_of(12), &[10, 3], &outline());

        assert_eq!(cut.len(), 2);
        assert_eq!((cut[0].start_page, cut[0].end_page), (2, 4));
        assert_eq!((cut[1].start_page, cut[1].end_page), (9, 12));
    }

    #[test]
    fn no_passages_is_no_context() {
        assert!(select_excerpts(&book_of(12), &[], &outline()).is_empty());
    }

    #[test]
    fn pulls_a_highlight_pointing_past_the_book_back_to_the_last_page() {
        assert_eq!(
            select_excerpt(&book_of(12), 99, &outline()),
            excerpt(&pages_text(9, 12), 9, 12, 12)
        );
    }
}
