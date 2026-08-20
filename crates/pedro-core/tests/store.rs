//! The library on disk, exercised against real files and a real pdfium.

use std::path::{Path, PathBuf};

use pedro_core::model::{NewHighlight, ReadingState, Role};
use pedro_core::store::{Store, StoreError};
use pedro_core::{Citation, CitationKind, PageLocation};
use pedro_pdf::{Rect, fixtures::pdf_with_pages};

/// A library of its own, emptied first so a rerun starts clean.
fn library(name: &str) -> (Store, PathBuf) {
    let root = std::env::temp_dir().join(format!("pedro-store-{name}"));
    let _ = std::fs::remove_dir_all(&root);

    (Store::open(&root).expect("a writable library"), root)
}

/// Writes a PDF next to the library, the way a reader's file sits on disk.
fn pdf(root: &Path, name: &str, pages: &[&str]) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(root).expect("a writable directory");
    std::fs::write(&path, pdf_with_pages(pages)).expect("a writable file");

    path
}

fn rect() -> Rect {
    Rect {
        left: 0.1,
        top: 0.2,
        right: 0.5,
        bottom: 0.24,
    }
}

fn highlight_of(text: &str, page: u32) -> NewHighlight {
    NewHighlight {
        selected_text: text.to_owned(),
        page_number: page,
        rects: vec![rect()],
    }
}

#[test]
fn adding_a_document_reads_its_pages_and_its_text() {
    let (store, root) = library("adds");
    let source = pdf(&root, "book.pdf", &["first", "second"]);

    let book = store.add_document(&source).expect("a readable pdf");

    assert_eq!(book.file_name, "book.pdf");
    assert_eq!(book.page_count, 2);
    assert!(book.reading.is_none());

    let full_text = store.full_text(&book.id).expect("a stored book");
    let pages: Vec<&str> = full_text.split('\u{000C}').collect();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].trim(), "first");
}

#[test]
fn the_bytes_are_copied_into_the_library() {
    let (store, root) = library("copies");
    let source = pdf(&root, "book.pdf", &["only"]);

    let book = store.add_document(&source).expect("a readable pdf");
    std::fs::remove_file(&source).expect("the original can be removed");

    assert!(store.document_path(&book).is_file());
    assert_eq!(store.book(&book.id).expect("a query").unwrap().id, book.id);
}

/// chatbook's most useful property, and the reason identity is the content:
/// re-adding a book keeps everything the reader built on it.
#[test]
fn re_adding_the_same_book_keeps_its_highlights_and_its_place() {
    let (store, root) = library("re-adds");
    let source = pdf(&root, "book.pdf", &["first", "second"]);

    let book = store.add_document(&source).expect("a readable pdf");
    store
        .add_highlight(&book.id, highlight_of("first", 1))
        .expect("a stored book");
    store
        .save_reading_state(
            &book.id,
            &ReadingState {
                page: 2,
                highlight_id: None,
                outline_open: Some(true),
                chat_panel_open: None,
            },
        )
        .expect("a stored book");

    let renamed = pdf(&root, "better name.pdf", &["first", "second"]);
    let again = store.add_document(&renamed).expect("a readable pdf");

    assert_eq!(again.id, book.id);
    assert_eq!(again.file_name, "better name.pdf");
    assert_eq!(again.reading.expect("a saved place").page, 2);
    assert_eq!(store.highlights(&book.id).expect("a stored book").len(), 1);
    assert_eq!(store.books().expect("a library").len(), 1);
}

#[test]
fn books_are_listed_most_recently_touched_first() {
    let (store, root) = library("orders");

    let first = store
        .add_document(&pdf(&root, "first.pdf", &["a"]))
        .expect("a readable pdf");
    let second = store
        .add_document(&pdf(&root, "second.pdf", &["b"]))
        .expect("a readable pdf");

    let ids: Vec<String> = store
        .books()
        .expect("a library")
        .into_iter()
        .map(|book| book.id)
        .collect();

    assert_eq!(ids, vec![second.id, first.id]);
}

#[test]
fn removing_a_book_takes_its_highlights_conversations_and_bytes() {
    let (store, root) = library("removes");
    let book = store
        .add_document(&pdf(&root, "book.pdf", &["a"]))
        .expect("a readable pdf");

    let highlight = store
        .add_highlight(&book.id, highlight_of("a", 1))
        .expect("a stored book");
    store
        .add_message(&highlight.id, Role::User, "これは?", &[])
        .expect("a stored highlight");

    let path = store.document_path(&book);
    store.remove_book(&book.id).expect("a stored book");

    assert!(store.book(&book.id).expect("a query").is_none());
    assert!(store.highlights(&book.id).expect("a query").is_empty());
    assert!(store.messages(&highlight.id).expect("a query").is_empty());
    assert!(!path.exists());
}

#[test]
fn removing_a_book_that_is_not_there_says_so() {
    let (store, _root) = library("removes-missing");

    let error = store.remove_book("nope").unwrap_err();
    assert!(matches!(error, StoreError::NoSuchBook(_)), "{error}");
}

#[test]
fn a_place_is_saved_and_read_back() {
    let (store, root) = library("place");
    let book = store
        .add_document(&pdf(&root, "book.pdf", &["a", "b", "c"]))
        .expect("a readable pdf");
    let highlight = store
        .add_highlight(&book.id, highlight_of("b", 2))
        .expect("a stored book");

    store
        .save_reading_state(
            &book.id,
            &ReadingState {
                page: 3,
                highlight_id: Some(highlight.id.clone()),
                outline_open: Some(false),
                chat_panel_open: Some(true),
            },
        )
        .expect("a stored book");

    let reading = store
        .book(&book.id)
        .expect("a query")
        .unwrap()
        .reading
        .expect("a saved place");

    assert_eq!(reading.page, 3);
    assert_eq!(reading.highlight_id, Some(highlight.id));
    assert_eq!(reading.outline_open, Some(false));
    assert_eq!(reading.chat_panel_open, Some(true));
}

