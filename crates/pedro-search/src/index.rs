//! The searchable copy of the books, in the library's own database.
//!
//! It is the same SQLite file the library lives in, so a hit can be joined back
//! to the book it came from and a deleted book takes its index with it. The
//! caller owns the connection: pedro has one, and search is not a good enough
//! reason to open a second.
//!
//! Adapted from the author's ellisii-toolkit `store-sqlite`, with permission.

use std::sync::Once;

use rusqlite::{Connection, OptionalExtension as _, params};

use crate::chunk::Chunk;
use crate::tokenize;

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("the search index failed: {0}")]
    Database(#[from] rusqlite::Error),
}

/// Loads sqlite-vec into every connection SQLite opens from here on.
///
/// It is an extension rather than part of SQLite, and registering it as an
/// auto-extension is what makes `vec0` tables exist without every connection
/// having to ask. Once per process; a second registration is an error.
fn ensure_vectors_available() {
    static ONCE: Once = Once::new();

    ONCE.call_once(|| unsafe {
        type Register = unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *mut std::os::raw::c_char,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> std::os::raw::c_int;

        let register: Register = std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());
        rusqlite::ffi::sqlite3_auto_extension(Some(register));
    });
}

/// What the vector table is asked to measure in. See [`create_vectors`].
const COSINE: &str = "distance_metric=cosine";

/// Call before opening the connection the index will live in.
pub fn prepare() {
    ensure_vectors_available();
}

/// One passage the search found, and where it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub book_id: String,
    pub page_number: u32,
    pub text: String,
    /// Higher is better, whatever the search was.
    pub score: f32,
}

/// Creates the tables if they are not there.
///
/// `chunks_fts` holds the tokens rather than the text: FTS5 is given words it
/// can see, and the text it came from stays in `chunks` where it is read from.
pub fn create(connection: &Connection) -> Result<(), IndexError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS chunks (
            id TEXT PRIMARY KEY,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            page_number INTEGER NOT NULL,
            ord INTEGER NOT NULL,
            text TEXT NOT NULL,
            fts_rowid INTEGER
        );

        CREATE INDEX IF NOT EXISTS chunks_by_book ON chunks(book_id);

        CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
            tokens,
            tokenize='unicode61 remove_diacritics 2'
        );",
    )?;

    Ok(())
}

/// Whether a book has been indexed.
pub fn is_indexed(connection: &Connection, book_id: &str) -> Result<bool, IndexError> {
    let indexed: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM chunks WHERE book_id = ?1 LIMIT 1",
            [book_id],
            |row| row.get(0),
        )
        .optional()?;

    Ok(indexed.is_some())
}

/// Replaces everything indexed for one book.
///
/// One transaction: a book that is half indexed would answer searches with half
/// its pages and look as though the rest were not in it.
pub fn index_book(
    connection: &mut Connection,
    book_id: &str,
    chunks: &[Chunk],
) -> Result<Vec<String>, IndexError> {
    let transaction = connection.transaction()?;
    let mut ids = Vec::with_capacity(chunks.len());

    forget_within(&transaction, book_id)?;

    for chunk in chunks {
        // The tokens are what FTS5 indexes; the row it lands on is what the
        // chunk is found by afterwards.
        transaction.execute(
            "INSERT INTO chunks_fts (tokens) VALUES (?1)",
            [tokenize::for_index(&chunk.text)],
        )?;
        let fts_rowid = transaction.last_insert_rowid();

        let id = uuid::Uuid::new_v4().to_string();
        transaction.execute(
            "INSERT INTO chunks (id, book_id, page_number, ord, text, fts_rowid)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                book_id,
                chunk.page_number,
                chunk.ord,
                chunk.text,
                fts_rowid,
            ],
        )?;
        ids.push(id);
    }

    transaction.commit()?;
    Ok(ids)
}

/// Removes one book from the index.
pub fn forget(connection: &Connection, book_id: &str) -> Result<(), IndexError> {
    forget_within(connection, book_id)
}

/// Removes a book from the index while leaving the book, so that a test can
/// produce the state a library written before there was an index is in.
#[doc(hidden)]
pub fn forget_for_test(connection: &Connection, book_id: &str) -> Result<(), IndexError> {
    forget_within(connection, book_id)
}

