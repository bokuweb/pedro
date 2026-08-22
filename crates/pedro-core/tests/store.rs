//! The library on disk, exercised against real files and a real pdfium.

use std::path::{Path, PathBuf};

use pedro_core::Conversation;
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
    let (mut store, root) = library("adds");
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
    let (mut store, root) = library("copies");
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
    let (mut store, root) = library("re-adds");
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
    let (mut store, root) = library("orders");

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
    let (mut store, root) = library("removes");
    let book = store
        .add_document(&pdf(&root, "book.pdf", &["a"]))
        .expect("a readable pdf");

    let highlight = store
        .add_highlight(&book.id, highlight_of("a", 1))
        .expect("a stored book");
    store
        .add_message(
            &Conversation::Highlight(highlight.id.clone()),
            Role::User,
            "これは?",
            &[],
        )
        .expect("a stored highlight");

    let path = store.document_path(&book);
    store.remove_book(&book.id).expect("a stored book");

    assert!(store.book(&book.id).expect("a query").is_none());
    assert!(store.highlights(&book.id).expect("a query").is_empty());
    assert!(
        store
            .messages(&Conversation::Highlight(highlight.id.clone()))
            .expect("a query")
            .is_empty()
    );
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
    let (mut store, root) = library("place");
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
    let (mut store, root) = library("panels");
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
    let (mut store, root) = library("highlights");
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
    let (mut store, root) = library("conversation");
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
        book: None,
        url: None,
    };

    store
        .add_message(
            &Conversation::Highlight(highlight.id.clone()),
            Role::User,
            "これは?",
            &[],
        )
        .expect("a stored highlight");
    store
        .add_message(
            &Conversation::Highlight(highlight.id.clone()),
            Role::Assistant,
            "こうです[1]\n\n## Sources\n[1] 「passage」",
            std::slice::from_ref(&citation),
        )
        .expect("a stored highlight");

    let messages = store
        .messages(&Conversation::Highlight(highlight.id.clone()))
        .expect("a query");

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
        .add_message(
            &Conversation::Highlight("nope".to_owned()),
            Role::User,
            "これは?",
            &[],
        )
        .unwrap_err();
    assert!(matches!(error, StoreError::NoSuchHighlight(_)), "{error}");
}

/// A column that cannot be read must not make the rest of the row unreadable:
/// a highlight with no geometry is simply not drawn, and the passage it holds
/// is still worth showing in the list.
#[test]
fn a_broken_geometry_column_still_yields_the_highlight() {
    let (mut store, root) = library("broken-json");
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
    let (mut store, root) = library("not-a-pdf");
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
    let (mut store, root) = library("reopen");
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

/// A book stored before pedro could read its kind of bookmark has an empty
/// outline against a document that has one. Opening it is what fixes that, and
/// the fix has to survive the window being closed.
#[test]
fn an_outline_can_be_filled_in_later() {
    let (mut store, root) = library("outline");
    let book = store
        .add_document(&pdf(&root, "book.pdf", &["a", "b", "c"]))
        .expect("a readable pdf");
    assert!(book.outline.is_empty(), "the fixture has no bookmarks");

    let chapters = vec![
        pedro_pdf::OutlineItem {
            title: "One".to_owned(),
            page_number: 1,
        },
        pedro_pdf::OutlineItem {
            title: "Two".to_owned(),
            page_number: 3,
        },
    ];
    store
        .set_outline(&book.id, &chapters)
        .expect("a stored book");

    let reopened = Store::open(&root).expect("an existing library");
    assert_eq!(
        reopened.book(&book.id).expect("a query").unwrap().outline,
        chapters
    );
}

#[test]
fn an_outline_for_a_book_that_is_not_there_says_so() {
    let (store, _root) = library("outline-missing");

    let error = store.set_outline("nope", &[]).unwrap_err();
    assert!(matches!(error, StoreError::NoSuchBook(_)), "{error}");
}

/// Adding a book makes it searchable, without opening it.
#[test]
fn a_new_book_can_be_searched_at_once() {
    let (mut store, root) = library("search");
    let book = store
        .add_document(&pdf(
            &root,
            "book.pdf",
            &["preface", "the runtime runs at the edge"],
        ))
        .expect("a readable pdf");

    let hits = store.search("runtime").expect("a search");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].book_id, book.id);
    assert_eq!(hits[0].page_number, 2);
}

#[test]
fn removing_a_book_removes_what_was_indexed_from_it() {
    let (mut store, root) = library("search-removed");
    let book = store
        .add_document(&pdf(&root, "book.pdf", &["the runtime"]))
        .expect("a readable pdf");

    store.remove_book(&book.id).expect("a stored book");

    assert!(store.search("runtime").expect("a search").is_empty());
}

