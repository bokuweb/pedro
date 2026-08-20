//! Reading a real document, end to end, through a real pdfium.
//!
//! The unit tests cover the coordinate arithmetic without a library present.
//! These are the ones that would notice pdfium changing under us — a page
//! rendering at the wrong size, text coming back in a different order, a
//! character losing its box — so they need the actual library and fail with
//! instructions rather than skipping when it is missing.
//!
//! They also run in parallel, like any other test binary, which is the check
//! that `pedro-pdf` really does serialise pdfium for its callers: without that
//! lock this file aborts the whole process rather than failing.

use pedro_pdf::fixtures::pdf_with_pages;
use pedro_pdf::{Document, PdfError, PixelFormat};

fn open(pages: &[&str]) -> Document {
    match Document::from_bytes(pdf_with_pages(pages)) {
        Ok(document) => document,
        Err(PdfError::LibraryUnavailable(attempts)) => panic!(
            "these tests need pdfium; run scripts/fetch-pdfium.sh or set \
             PEDRO_PDFIUM_PATH ({attempts})"
        ),
        Err(err) => panic!("the hand-written PDF was rejected: {err}"),
    }
}

#[test]
fn every_page_is_counted() {

    assert_eq!(open(&["one", "two", "three"]).page_count(), 3);
}

#[test]
fn a_page_reports_its_size_in_points() {

    let size = open(&["one"]).page_size(0).expect("page 0 exists");
    assert_eq!(size.width, 300.0);
    assert_eq!(size.height, 200.0);
}

#[test]
fn a_page_past_the_end_is_an_error() {

    let error = open(&["one"]).page_size(7).unwrap_err();
    assert!(
        matches!(
            error,
            PdfError::NoSuchPage {
                requested: 7,
                page_count: 1
            }
        ),
        "{error}"
    );
}

#[test]
fn the_text_written_onto_a_page_comes_back_off_it() {

    let text = open(&["Hello pedro"]).page_text(0).expect("page 0 exists");
    assert_eq!(text.text.trim(), "Hello pedro");
}

#[test]
fn every_visible_character_has_a_box_inside_the_page() {

    let text = open(&["Hello"]).page_text(0).expect("page 0 exists");

    assert_eq!(text.chars.len(), "Hello".len());
    for character in &text.chars {
        let rect = character.rect;
        assert!(rect.left >= 0.0 && rect.right <= 1.0, "{rect:?}");
        assert!(rect.top >= 0.0 && rect.bottom <= 1.0, "{rect:?}");
        assert!(rect.width() > 0.0 && rect.height() > 0.0, "{rect:?}");
    }
}

/// The one property the citation lookup depends on: the boxes address the
/// string, so the text under a drag is the text the reader saw.
#[test]
fn a_boxs_index_addresses_the_page_text() {

    let text = open(&["Hello"]).page_text(0).expect("page 0 exists");
    let third = text.chars[2];

    assert_eq!(text.slice(third.index, third.index), "l");
}

#[test]
fn the_characters_of_a_word_run_left_to_right() {

    let text = open(&["Hello"]).page_text(0).expect("page 0 exists");

    let lefts: Vec<f32> = text.chars.iter().map(|character| character.rect.left).collect();
    assert!(
        lefts.windows(2).all(|pair| pair[0] < pair[1]),
        "characters out of order: {lefts:?}"
    );
}

#[test]
fn the_full_text_joins_pages_with_a_form_feed() {

    let full_text = open(&["first", "second"]).full_text().expect("readable pages");

    let pages: Vec<&str> = full_text.split('\u{000C}').collect();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].trim(), "first");
    assert_eq!(pages[1].trim(), "second");
}

#[test]
fn a_page_renders_at_the_requested_scale() {

    let image = open(&["one"]).render_page(0, 2.0).expect("page 0 exists");

    assert_eq!((image.width, image.height), (600, 400));
    assert_eq!(image.format, PixelFormat::Bgra8);
    assert_eq!(image.bytes.len(), 600 * 400 * 4);
}

#[test]
fn a_scale_that_rounds_to_nothing_still_renders_a_pixel() {

    let image = open(&["one"]).render_page(0, 0.0).expect("page 0 exists");
    assert_eq!((image.width, image.height), (1, 1));
}

#[test]
fn a_document_without_bookmarks_has_no_outline() {

    assert!(open(&["one"]).outline().is_empty());
}

#[test]
fn a_document_opens_from_a_file() {

    let path = std::env::temp_dir().join("pedro-pdf-open-test.pdf");
    std::fs::write(&path, pdf_with_pages(&["from disk"])).expect("a writable temp directory");

    let document = Document::open(&path).expect("pdfium and a readable file");
    assert_eq!(document.page_count(), 1);

    std::fs::remove_file(&path).ok();
}
