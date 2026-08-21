//! The searchable copy of the books, in the library's own database.
//!
//! It is the same SQLite file the library lives in, so a hit can be joined back
//! to the book it came from and a deleted book takes its index with it. The
//! caller owns the connection: pedro has one, and search is not a good enough
//! reason to open a second.
//!
//! Adapted from the author's ellisii-toolkit `store-sqlite`, with permission.

use rusqlite::{Connection, OptionalExtension as _, params};

use crate::chunk::Chunk;
use crate::tokenize;

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("the search index failed: {0}")]
    Database(#[from] rusqlite::Error),
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
) -> Result<(), IndexError> {
    let transaction = connection.transaction()?;

    forget_within(&transaction, book_id)?;

    for chunk in chunks {
        // The tokens are what FTS5 indexes; the row it lands on is what the
        // chunk is found by afterwards.
        transaction.execute(
            "INSERT INTO chunks_fts (tokens) VALUES (?1)",
            [tokenize::for_index(&chunk.text)],
        )?;
        let fts_rowid = transaction.last_insert_rowid();

        transaction.execute(
            "INSERT INTO chunks (id, book_id, page_number, ord, text, fts_rowid)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                uuid::Uuid::new_v4().to_string(),
                book_id,
                chunk.page_number,
                chunk.ord,
                chunk.text,
                fts_rowid,
            ],
        )?;
    }

    transaction.commit()?;
    Ok(())
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

/// How many passages are indexed, for a settings screen that would rather say a
/// number than "yes".
pub fn count(connection: &Connection) -> Result<u64, IndexError> {
    let count: i64 = connection.query_row("SELECT count(*) FROM chunks", [], |row| row.get(0))?;

    Ok(count as u64)
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
    connection.execute("DELETE FROM chunks WHERE book_id = ?1", [book_id])?;

    Ok(())
}