/// Books added before there was an index have to get one.
#[test]
fn books_stored_without_an_index_are_indexed_later() {
    let (mut store, root) = library("search-backfill");
    let book = store
        .add_document(&pdf(&root, "book.pdf", &["the runtime"]))
        .expect("a readable pdf");

    // As though it had been added by a pedro that could not search.
    pedro_search::index::forget_for_test(store.connection(), &book.id).expect("forgotten");
    assert!(store.search("runtime").expect("a search").is_empty());

    assert_eq!(store.index_missing().expect("a backfill"), 1);
    assert_eq!(store.search("runtime").expect("a search").len(), 1);
    assert_eq!(store.index_missing().expect("a second backfill"), 0);
}

/// A shelf holds books, and says how many without being asked twice.
#[test]
fn a_shelf_counts_the_books_on_it() {
    let (mut store, root) = library("shelf-count");
    let shelf = store.create_folder("あとで読む").expect("a shelf");
    assert_eq!(shelf.book_count, 0);

    let one = store
        .add_document(&pdf(&root, "one.pdf", &["まえがき", "本文"]))
        .expect("a readable pdf");
    let two = store
        .add_document(&pdf(&root, "two.pdf", &["preface", "body"]))
        .expect("a readable pdf");

    store.move_book(&one.id, Some(&shelf.id)).expect("a move");
    store.move_book(&two.id, Some(&shelf.id)).expect("a move");

    let shelves = store.folders().expect("a query");
    assert_eq!(shelves.len(), 1);
    assert_eq!(shelves[0].book_count, 2);

    let on_it: Vec<String> = store
        .books_in(&shelf.id)
        .expect("a query")
        .into_iter()
        .map(|book| book.file_name)
        .collect();
    assert_eq!(on_it.len(), 2);
    assert!(on_it.contains(&"one.pdf".to_owned()));

    // And the book knows where it is, which is what the sidebar draws from.
    let book = store.book(&one.id).expect("a query").expect("the book");
    assert_eq!(book.folder_id, Some(shelf.id));
}

/// A book is on one shelf at a time, so moving it is a move and not a copy.
#[test]
fn moving_a_book_takes_it_off_the_shelf_it_was_on() {
    let (mut store, root) = library("shelf-move");
    let from = store.create_folder("読みかけ").expect("a shelf");
    let to = store.create_folder("読み終わった").expect("a shelf");
    let book = store
        .add_document(&pdf(&root, "one.pdf", &["まえがき", "本文"]))
        .expect("a readable pdf");

    store.move_book(&book.id, Some(&from.id)).expect("a move");
    store.move_book(&book.id, Some(&to.id)).expect("a move");

    assert!(store.books_in(&from.id).expect("a query").is_empty());
    assert_eq!(store.books_in(&to.id).expect("a query").len(), 1);

    // And off every shelf, which is where a book starts.
    store.move_book(&book.id, None).expect("a move");
    assert!(store.books_in(&to.id).expect("a query").is_empty());
    assert_eq!(
        store
            .book(&book.id)
            .expect("a query")
            .expect("it")
            .folder_id,
        None
    );
}

/// A shelf is an arrangement of the library, not a part of it: throwing the
/// arrangement away must not throw the books away with it.
#[test]
fn deleting_a_shelf_keeps_its_books_and_drops_its_conversation() {
    let (mut store, root) = library("shelf-delete");
    let shelf = store.create_folder("暗号").expect("a shelf");
    let book = store
        .add_document(&pdf(&root, "one.pdf", &["まえがき", "本文"]))
        .expect("a readable pdf");
    store.move_book(&book.id, Some(&shelf.id)).expect("a move");

    let about = Conversation::Folder(shelf.id.clone());
    store
        .add_message(&about, Role::User, "この2冊はどう違う?", &[])
        .expect("a stored message");
    assert_eq!(store.messages(&about).expect("a query").len(), 1);

    store.remove_folder(&shelf.id).expect("a removal");

    assert!(store.folders().expect("a query").is_empty());
    assert!(store.messages(&about).expect("a query").is_empty());

    // The book is still in the library, and on no shelf.
    let book = store.book(&book.id).expect("a query").expect("the book");
    assert_eq!(book.folder_id, None);
}

