//! The index against a real SQLite database.

use pedro_search::chunk::Chunk;
use pedro_search::index;
use rusqlite::Connection;

/// A database with the one table the index refers to, and the index itself.
fn library() -> Connection {
    // Before the connection is opened, which is when `vec0` is handed to it.
    index::prepare();

    let connection = Connection::open_in_memory().expect("an in-memory database");
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

/// Three passages, filed under vectors a test can reason about: the first
/// points the same way as the query, the second halfway, the third at right
/// angles to it.
fn with_vectors() -> Connection {
    let mut connection = library();
    index::index_book(
        &mut connection,
        "book-a",
        &[
            chunk(1, 0, "the same way"),
            chunk(2, 1, "halfway"),
            chunk(3, 2, "at right angles"),
        ],
    )
    .expect("indexed");

    index::create_vectors(&connection, 2).expect("a vector table");

    let ids: Vec<String> = connection
        .prepare("SELECT id FROM chunks ORDER BY ord")
        .expect("a query")
        .query_map([], |row| row.get(0))
        .expect("ids")
        .collect::<Result<_, _>>()
        .expect("ids");

    let halfway = std::f32::consts::FRAC_1_SQRT_2;
    index::index_vectors(
        &mut connection,
        &ids,
        &[vec![1., 0.], vec![halfway, halfway], vec![0., 1.]],
    )
    .expect("stored vectors");

    connection
}

/// The score has to be the cosine itself. `vec0` measures in L2 unless it is
/// asked otherwise, and for normalised vectors L2 is `sqrt(2 - 2·cos)` — which
/// for the halfway passage would read as 0.23 rather than 0.71, putting a good
/// match below any floor worth having.
#[test]
fn the_score_is_the_cosine() {
    let connection = with_vectors();
    let hits = index::search_similar(&connection, &[1., 0.], 10, -1.0).expect("a search");

    assert_eq!(hits.len(), 3);
    assert!((hits[0].score - 1.0).abs() < 0.01, "{}", hits[0].score);
    assert!(
        (hits[1].score - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01,
        "{}",
        hits[1].score
    );
    assert!(hits[2].score.abs() < 0.01, "{}", hits[2].score);
}

/// A nearest-neighbour search always has a nearest. The floor is what lets it
/// answer "nothing here is about that" instead.
#[test]
fn a_passage_further_than_the_floor_is_not_returned() {
    let connection = with_vectors();

    let close = index::search_similar(&connection, &[1., 0.], 10, 0.25).expect("a search");
    assert_eq!(close.len(), 2, "the right-angled passage came back");

    let nothing = index::search_similar(&connection, &[0., -1.], 10, 0.25).expect("a search");
    assert!(nothing.is_empty(), "{nothing:?}");
}

/// A library whose vectors were built before the metric was asked for holds
/// distances that mean something else. Reading them as cosines would rank the
/// book backwards, so the table is rebuilt rather than reused.
#[test]
fn a_table_built_for_another_metric_is_rebuilt() {
    let connection = library();
    connection
        .execute_batch(
            "CREATE VIRTUAL TABLE chunks_vec USING vec0(
                chunk_id TEXT PRIMARY KEY,
                embedding float[2]
            )",
        )
        .expect("a table measuring in L2");

    index::create_vectors(&connection, 2).expect("a vector table");

    let sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'chunks_vec'",
            [],
            |row| row.get(0),
        )
        .expect("a declaration");
    assert!(sql.contains("distance_metric=cosine"), "{sql}");
}

/// A question's grammar must not decide its answer.
///
/// Cut into character pairs, 「で動」 and 「の本」 are rarer than the words the
/// question is about, so a passage matching the grammar outranks the passage
/// that holds the subject. Cut into content words, the grammar is not in the
/// query at all.
#[test]
fn a_question_is_matched_on_what_it_is_about() {
    let mut connection = library();
    index::index_book(
        &mut connection,
        "book-a",
        &[
            chunk(
                1,
                0,
                "the runtime executes at the edge, close to the reader",
            ),
            chunk(
                2,
                1,
                "この章では別の話題を扱いますが、本を動かす話ではありません",
            ),
        ],
    )
    .expect("indexed");

    let about = index::search_about(&connection, "runtime が edge で動くという話はどの本?", 10)
        .expect("a search");

    assert_eq!(about.len(), 1, "{about:?}");
    assert_eq!(about[0].page_number, 1);
}

