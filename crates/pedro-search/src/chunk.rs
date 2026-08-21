//! Cutting a book into the pieces a search returns.
//!
//! A hit has to name a page, because a page is where the reader can be sent, so
//! nothing here ever spans one. Within a page the text is cut into overlapping
//! windows: a passage that straddles a cut would otherwise be findable by
//! neither half, and the overlap is what stops that.

/// How long a chunk is, in characters.
///
/// Long enough that a paragraph usually survives whole — which is what makes a
/// hit readable on its own — and short enough that a page of a technical book
/// is several of them rather than one.
const LENGTH: usize = 400;

/// How much of the end of one chunk the next one repeats.
const OVERLAP: usize = 80;

/// Where a chunk cut is allowed to land, in preference order. A cut at a
/// sentence end reads as a passage; a cut mid-word reads as damage.
const BOUNDARIES: [char; 6] = ['。', '.', '\n', '！', '？', '、'];

/// One piece of a book, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// One-based, the way the reader counts.
    pub page_number: u32,
    /// Position within the book, so chunks come back in reading order.
    pub ord: u32,
    pub text: String,
}

/// Cuts a book's text — pages joined by `delimiter` — into chunks.
pub fn split(full_text: &str, delimiter: char) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut ord = 0;

    for (index, page) in full_text.split(delimiter).enumerate() {
        for text in split_page(page) {
            chunks.push(Chunk {
                page_number: index as u32 + 1,
                ord,
                text,
            });
            ord += 1;
        }
    }

    chunks
}

/// Cuts one page into overlapping windows, preferring to break at the end of a
/// sentence.
fn split_page(page: &str) -> Vec<String> {
    let characters: Vec<char> = page.chars().collect();
    if characters.iter().all(|c| c.is_whitespace()) {
        return Vec::new();
    }
    if characters.len() <= LENGTH {
        return vec![page.trim().to_owned()];
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < characters.len() {
        let end = cut_at(&characters, start);
        let text: String = characters[start..end].iter().collect();
        let text = text.trim();
        if !text.is_empty() {
            chunks.push(text.to_owned());
        }

        if end >= characters.len() {
            break;
        }
        // Step back so the next chunk repeats the end of this one, and forward
        // at least once so a page can never produce chunks for ever.
        start = (end - OVERLAP.min(end - start - 1)).max(start + 1);
    }

    chunks
}

/// Where the chunk starting at `start` should end: the last sentence boundary
/// inside the window, or the end of the window when it holds none.
fn cut_at(characters: &[char], start: usize) -> usize {
    let end = (start + LENGTH).min(characters.len());
    if end == characters.len() {
        return end;
    }

    // Only look in the last part of the window: a boundary near the start would
    // make a chunk far shorter than it should be.
    let earliest = start + LENGTH / 2;
    characters[earliest..end]
        .iter()
        .rposition(|c| BOUNDARIES.contains(c))
        .map(|at| earliest + at + 1)
        .unwrap_or(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: char = '\u{000C}';

    #[test]
    fn a_short_page_is_one_chunk() {
        let chunks = split("short page", PAGE);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "short page");
        assert_eq!(chunks[0].page_number, 1);
    }

    #[test]
    fn every_chunk_knows_its_page() {
        let chunks = split(&format!("one{PAGE}two{PAGE}three"), PAGE);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[2].page_number, 3);
        assert_eq!(chunks[2].text, "three");
    }

    /// Blank pages are common in a scanned book and have nothing to find.
    #[test]
    fn an_empty_page_produces_nothing_but_still_counts() {
        let chunks = split(&format!("one{PAGE}   {PAGE}three",), PAGE);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[1].page_number, 3);
    }

    #[test]
    fn a_long_page_is_cut_into_several() {
        let page = "あ".repeat(1000);
        let chunks = split(&page, PAGE);

        assert!(chunks.len() > 2, "{}", chunks.len());
        assert!(chunks.iter().all(|chunk| chunk.page_number == 1));
        assert!(chunks.iter().all(|chunk| chunk.text.chars().count() <= 400));
    }

    /// A passage that lands on a cut has to be findable, which is what the
    /// overlap is for.
    #[test]
    fn chunks_overlap_so_nothing_falls_between_them() {
        let page = "あ".repeat(500) + "みつけて" + &"い".repeat(500);
        let chunks = split(&page, PAGE);

        assert!(
            chunks.iter().any(|chunk| chunk.text.contains("みつけて")),
            "the passage on the seam was lost"
        );
    }

    #[test]
    fn a_cut_prefers_the_end_of_a_sentence() {
        let page = format!("{}。{}", "あ".repeat(300), "い".repeat(300));
        let chunks = split(&page, PAGE);

        assert!(chunks[0].text.ends_with('。'), "{}", chunks[0].text);
    }

    #[test]
    fn chunks_are_numbered_in_reading_order() {
        let page = "あ".repeat(1200);
        let chunks = split(&format!("{page}{PAGE}{page}"), PAGE);

        let ords: Vec<u32> = chunks.iter().map(|chunk| chunk.ord).collect();
        assert_eq!(ords, (0..chunks.len() as u32).collect::<Vec<_>>());
    }

    /// A page of nothing but boundaries must not loop for ever.
    #[test]
    fn a_page_of_punctuation_terminates() {
        let chunks = split(&"。".repeat(2000), PAGE);
        assert!(!chunks.is_empty());
    }
}