/// Two conversations, one about a passage and one about a shelf, do not run
/// into each other — which is the whole reason for the check constraint.
#[test]
fn a_shelf_conversation_is_not_a_highlight_conversation() {
    let (mut store, root) = library("shelf-conversations");
    let shelf = store.create_folder("暗号").expect("a shelf");
    let book = store
        .add_document(&pdf(&root, "one.pdf", &["まえがき", "本文"]))
        .expect("a readable pdf");
    store.move_book(&book.id, Some(&shelf.id)).expect("a move");

    let highlight = store
        .add_highlight(&book.id, highlight_of("本文", 2))
        .expect("a stored highlight");

    let on_shelf = Conversation::Folder(shelf.id.clone());
    let on_passage = Conversation::Highlight(highlight.id.clone());

    store
        .add_message(&on_shelf, Role::User, "棚について", &[])
        .expect("a stored message");
    store
        .add_message(&on_passage, Role::User, "この一節について", &[])
        .expect("a stored message");

    let shelf_turns = store.messages(&on_shelf).expect("a query");
    assert_eq!(shelf_turns.len(), 1);
    assert_eq!(shelf_turns[0].content, "棚について");
    assert_eq!(shelf_turns[0].about, on_shelf);

    let passage_turns = store.messages(&on_passage).expect("a query");
    assert_eq!(passage_turns.len(), 1);
    assert_eq!(passage_turns[0].content, "この一節について");
}

/// A conversation cannot hang off a shelf that is not there.
#[test]
fn a_message_about_a_shelf_that_is_not_there_says_so() {
    let (store, _root) = library("shelf-missing");
    let error = store
        .add_message(
            &Conversation::Folder("nope".to_owned()),
            Role::User,
            "これは?",
            &[],
        )
        .expect_err("no such shelf");

    assert!(matches!(error, StoreError::NoSuchFolder(id) if id == "nope"));
}

/// A library written before shelves existed still opens, and still holds every
/// conversation it held.
///
/// The shelf migration cannot alter `chat_messages` in place — SQLite will not
/// drop a NOT NULL — so it rebuilds the table and carries the rows across. A
/// reader's conversations are the part of this file that cannot be rebuilt from
/// their PDFs, which makes this the one migration worth a test of its own.
#[test]
fn a_library_from_before_shelves_keeps_its_conversations() {
    let root = std::env::temp_dir().join("pedro-store-old-version");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a writable directory");

    // Version 2, written by hand: the schema as it stood before shelves.
    let old = rusqlite::Connection::open(root.join("pedro.sqlite3")).expect("a database");
    old.execute_batch(
        r#"
        CREATE TABLE books (
            id TEXT PRIMARY KEY, file_name TEXT NOT NULL, file_hash TEXT NOT NULL UNIQUE,
            full_text TEXT NOT NULL, page_count INTEGER NOT NULL, outline TEXT,
            created_at TEXT NOT NULL, updated_at TEXT NOT NULL, last_read_page INTEGER,
            last_read_highlight_id TEXT, last_read_outline_open INTEGER,
            last_read_chat_panel_open INTEGER
        );
        CREATE TABLE highlights (
            id TEXT PRIMARY KEY, book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            selected_text TEXT NOT NULL, page_number INTEGER NOT NULL, rects TEXT NOT NULL,
            color TEXT NOT NULL, created_at TEXT NOT NULL
        );
        CREATE INDEX highlights_by_book ON highlights(book_id);
        CREATE TABLE chat_messages (
            id TEXT PRIMARY KEY,
            highlight_id TEXT NOT NULL REFERENCES highlights(id) ON DELETE CASCADE,
            role TEXT NOT NULL, content TEXT NOT NULL, citations TEXT, created_at TEXT NOT NULL
        );
        CREATE INDEX messages_by_highlight ON chat_messages(highlight_id);

        INSERT INTO books VALUES
            ('b1', 'one.pdf', 'hash-1', 'preface', 1, NULL,
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL, NULL, NULL, NULL);
        INSERT INTO highlights VALUES
            ('h1', 'b1', 'preface', 1, '[]', 'yellow', '2026-01-01T00:00:00Z');
        INSERT INTO chat_messages VALUES
            ('m1', 'h1', 'user', 'これは?', NULL, '2026-01-01T00:00:00Z'),
            ('m2', 'h1', 'assistant', 'こうです', NULL, '2026-01-01T00:00:01Z');

        PRAGMA user_version = 2;
        "#,
    )
    .expect("the old schema");
    drop(old);

    let store = Store::open(&root).expect("a library that migrates");

    let messages = store
        .messages(&Conversation::Highlight("h1".to_owned()))
        .expect("a query");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "これは?");
    assert_eq!(messages[1].content, "こうです");

    // And the new half works on the migrated file.
    let shelf = store.create_folder("あとで").expect("a shelf");
    store.move_book("b1", Some(&shelf.id)).expect("a move");
    assert_eq!(store.books_in(&shelf.id).expect("a query").len(), 1);

    let about = Conversation::Folder(shelf.id.clone());
    store
        .add_message(&about, Role::User, "棚について", &[])
        .expect("a stored message");
    assert_eq!(store.messages(&about).expect("a query").len(), 1);
}