/// The passages matching `query`, best first.
///
/// The query is segmented the same way the text was, because a search for
/// 京駅 only finds 東京駅 if both were cut into the same pairs.
pub fn search(connection: &Connection, query: &str, limit: usize) -> Result<Vec<Hit>, IndexError> {
    let terms = tokenize::for_query(query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }

    let mut statement = connection.prepare(
        "SELECT c.book_id, c.page_number, c.text, bm25(chunks_fts) AS rank
         FROM chunks_fts
         JOIN chunks c ON c.fts_rowid = chunks_fts.rowid
         WHERE chunks_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;

    let hits = statement
        .query_map(params![terms, limit as i64], |row| {
            Ok(Hit {
                book_id: row.get(0)?,
                page_number: row.get(1)?,
                text: row.get(2)?,
                // bm25 counts down from zero — the better the match, the more
                // negative — and everything else here counts up.
                score: -row.get::<_, f64>(3)? as f32,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(hits)
}

/// Creates the table vectors live in, if the model that fills it is present.
///
/// The width is the model's, and a model of a different width cannot use a
/// table built for the old one — so the table is rebuilt, which costs the
/// vectors and not the passages: re-embedding is the cheap half.
///
/// The metric is asked for rather than taken: `vec0` measures in L2 by
/// default, and for the normalised vectors stored here that is
/// `sqrt(2 - 2·cos)` — a real distance, but not one a similarity can be read
/// off by subtracting from one. Asking for cosine makes the distance
/// `1 - cos`, so the score really is the cosine the caller thinks it is. A
/// table built before this was asked for is rebuilt for the same reason a
/// narrower one is.
pub fn create_vectors(connection: &Connection, dimensions: usize) -> Result<(), IndexError> {
    let stored = declaration(connection);

    if let Some(sql) = &stored {
        let width = width_in(sql);
        let cosine = sql.contains(COSINE);

        if width != Some(dimensions) || !cosine {
            tracing::warn!(
                ?width,
                dimensions,
                cosine,
                "the vector table does not match the model; it will be built again"
            );
            connection.execute_batch("DROP TABLE IF EXISTS chunks_vec")?;
        }
    }

    connection.execute(
        &format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_vec USING vec0(
                chunk_id TEXT PRIMARY KEY,
                embedding float[{dimensions}] {COSINE}
            )"
        ),
        [],
    )?;

    Ok(())
}

/// Whether the vectors are there to be searched.
pub fn has_vectors(connection: &Connection) -> bool {
    vector_width(connection).is_some()
}

/// Files the vectors of passages already indexed.
pub fn index_vectors(
    connection: &mut Connection,
    ids: &[String],
    vectors: &[Vec<f32>],
) -> Result<(), IndexError> {
    let transaction = connection.transaction()?;

    for (id, vector) in ids.iter().zip(vectors) {
        transaction.execute(
            "INSERT OR REPLACE INTO chunks_vec (chunk_id, embedding) VALUES (?1, ?2)",
            params![id, bytemuck::cast_slice::<f32, u8>(vector)],
        )?;
    }

    transaction.commit()?;
    Ok(())
}

/// The passages nearest `vector`, nearest first, none further than `floor`.
///
/// The floor is what keeps this from answering every query. Keyword search
/// says nothing when the words are absent; a nearest-neighbour search always
/// has a nearest, so without a floor a question about primes attached the six
/// least-unrelated pages of a book that never mentions them.
pub fn search_similar(
    connection: &Connection,
    vector: &[f32],
    limit: usize,
    floor: f32,
) -> Result<Vec<Hit>, IndexError> {
    let mut statement = connection.prepare(
        "SELECT c.book_id, c.page_number, c.text, v.distance
         FROM chunks_vec v
         JOIN chunks c ON c.id = v.chunk_id
         WHERE v.embedding MATCH ?1 AND k = ?2
         ORDER BY v.distance",
    )?;

    let hits = statement
        .query_map(
            params![bytemuck::cast_slice::<f32, u8>(vector), limit as i64],
            |row| {
                Ok(Hit {
                    book_id: row.get(0)?,
                    page_number: row.get(1)?,
                    text: row.get(2)?,
                    // The table measures in cosine, so this is one minus the
                    // cosine of the angle between the two.
                    score: 1.0 - row.get::<_, f64>(3)? as f32,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;

    // Filtered here rather than in the query: `vec0` wants `k` rows and a
    // `distance` bound in the same MATCH is not something it takes.
    Ok(hits.into_iter().filter(|hit| hit.score >= floor).collect())
}

/// Whether the book's passages have vectors as well as words.
///
/// A book indexed before the model was downloaded has one and not the other,
/// and nothing else distinguishes it from a book that has both.
pub fn has_vectors_for(connection: &Connection, book_id: &str) -> Result<bool, IndexError> {
    if !has_vectors(connection) {
        return Ok(false);
    }

    let count: i64 = connection.query_row(
        "SELECT count(*) FROM chunks_vec v
         WHERE v.chunk_id IN (SELECT id FROM chunks WHERE book_id = ?1)",
        [book_id],
        |row| row.get(0),
    )?;

    Ok(count > 0)
}

/// How many passages are indexed, for a settings screen that would rather say a
/// number than "yes".
pub fn count(connection: &Connection) -> Result<u64, IndexError> {
    let count: i64 = connection.query_row("SELECT count(*) FROM chunks", [], |row| row.get(0))?;

    Ok(count as u64)
}

/// How wide the stored vectors are, or `None` when there are none.
fn vector_width(connection: &Connection) -> Option<usize> {
    width_in(&declaration(connection)?)
}

/// How the vector table was built, or `None` when there is no vector table.
fn declaration(connection: &Connection) -> Option<String> {
    connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'chunks_vec'",
            [],
            |row| row.get(0),
        )
        .ok()
}

/// "… embedding float[1024] distance_metric=cosine)"
fn width_in(sql: &str) -> Option<usize> {
    let (_, rest) = sql.split_once("float[")?;
    let (width, _) = rest.split_once(']')?;

    width.parse().ok()
}

fn forget_within(connection: &Connection, book_id: &str) -> Result<(), IndexError> {
    // The FTS rows go first: they are found through the chunks that name them,
    // and deleting those first would leave the tokens behind with nothing
    // pointing at them.
    connection.execute(
        "DELETE FROM chunks_fts WHERE rowid IN
         (SELECT fts_rowid FROM chunks WHERE book_id = ?1 AND fts_rowid IS NOT NULL)",
        [book_id],
    )?;
    // The vector table is not reached by the foreign key the chunks have, so
    // its rows have to be named before the chunks that name them go.
    if has_vectors(connection) {
        connection.execute(
            "DELETE FROM chunks_vec WHERE chunk_id IN
             (SELECT id FROM chunks WHERE book_id = ?1)",
            [book_id],
        )?;
    }

    connection.execute("DELETE FROM chunks WHERE book_id = ?1", [book_id])?;

    Ok(())
}
