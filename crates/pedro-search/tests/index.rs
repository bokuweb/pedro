//! The index against a real SQLite database.

use pedro_search::chunk::Chunk;
use pedro_search::index;
use rusqlite::Connection;

/// A database with the one table the index refers to, and the index itself.
fn library() -> Connection {
    let mut connection = Connection::open_in_memory().expect("an in-memory database");
    connection.pragma_update(None, "foreign_keys", "ON").ok();
    connection
        .execute_batch("CREATE TABLE books (id TEXT PRIMARY KEY);")
        .expect("a books table");
    index::create(&connection).expect("the index");

    for id in ["book-a", "book-b"] {
        connection
            .execute("INSERT INTO books (id) VALUES (?1)", [id])
            .expect("a book");
    }

    connection
}

fn chunk(page_number: u32, ord: u32, text: &str) -> Chunk {
    Chunk {
        page_number,
        ord,
        text: text.to_owned(),
    }
}

#[test]
fn a_japanese_phrase_is_found_by_part_of_itself() {
    let mut connection = library();
    index::index_book(
        &mut connection,
        "book-a",
        &[chunk(7, 0, "エラトステネスのふるいで素数を生成する")],
    )
    .expect("an indexed book");

    let hits = index::search(&connection, "素数", 10).expect("a search");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].page_number, 7);
    assert_eq!(hits[0].book_id, "book-a");
}

/// The point of the bigrams: a search inside a word, with no space anywhere.
#[test]
fn a_search_inside_a_word_finds_it() {
    let mut connection = library();
    index::index_book(&mut connection, "book-a", &[chunk(1, 0, "東京駅前の広場")])
        .expect("an indexed book");

    assert_eq!(
        index::search(&connection, "京駅", 10)
            .expect("a search")
            .len(),
        1
    );
}

#[test]
fn latin_words_are_found_whatever_the_case() {
    let mut connection = library();
    index::index_book(
        &mut connection,
        "book-a",
        &[chunk(3, 0, "The Rabin-Miller primality test")],
    )
    .expect("an indexed book");

    assert_eq!(
        index::search(&connection, "rabin", 10)
            .expect("a search")
            .len(),
        1
    );
    assert_eq!(
        index::search(&connection, "PRIMALITY", 10)
            .expect("a search")
            .len(),
        1
    );
}

#[test]
fn a_search_that_matches_nothing_finds_nothing() {
    let mut connection = library();
    index::index_book(&mut connection, "book-a", &[chunk(1, 0, "エラトステネス")])
        .expect("an indexed book");

    assert!(
        index::search(&connection, "量子力学", 10)
            .expect("a search")
            .is_empty()
    );
    assert!(
        index::search(&connection, "   ", 10)
            .expect("a search")
            .is_empty()
    );
}

/// A hit names its book, so a library of many can be searched at once.
#[test]
fn a_search_spans_every_book() {
    let mut connection = library();
    index::index_book(&mut connection, "book-a", &[chunk(1, 0, "素数の話")]).expect("indexed");
    index::index_book(&mut connection, "book-b", &[chunk(2, 0, "素数と暗号")]).expect("indexed");

    let hits = index::search(&connection, "素数", 10).expect("a search");
    let books: Vec<&str> = hits.iter().map(|hit| hit.book_id.as_str()).collect();

    assert_eq!(hits.len(), 2);
    assert!(books.contains(&"book-a") && books.contains(&"book-b"));
}

/// Indexing a book again replaces what was there rather than doubling it.
#[test]
fn indexing_a_book_twice_does_not_double_it() {
    let mut connection = library();
    let chunks = [chunk(1, 0, "素数の話")];

    index::index_book(&mut connection, "book-a", &chunks).expect("indexed");
    index::index_book(&mut connection, "book-a", &chunks).expect("indexed again");

    assert_eq!(
        index::search(&connection, "素数", 10)
            .expect("a search")
            .len(),
        1
    );
    assert_eq!(index::count(&connection).expect("a count"), 1);
}

#[test]
fn forgetting_a_book_leaves_the_others() {
    let mut connection = library();
    index::index_book(&mut connection, "book-a", &[chunk(1, 0, "素数の話")]).expect("indexed");
    index::index_book(&mut connection, "book-b", &[chunk(1, 0, "素数と暗号")]).expect("indexed");

    index::forget(&connection, "book-a").expect("forgotten");

    let hits = index::search(&connection, "素数", 10).expect("a search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].book_id, "book-b");
}

/// Nothing of a deleted book may be left behind to be found.
#[test]
fn deleting_a_book_takes_its_index_with_it() {
    let mut connection = library();
    index::index_book(&mut connection, "book-a", &[chunk(1, 0, "素数の話")]).expect("indexed");

    connection
        .execute("DELETE FROM books WHERE id = 'book-a'", [])
        .expect("a deleted book");

    // The rows go by cascade; the tokens they named are cleaned up by the same
    // call the library makes when it removes a book.
    index::forget(&connection, "book-a").expect("forgotten");
    assert!(
        index::search(&connection, "素数", 10)
            .expect("a search")
            .is_empty()
    );
    assert_eq!(index::count(&connection).expect("a count"), 0);
}

#[test]
fn a_book_says_whether_it_has_been_indexed() {
    let mut connection = library();

    assert!(!index::is_indexed(&connection, "book-a").expect("a query"));
    index::index_book(&mut connection, "book-a", &[chunk(1, 0, "素数")]).expect("indexed");
    assert!(index::is_indexed(&connection, "book-a").expect("a query"));
}

/// The better match should come first, which is what bm25 is for.
#[test]
fn the_best_match_comes_first() {
    let mut connection = library();
    index::index_book(
        &mut connection,
        "book-a",
        &[
            chunk(1, 0, "この章では別の話題を扱います"),
            chunk(2, 1, "素数 素数 素数 について"),
        ],
    )
    .expect("indexed");

    let hits = index::search(&connection, "素数", 10).expect("a search");
    assert_eq!(hits[0].page_number, 2);
    assert!(hits[0].score >= hits.last().expect("a hit").score);
}