/// Saving a page must not fold away a panel the reader opened, so a panel left
/// unsaid keeps whatever was stored.
#[test]
fn saving_a_page_leaves_panels_nobody_mentioned_alone() {
    let (store, root) = library("panels");
    let book = store
        .add_document(&pdf(&root, "book.pdf", &["a", "b"]))
        .expect("a readable pdf");

    let opened = ReadingState {
        page: 1,
        highlight_id: None,
        outline_open: Some(true),
        chat_panel_open: Some(true),
    };
    store
        .save_reading_state(&book.id, &opened)
        .expect("a stored book");

    store
        .save_reading_state(
            &book.id,
            &ReadingState {
                page: 2,
                highlight_id: None,
                outline_open: None,
                chat_panel_open: None,
            },
        )
        .expect("a stored book");

    let reading = store
        .book(&book.id)
        .expect("a query")
        .unwrap()
        .reading
        .expect("a saved place");

    assert_eq!(reading.page, 2);
    assert_eq!(reading.outline_open, Some(true));
    assert_eq!(reading.chat_panel_open, Some(true));
}

#[test]
fn a_highlight_keeps_its_geometry() {
    let (store, root) = library("highlights");
    let book = store
        .add_document(&pdf(&root, "book.pdf", &["a"]))
        .expect("a readable pdf");

    let stored = store
        .add_highlight(&book.id, highlight_of("passage", 1))
        .expect("a stored book");
    let read_back = store.highlights(&book.id).expect("a query");

    assert_eq!(read_back, vec![stored]);
    assert_eq!(read_back[0].rects, vec![rect()]);
}

#[test]
fn a_highlight_on_a_book_that_is_not_there_says_so() {
    let (store, _root) = library("highlights-missing");

    let error = store
        .add_highlight("nope", highlight_of("passage", 1))
        .unwrap_err();
    assert!(matches!(error, StoreError::NoSuchBook(_)), "{error}");
}

#[test]
fn a_conversation_is_stored_in_the_order_it_happened() {
    let (store, root) = library("conversation");
    let book = store
        .add_document(&pdf(&root, "book.pdf", &["a"]))
        .expect("a readable pdf");
    let highlight = store
        .add_highlight(&book.id, highlight_of("passage", 1))
        .expect("a stored book");

    let citation = Citation {
        id: "1".to_owned(),
        kind: CitationKind::Pdf,
        text: "passage".to_owned(),
        page: Some(PageLocation::Found(1)),
        url: None,
    };

    store
        .add_message(&highlight.id, Role::User, "これは?", &[])
        .expect("a stored highlight");
    store
        .add_message(
            &highlight.id,
            Role::Assistant,
            "こうです[1]\n\n## Sources\n[1] 「passage」",
            std::slice::from_ref(&citation),
        )
        .expect("a stored highlight");

    let messages = store.messages(&highlight.id).expect("a query");

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, Role::User);
    assert!(messages[0].citations.is_empty());
    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[1].citations, vec![citation]);
}

#[test]
fn a_message_on_a_highlight_that_is_not_there_says_so() {
    let (store, _root) = library("message-missing");

    let error = store
        .add_message("nope", Role::User, "これは?", &[])
        .unwrap_err();
    assert!(matches!(error, StoreError::NoSuchHighlight(_)), "{error}");
}

/// A column that cannot be read must not make the rest of the row unreadable:
/// a highlight with no geometry is simply not drawn, and the passage it holds
/// is still worth showing in the list.
#[test]
fn a_broken_geometry_column_still_yields_the_highlight() {
    let (store, root) = library("broken-json");
    let book = store
        .add_document(&pdf(&root, "book.pdf", &["a"]))
        .expect("a readable pdf");
    let highlight = store
        .add_highlight(&book.id, highlight_of("passage", 1))
        .expect("a stored book");

    let connection = rusqlite::Connection::open(root.join("pedro.sqlite3")).expect("the database");
    connection
        .execute(
            "UPDATE highlights SET rects = '{broken' WHERE id = ?1",
            [&highlight.id],
        )
        .expect("a writable database");
    drop(connection);

    let read_back = store.highlights(&book.id).expect("a query");
    assert_eq!(read_back.len(), 1);
    assert_eq!(read_back[0].selected_text, "passage");
    assert!(read_back[0].rects.is_empty());
}

#[test]
fn a_file_that_is_not_a_pdf_is_refused_and_leaves_nothing_behind() {
    let (store, root) = library("not-a-pdf");
    let source = root.join("notes.pdf");
    std::fs::write(&source, b"this is not a pdf").expect("a writable file");

    assert!(store.add_document(&source).is_err());
    assert!(store.books().expect("a library").is_empty());

    let stored: Vec<_> = std::fs::read_dir(root.join("documents"))
        .expect("the documents directory")
        .collect();
    assert!(stored.is_empty(), "a rejected document was left behind");
}

#[test]
fn a_library_reopens_with_what_was_put_in_it() {
    let (store, root) = library("reopen");
    let book = store
        .add_document(&pdf(&root, "book.pdf", &["a"]))
        .expect("a readable pdf");
    drop(store);

    let reopened = Store::open(&root).expect("an existing library");
    assert_eq!(
        reopened.books().expect("a library")[0].file_name,
        book.file_name
    );
}