/// The search box keeps finding a string inside a longer word, which is what
/// the character pairs are for and what content words cannot do.
#[test]
fn a_search_still_finds_a_word_inside_a_word() {
    let mut connection = library();
    index::index_book(&mut connection, "book-a", &[chunk(3, 0, "東京駅から歩く")])
        .expect("indexed");

    assert_eq!(
        index::search(&connection, "京駅", 10)
            .expect("a search")
            .len(),
        1
    );
}

/// A question written entirely in hiragana has no content words, and says so
/// rather than matching everything.
#[test]
fn a_question_with_no_content_words_finds_nothing() {
    let mut connection = library();
    index::index_book(
        &mut connection,
        "book-a",
        &[chunk(1, 0, "エラトステネスのふるいで素数を生成する")],
    )
    .expect("indexed");

    let found = index::search_about(&connection, "これはどうですか", 10).expect("a search");
    assert!(found.is_empty(), "{found:?}");
}

/// An index built before there was a content column is rebuilt from the
/// passages, which keep their ids — and so keep the vectors that are keyed by
/// them, which cost forty seconds to make.
#[test]
fn an_index_without_content_words_is_rebuilt_from_its_passages() {
    let mut connection = library();
    index::index_book(
        &mut connection,
        "book-a",
        &[chunk(1, 0, "素数を生成するアルゴリズム")],
    )
    .expect("indexed");

    let ids_before: Vec<String> = connection
        .prepare("SELECT id FROM chunks ORDER BY ord")
        .expect("a query")
        .query_map([], |row| row.get(0))
        .expect("ids")
        .collect::<Result<_, _>>()
        .expect("ids");

    // The index as it stood before content words: one column.
    connection
        .execute_batch(
            "DROP TABLE chunks_fts;
             CREATE VIRTUAL TABLE chunks_fts USING fts5(
                 tokens, tokenize='unicode61 remove_diacritics 2'
             );",
        )
        .expect("the old index");

    let rebuilt = index::rebuild_text(&mut connection).expect("a rebuild");
    assert_eq!(rebuilt, 1);

    let ids_after: Vec<String> = connection
        .prepare("SELECT id FROM chunks ORDER BY ord")
        .expect("a query")
        .query_map([], |row| row.get(0))
        .expect("ids")
        .collect::<Result<_, _>>()
        .expect("ids");
    assert_eq!(ids_before, ids_after, "the passages were re-cut");

    // And both ways of asking work against the rebuilt index.
    assert_eq!(
        index::search(&connection, "素数", 10)
            .expect("a search")
            .len(),
        1
    );
    assert_eq!(
        index::search_about(&connection, "素数はどう生成する?", 10)
            .expect("a search")
            .len(),
        1
    );
}

/// The two cuts are genuinely different indexes, and each answers only for
/// itself.
///
/// 東京駅 is one content word and two character pairs. Searching for 京駅 finds
/// it through the pairs and must not find it through the words — if the column
/// filter were wrong, this would pass either way, and the pairs would be
/// answering questions the words were asked.
#[test]
fn each_cut_answers_only_for_itself() {
    let mut connection = library();
    index::index_book(&mut connection, "book-a", &[chunk(1, 0, "東京駅から歩く")])
        .expect("indexed");

    assert_eq!(
        index::search(&connection, "京駅", 10)
            .expect("a search")
            .len(),
        1,
        "the pairs lost a search inside a word"
    );
    assert!(
        index::search_about(&connection, "京駅", 10)
            .expect("a search")
            .is_empty(),
        "the words answered for the pairs"
    );

    // And the whole word is found either way.
    assert_eq!(
        index::search_about(&connection, "東京駅はどこ?", 10)
            .expect("a search")
            .len(),
        1
    );
}

/// Finding nothing and being unable to read the question look alike and mean
/// opposite things, so the caller is told which it is.
#[test]
fn a_question_says_whether_it_had_anything_to_ask_about() {
    assert!(index::asks_about_anything("素数はどうやって生成する?"));
    assert!(index::asks_about_anything("runtime が edge で動く?"));
    assert!(index::asks_about_anything("本と鍵"));

    assert!(!index::asks_about_anything("これはどうですか"));
    assert!(!index::asks_about_anything(""));
}
