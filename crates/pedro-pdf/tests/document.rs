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

    let lefts: Vec<f32> = text
        .chars
        .iter()
        .map(|character| character.rect.left)
        .collect();
    assert!(
        lefts.windows(2).all(|pair| pair[0] < pair[1]),
        "characters out of order: {lefts:?}"
    );
}

#[test]
fn the_full_text_joins_pages_with_a_form_feed() {
    let full_text = open(&["first", "second"])
        .full_text()
        .expect("readable pages");

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

/// The band of the page that has ink on it, as fractions of its height.
fn inked_band(image: &pedro_pdf::PageImage) -> (f32, f32) {
    let rows: Vec<u32> = (0..image.height)
        .filter(|row| {
            (0..image.width).any(|column| {
                let at = ((row * image.width + column) * 4) as usize;
                image.bytes.get(at).is_some_and(|blue| *blue < 128)
            })
        })
        .collect();

    let height = image.height as f32;
    (
        *rows.first().expect("a page with ink on it") as f32 / height,
        *rows.last().expect("a page with ink on it") as f32 / height,
    )
}

/// Where the character boxes say the ink is, as fractions of the page height.
fn boxed_band(text: &pedro_pdf::PageText) -> (f32, f32) {
    let top = text
        .chars
        .iter()
        .map(|character| character.rect.top)
        .fold(f32::MAX, f32::min);
    let bottom = text
        .chars
        .iter()
        .map(|character| character.rect.bottom)
        .fold(f32::MIN, f32::max);

    (top, bottom)
}

/// A page's characters are reported in the coordinates of its media box, while
/// the page is rasterised from its crop box. A book with trim marks — which is
/// most printed books — has one inset from the other, and normalising against
/// the wrong one puts every mark above the words it belongs to.
///
/// Comparing the middle of what the boxes claim with the middle of the ink is
/// what catches it: an offset moves the claim without moving the ink.
#[test]
fn character_boxes_sit_on_the_ink_of_a_cropped_page() {
    let inset = 12.0;
    let document = Document::from_bytes(pedro_pdf::fixtures::pdf_with_crop_box(&["Hello"], inset))
        .expect("pdfium and a readable fixture");

    let image = document.render_page(0, 4.0).expect("page 0 exists");
    let text = document.page_text(0).expect("page 0 exists");

    let (ink_top, ink_bottom) = inked_band(&image);
    let (box_top, box_bottom) = boxed_band(&text);

    let ink_middle = (ink_top + ink_bottom) / 2.0;
    let box_middle = (box_top + box_bottom) / 2.0;

    // The inset is 12 of 200 points: an error of that size is 0.06 of the page,
    // which is ten times this tolerance.
    assert!(
        (ink_middle - box_middle).abs() < 0.006,
        "the boxes are centred at {box_middle:.4} and the ink at {ink_middle:.4}"
    );
}

/// The same page with no crop box, so a failure above is about the crop box
/// rather than about a fixture that draws nothing.
#[test]
fn character_boxes_sit_on_the_ink_of_a_plain_page() {
    let document =
        Document::from_bytes(pdf_with_pages(&["Hello"])).expect("pdfium and a readable fixture");

    let image = document.render_page(0, 4.0).expect("page 0 exists");
    let text = document.page_text(0).expect("page 0 exists");

    let (ink_top, ink_bottom) = inked_band(&image);
    let (box_top, box_bottom) = boxed_band(&text);

    assert!(
        ((ink_top + ink_bottom) / 2.0 - (box_top + box_bottom) / 2.0).abs() < 0.006,
        "the boxes and the ink disagree on a page with nothing to confuse them"
    );
}

/// A book is not always all one shape: a plan or a scanned spread turns up
/// sideways among upright pages, and every page has to answer for itself.
#[test]
fn a_book_of_mixed_page_sizes_reports_each_one() {
    use pedro_pdf::fixtures::{Page, pdf_with_sizes};

    let path = std::env::temp_dir().join("pedro-pdf-mixed.pdf");
    std::fs::write(
        &path,
        pdf_with_sizes(&[
            Page::sized("upright", 595., 842.),
            Page::sized("sideways", 1191., 842.),
            Page::sized("upright again", 595., 842.),
        ]),
    )
    .expect("a writable file");

    let document = Document::open(&path).expect("a readable pdf");
    assert_eq!(document.page_count(), 3);

    let first = document.page_size(0).expect("a size");
    assert!((first.width - 595.).abs() < 1., "{first:?}");
    assert!((first.height - 842.).abs() < 1., "{first:?}");

    let sideways = document.page_size(1).expect("a size");
    assert!(sideways.width > sideways.height, "{sideways:?}");
    assert!((sideways.width - 1191.).abs() < 1., "{sideways:?}");

    let third = document.page_size(2).expect("a size");
    assert!((third.width - 595.).abs() < 1., "{third:?}");
}

/// The page table and the pages themselves have to agree about how large a page
/// is.
///
/// A printed book is inset from its media box to its crop box, and pdfium
/// answers in both spaces depending on what is asked — which has already cost
/// this reader once, when every mark landed a line above its words. The reader
/// now lays a book out from the page table and draws it from the pages, so a
/// disagreement between the two would stretch every page of a cropped book.
#[test]
fn the_page_table_agrees_with_the_pages_about_size() {
    use pedro_pdf::fixtures::pdf_with_crop_box;

    let path = std::env::temp_dir().join("pedro-pdf-cropped-sizes.pdf");
    std::fs::write(&path, pdf_with_crop_box(&["one", "two", "three"], 20.))
        .expect("a writable file");

    let document = Document::open(&path).expect("a readable pdf");
    let table = document.page_sizes().expect("every size");

    assert_eq!(table.len(), 3);
    for (index, from_table) in table.iter().enumerate() {
        let from_page = document.page_size(index as u32).expect("a size");

        assert!(
            (from_table.width - from_page.width).abs() < 0.5
                && (from_table.height - from_page.height).abs() < 0.5,
            "page {index}: the table says {from_table:?}, the page says {from_page:?}"
        );
    }
}
