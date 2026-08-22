//! The library on disk: a SQLite database and the PDFs beside it.
//!
//! This is what chatbook keeps in D1 and R2. The schema follows its migrations
//! closely enough that they stay readable as documentation of this one, with
//! two deliberate differences:
//!
//! - highlight geometry is stored as fractions of the page rather than as
//!   pixels plus the width they were measured at, so nothing has to be
//!   rescaled and nothing can disagree;
//! - what an answer cost is not recorded. chatbook's reader pays per token; a
//!   CLI on a subscription reports no comparable number, and a column of
//!   zeroes says less than no column at all.

use std::path::{Path, PathBuf};

use pedro_pdf::{Document, OutlineItem, PdfError};
use pedro_search::index;
use rusqlite::{Connection, OptionalExtension as _, Row, params};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::citation::Citation;
use crate::model::{
    Book, ChatMessage, Conversation, DEFAULT_HIGHLIGHT_COLOR, Folder, Highlight, NewHighlight,
    ReadingState, Role,
};

/// The name of the database inside the library directory.
const DATABASE: &str = "pedro.sqlite3";

/// Where the stored copies of the PDFs live, inside the library directory.
const DOCUMENTS: &str = "documents";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the library database failed: {0}")]
    Database(#[from] rusqlite::Error),

    #[error(transparent)]
    Index(#[from] pedro_search::IndexError),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("the document could not be read: {0}")]
    Pdf(#[from] PdfError),

    #[error("this platform has no application data directory")]
    NoDataDirectory,

    #[error("no book with id {0}")]
    NoSuchBook(String),

    #[error("no highlight with id {0}")]
    NoSuchHighlight(String),

    #[error("no folder with id {0}")]
    NoSuchFolder(String),
}

/// The reader's books, highlights and conversations.
pub struct Store {
    root: PathBuf,
    connection: Connection,
    /// The model that turns a passage into a vector, when it has been fetched.
    ///
    /// Optional on purpose: everything works without it, and searching by
    /// meaning is what it adds. See `scripts/fetch-embedding.sh`.
    embedder: Option<pedro_search::Embedder>,
}

/// How many passages a search returns before anyone asks for fewer.
const SEARCH_LIMIT: usize = 40;

/// How alike a passage has to be before it counts as being about the query.
///
/// Measured rather than picked: with this model a question and a passage that
/// answers it score between 0.43 and 0.81, and a question against text on
/// another subject scores 0.06 and below
/// (`cargo run -p pedro-search --example similarity`). The floor sits in the
/// gap, far enough from both ends that a near miss on either side of it is
/// still the right call.
const RELATED: f32 = 0.25;

impl Store {
    /// Opens the library under `root`, creating it if it is not there yet.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        std::fs::create_dir_all(root.join(DOCUMENTS))?;

        // Before the connection is opened: the vector index is an extension,
        // and a connection only has it if it was registered first.
        index::prepare();

        let connection = Connection::open(root.join(DATABASE))?;
        // Deleting a book has to take its highlights and their conversations
        // with it, and SQLite only honours that when foreign keys are on — the
        // default is off, per connection.
        connection.pragma_update(None, "foreign_keys", "ON")?;
        // A write must not stop the reader turning pages.
        let _: String = connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;

        migrate(&connection)?;

        let embedder = pedro_search::Embedder::find();
        if let Some(embedder) = &embedder {
            index::create_vectors(&connection, embedder.dimensions())?;
        }

        Ok(Self {
            root,
            connection,
            embedder,
        })
    }

    /// Opens the library where the platform keeps application data.
    pub fn open_default() -> Result<Self, StoreError> {
        Self::open(default_root()?)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The connection, for tests that have to put the library into a state it
    /// would otherwise take an older version of pedro to produce.
    #[doc(hidden)]
    pub fn connection(&self) -> &rusqlite::Connection {
        &self.connection
    }

    /// Where a book's bytes are stored.
    pub fn document_path(&self, book: &Book) -> PathBuf {
        self.path_for_hash(&book.file_hash)
    }

    /// Adds a PDF to the library, or hands back the book it already is.
    ///
    /// Identity is the file's content. Adding the same bytes again keeps the
    /// existing book — with its highlights, its conversations and its place —
    /// and only takes the new file's name, which is how re-adding a book under
    /// a better name renames it.
    ///
    /// Reads the whole document, so this belongs on a background thread.
    pub fn add_document(&mut self, source: &Path) -> Result<Book, StoreError> {
        let bytes = std::fs::read(source)?;
        let file_hash = hash_of(&bytes);
        let file_name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| file_hash.clone());

        if let Some(id) = self.id_with_hash(&file_hash)? {
            self.connection.execute(
                "UPDATE books SET file_name = ?1, updated_at = ?2 WHERE id = ?3",
                params![file_name, now(), id],
            )?;
            return self.book(&id)?.ok_or(StoreError::NoSuchBook(id));
        }

        let path = self.path_for_hash(&file_hash);
        std::fs::write(&path, &bytes)?;

        // A file pdfium will not open must not be left behind: the hash would
        // then name a stored document that no row knows about, and the next
        // attempt to add it would skip the write and fail the same way.
        let extracted = extract(&path).inspect_err(|_| {
            let _ = std::fs::remove_file(&path);
        })?;

        let id = new_id();
        let timestamp = now();
        self.connection.execute(
            "INSERT INTO books (id, file_name, file_hash, full_text, page_count, outline, \
             created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                id,
                file_name,
                file_hash,
                extracted.full_text,
                extracted.page_count,
                write_outline(&extracted.outline),
                timestamp,
            ],
        )?;

        // Searchable before it is opened: a reader who adds a book and asks
        // where something is in it should not have to open it first.
        self.index_book(&id, &extracted.full_text)?;

        self.book(&id)?.ok_or(StoreError::NoSuchBook(id))
    }

    /// Cuts a book into passages and puts them in the search index.
    ///
    /// Reading a book is what costs; this is a few thousand rows and one
    /// transaction, so it happens on the same thread that just read the book.
    pub fn index_book(&mut self, book_id: &str, full_text: &str) -> Result<(), StoreError> {
        let chunks = pedro_search::chunk::split(full_text, crate::excerpt::PAGE_DELIMITER);
        let ids = index::index_book(&mut self.connection, book_id, &chunks)?;

        // The words are indexed whatever happens; the meanings only when the
        // model to read them with is present.
        let embedded = match &self.embedder {
            Some(embedder) => {
                let texts: Vec<String> = chunks.iter().map(|chunk| chunk.text.clone()).collect();
                match embedder.embed_all(&texts) {
                    Ok(vectors) => {
                        index::index_vectors(&mut self.connection, &ids, &vectors)?;
                        true
                    }
                    Err(err) => {
                        tracing::warn!(?err, book_id, "could not embed a book");
                        false
                    }
                }
            }
            None => false,
        };

        tracing::info!(book_id, passages = chunks.len(), embedded, "indexed a book");
        Ok(())
    }

    /// Indexes the books added before there was an index to put them in.
    ///
    /// Returns how many it did. Called at startup, where finding nothing to do
    /// is the usual answer and costs a query or two per book.
    ///
    /// A book counts as missing when its words are not indexed, and also when
    /// its words are but its meanings are not — which is every book in a
    /// library that was read before the model was downloaded. Without the
    /// second test those books would keep their keyword search and never gain
    /// the other kind, with nothing on screen to say why.
    pub fn index_missing(&mut self) -> Result<usize, StoreError> {
        let books = self.books()?;
        let embedder = self.embedder.is_some();
        let mut done = 0;

        for book in books {
            let words = index::is_indexed(&self.connection, &book.id)?;
            let meanings = index::has_vectors_for(&self.connection, &book.id)?;

            if words && (meanings || !embedder) {
                continue;
            }

            let full_text = self.full_text(&book.id)?;
            self.index_book(&book.id, &full_text)?;
            done += 1;
        }

        Ok(done)
    }

    /// The passages matching `query`, across every book, best first.
    ///
    /// Both ways of looking when there is a model to look the second way with:
    /// the words the reader typed, and what those words mean. Their scores are
    /// not comparable, so the two rankings are fused by position rather than
    /// by score.
    pub fn search(&self, query: &str) -> Result<Vec<pedro_search::Hit>, StoreError> {
        let words = index::search(&self.connection, query, SEARCH_LIMIT)?;
        let meaning = self.by_meaning(query)?;

        if meaning.is_empty() {
            return Ok(words);
        }

        Ok(pedro_search::fuse::reciprocal_rank(
            &[words, meaning],
            SEARCH_LIMIT,
        ))
    }

    /// The passages of `books` that bear on `question`, best first.
    ///
    /// What a question is answered from beyond the pages the reader marked:
    /// the book is searched for the question itself, and what it finds is
    /// attached to the prompt.
    ///
    /// Held to a higher bar than the search box is. A search box that returns
    /// something loosely related costs the reader a glance, and they can see
    /// for themselves that it is not what they meant; the same passage in a
    /// prompt is indistinguishable, to whatever is reading it, from a passage
    /// that answers the question. So a passage joins the context only if it
    /// means something like the question — the vector floor — or actually
    /// holds the words in it that were worth typing. Turning up nothing is a
    /// fine answer here, and the marked pages are still sent.
    pub fn passages_for(
        &self,
        books: &[String],
        question: &str,
        limit: usize,
    ) -> Result<Vec<pedro_search::Hit>, StoreError> {
        let meaning = self.by_meaning(question)?;
        let known: std::collections::HashSet<&str> =
            meaning.iter().map(|hit| hit.text.as_str()).collect();

        let words = index::search(&self.connection, question, SEARCH_LIMIT)?
            .into_iter()
            .filter(|hit| !known.contains(hit.text.as_str()))
            .collect();

        Ok(
            pedro_search::fuse::reciprocal_rank(&[words, meaning], limit)
                .into_iter()
                .filter(|hit| books.contains(&hit.book_id))
                .take(limit)
                .collect(),
        )
    }

    /// The passages that mean something like `query`, or none when there is no
    /// model to judge that with.
    fn by_meaning(&self, query: &str) -> Result<Vec<pedro_search::Hit>, StoreError> {
        let Some(embedder) = &self.embedder else {
            return Ok(Vec::new());
        };
        if !index::has_vectors(&self.connection) {
            return Ok(Vec::new());
        }

        match embedder.embed(query) {
            Ok(vector) => Ok(index::search_similar(
                &self.connection,
                &vector,
                SEARCH_LIMIT,
                RELATED,
            )?),
            Err(err) => {
                tracing::warn!(?err, "could not embed the query");
                Ok(Vec::new())
            }
        }
    }

    /// Whether searching by meaning is available.
    pub fn can_search_by_meaning(&self) -> bool {
        self.embedder.is_some() && index::has_vectors(&self.connection)
    }

    /// Every book, most recently touched first.
    pub fn books(&self) -> Result<Vec<Book>, StoreError> {
        let mut statement = self.connection.prepare(&format!(
            "{BOOK_COLUMNS} ORDER BY updated_at DESC, rowid DESC"
        ))?;
        let books = statement
            .query_map([], read_book)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(books)
    }

    pub fn book(&self, id: &str) -> Result<Option<Book>, StoreError> {
        Ok(self
            .connection
            .query_row(&format!("{BOOK_COLUMNS} WHERE id = ?1"), [id], read_book)
            .optional()?)
    }

    /// The book's whole text, pages joined with the delimiter.
    ///
    /// Kept out of [`Book`] because it is the largest thing in the library and
    /// only two callers want it: cutting an excerpt, and finding the page a
    /// quotation came from.
    pub fn full_text(&self, book_id: &str) -> Result<String, StoreError> {
        self.connection
            .query_row(
                "SELECT full_text FROM books WHERE id = ?1",
                [book_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NoSuchBook(book_id.to_owned()))
    }

    /// Removes a book, its highlights, its conversations, its bytes and
    /// everything indexed from it.
    pub fn remove_book(&self, id: &str) -> Result<(), StoreError> {
        let Some(book) = self.book(id)? else {
            return Err(StoreError::NoSuchBook(id.to_owned()));
        };

        // Before the row goes: the index is found through it.
        index::forget(&self.connection, id)?;
        self.connection
            .execute("DELETE FROM books WHERE id = ?1", [id])?;

        // The row is what the library is; a file left behind is waste, not
        // corruption, so failing to remove it does not fail the deletion.
        if let Err(err) = std::fs::remove_file(self.document_path(&book)) {
            tracing::warn!(
                ?err,
                book = book.file_name,
                "could not remove the stored file"
            );
        }

        Ok(())
    }

    /// Records a book's table of contents.
    ///
    /// Books added before pedro could read a particular kind of bookmark have
    /// an empty outline stored against a document that has one. Reading it
    /// again when the book is opened is what fixes them, and it has to be
    /// written down or every question pays for the extraction again.
    pub fn set_outline(&self, book_id: &str, outline: &[OutlineItem]) -> Result<(), StoreError> {
        let changed = self.connection.execute(
            "UPDATE books SET outline = ?1 WHERE id = ?2",
            params![write_outline(outline), book_id],
        )?;

        match changed {
            0 => Err(StoreError::NoSuchBook(book_id.to_owned())),
            _ => Ok(()),
        }
    }

    /// Saves where the reader is. Panel states left as `None` keep whatever
    /// was stored, so saving a page does not fold away an opened panel.
    pub fn save_reading_state(
        &self,
        book_id: &str,
        state: &ReadingState,
    ) -> Result<(), StoreError> {
        let changed = self.connection.execute(
            "UPDATE books SET last_read_page = ?1, last_read_highlight_id = ?2, \
             last_read_outline_open = COALESCE(?3, last_read_outline_open), \
             last_read_chat_panel_open = COALESCE(?4, last_read_chat_panel_open), \
             updated_at = ?5 WHERE id = ?6",
            params![
                state.page,
                state.highlight_id,
                state.outline_open,
                state.chat_panel_open,
                now(),
                book_id,
            ],
        )?;

        if changed == 0 {
            return Err(StoreError::NoSuchBook(book_id.to_owned()));
        }

        Ok(())
    }

    /// Marks a passage, and returns it as stored.
    pub fn add_highlight(
        &self,
        book_id: &str,
        highlight: NewHighlight,
    ) -> Result<Highlight, StoreError> {
        if self.book(book_id)?.is_none() {
            return Err(StoreError::NoSuchBook(book_id.to_owned()));
        }

        let stored = Highlight {
            id: new_id(),
            book_id: book_id.to_owned(),
            selected_text: highlight.selected_text,
            page_number: highlight.page_number,
            rects: highlight.rects,
            color: DEFAULT_HIGHLIGHT_COLOR.to_owned(),
            created_at: OffsetDateTime::now_utc(),
        };

        self.connection.execute(
            "INSERT INTO highlights (id, book_id, selected_text, page_number, rects, color, \
             created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                stored.id,
                stored.book_id,
                stored.selected_text,
                stored.page_number,
                serde_json::to_string(&stored.rects).unwrap_or_else(|_| "[]".to_owned()),
                stored.color,
                format_time(stored.created_at),
            ],
        )?;

        Ok(stored)
    }

    /// The passages marked in a book, in the order they were marked.
    pub fn highlights(&self, book_id: &str) -> Result<Vec<Highlight>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id, book_id, selected_text, page_number, rects, color, created_at \
             FROM highlights WHERE book_id = ?1 ORDER BY created_at, rowid",
        )?;
        let highlights = statement
            .query_map([book_id], read_highlight)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(highlights)
    }

    pub fn highlight(&self, id: &str) -> Result<Option<Highlight>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, book_id, selected_text, page_number, rects, color, created_at \
                 FROM highlights WHERE id = ?1",
                [id],
                read_highlight,
            )
            .optional()?)
    }

    pub fn remove_highlight(&self, id: &str) -> Result<(), StoreError> {
        let removed = self
            .connection
            .execute("DELETE FROM highlights WHERE id = ?1", [id])?;

        match removed {
            0 => Err(StoreError::NoSuchHighlight(id.to_owned())),
            _ => Ok(()),
        }
    }

    /// Stores one turn of the conversation about a highlight.
    pub fn add_message(
        &self,
        about: &Conversation,
        role: Role,
        content: &str,
        citations: &[Citation],
    ) -> Result<ChatMessage, StoreError> {
        let message = ChatMessage {
            id: new_id(),
            about: about.clone(),
            role,
            content: content.to_owned(),
            citations: citations.to_vec(),
            created_at: OffsetDateTime::now_utc(),
        };

        let (highlight_id, folder_id) = about.columns();

        self.connection
            .execute(
                "INSERT INTO chat_messages (id, highlight_id, folder_id, role, content, \
                 citations, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    message.id,
                    highlight_id,
                    folder_id,
                    write_role(message.role),
                    message.content,
                    (!citations.is_empty())
                        .then(|| serde_json::to_string(citations).ok())
                        .flatten(),
                    format_time(message.created_at),
                ],
            )
            .map_err(|err| match err {
                // The foreign keys here are the highlight and the folder, and
                // exactly one of them was given — so a violation can only mean
                // the conversation has nothing to hang on.
                rusqlite::Error::SqliteFailure(error, _)
                    if error.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    match about {
                        Conversation::Highlight(id) => StoreError::NoSuchHighlight(id.clone()),
                        Conversation::Folder(id) => StoreError::NoSuchFolder(id.clone()),
                    }
                }
                other => StoreError::Database(other),
            })?;

        Ok(message)
    }

    /// Makes a shelf. Names are not unique: two shelves called "later" are two
    /// shelves, and telling them apart is the reader's business, not ours.
    pub fn create_folder(&self, name: &str) -> Result<Folder, StoreError> {
        let folder = Folder {
            id: new_id(),
            name: name.trim().to_owned(),
            created_at: OffsetDateTime::now_utc(),
            book_count: 0,
        };

        self.connection.execute(
            "INSERT INTO folders (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![folder.id, folder.name, format_time(folder.created_at)],
        )?;

        Ok(folder)
    }

    /// Every shelf, oldest first, each with how many books are on it.
    ///
    /// Oldest first rather than most-recently-used: a sidebar whose rows move
    /// when they are used is a sidebar the reader has to read every time.
    pub fn folders(&self) -> Result<Vec<Folder>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT f.id, f.name, f.created_at, count(b.id) AS book_count \
             FROM folders f LEFT JOIN books b ON b.folder_id = f.id \
             GROUP BY f.id ORDER BY f.created_at, f.rowid",
        )?;
        let folders = statement
            .query_map([], read_folder)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(folders)
    }

    pub fn folder(&self, id: &str) -> Result<Option<Folder>, StoreError> {
        Ok(self.folders()?.into_iter().find(|folder| folder.id == id))
    }

    pub fn rename_folder(&self, id: &str, name: &str) -> Result<(), StoreError> {
        let changed = self.connection.execute(
            "UPDATE folders SET name = ?2 WHERE id = ?1",
            params![id, name.trim()],
        )?;

        match changed {
            0 => Err(StoreError::NoSuchFolder(id.to_owned())),
            _ => Ok(()),
        }
    }

    /// Removes a shelf and the conversation held with it.
    ///
    /// The books stay. A shelf is an arrangement of the library, not a part of
    /// it, and deleting an arrangement must not delete what was arranged —
    /// which is why `folder_id` is set to null rather than cascading.
    pub fn remove_folder(&self, id: &str) -> Result<(), StoreError> {
        let changed = self
            .connection
            .execute("DELETE FROM folders WHERE id = ?1", [id])?;

        match changed {
            0 => Err(StoreError::NoSuchFolder(id.to_owned())),
            _ => Ok(()),
        }
    }

    /// Puts a book on a shelf, or takes it off one with `None`.
    ///
    /// A book is on one shelf at a time. Two shelves sharing a book would make
    /// "ask this shelf" ambiguous about which conversation a passage belongs
    /// to, and the reader gains a filing system in return for that.
    pub fn move_book(&self, book_id: &str, folder_id: Option<&str>) -> Result<(), StoreError> {
        let changed = self
            .connection
            .execute(
                "UPDATE books SET folder_id = ?2 WHERE id = ?1",
                params![book_id, folder_id],
            )
            .map_err(|err| match err {
                rusqlite::Error::SqliteFailure(error, _)
                    if error.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    StoreError::NoSuchFolder(folder_id.unwrap_or_default().to_owned())
                }
                other => StoreError::Database(other),
            })?;

        match changed {
            0 => Err(StoreError::NoSuchBook(book_id.to_owned())),
            _ => Ok(()),
        }
    }

    /// The books on a shelf, most recently touched first.
    pub fn books_in(&self, folder_id: &str) -> Result<Vec<Book>, StoreError> {
        let mut statement = self.connection.prepare(&format!(
            "{BOOK_COLUMNS} WHERE folder_id = ?1 ORDER BY updated_at DESC, rowid DESC"
        ))?;
        let books = statement
            .query_map([folder_id], read_book)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(books)
    }

    /// One conversation, oldest first.
    pub fn messages(&self, about: &Conversation) -> Result<Vec<ChatMessage>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id, highlight_id, folder_id, role, content, citations, created_at \
             FROM chat_messages \
             WHERE highlight_id IS ?1 AND folder_id IS ?2 ORDER BY created_at, rowid",
        )?;
        let (highlight_id, folder_id) = about.columns();
        let messages = statement
            .query_map(params![highlight_id, folder_id], read_message)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(messages)
    }

    fn id_with_hash(&self, file_hash: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT id FROM books WHERE file_hash = ?1",
                [file_hash],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn path_for_hash(&self, file_hash: &str) -> PathBuf {
        self.root.join(DOCUMENTS).join(format!("{file_hash}.pdf"))
    }
}

/// What is read out of a document once, when it is added.
struct Extracted {
    page_count: u32,
    full_text: String,
    outline: Vec<OutlineItem>,
}

fn extract(path: &Path) -> Result<Extracted, StoreError> {
    let document = Document::open(path)?;

    Ok(Extracted {
        page_count: document.page_count(),
        full_text: document.full_text()?,
        outline: document.outline(),
    })
}

const BOOK_COLUMNS: &str = "SELECT id, file_name, file_hash, page_count, outline, folder_id, \
                            created_at, updated_at, last_read_page, last_read_highlight_id, \
                            last_read_outline_open, last_read_chat_panel_open FROM books";

fn read_book(row: &Row<'_>) -> rusqlite::Result<Book> {
    let page: Option<u32> = row.get("last_read_page")?;

    Ok(Book {
        id: row.get("id")?,
        file_name: row.get("file_name")?,
        file_hash: row.get("file_hash")?,
        page_count: row.get("page_count")?,
        outline: read_outline(row.get::<_, Option<String>>("outline")?.as_deref()),
        folder_id: row.get("folder_id")?,
        created_at: read_time(&row.get::<_, String>("created_at")?),
        updated_at: read_time(&row.get::<_, String>("updated_at")?),
        reading: page.map(|page| ReadingState {
            page,
            highlight_id: row.get("last_read_highlight_id").unwrap_or(None),
            outline_open: row.get("last_read_outline_open").unwrap_or(None),
            chat_panel_open: row.get("last_read_chat_panel_open").unwrap_or(None),
        }),
    })
}

fn read_highlight(row: &Row<'_>) -> rusqlite::Result<Highlight> {
    Ok(Highlight {
        id: row.get("id")?,
        book_id: row.get("book_id")?,
        selected_text: row.get("selected_text")?,
        page_number: row.get("page_number")?,
        rects: read_json(row.get::<_, Option<String>>("rects")?.as_deref()),
        color: row.get("color")?,
        created_at: read_time(&row.get::<_, String>("created_at")?),
    })
}

fn read_folder(row: &Row<'_>) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: row.get("id")?,
        name: row.get("name")?,
        created_at: read_time(&row.get::<_, String>("created_at")?),
        book_count: row.get("book_count")?,
    })
}

fn read_message(row: &Row<'_>) -> rusqlite::Result<ChatMessage> {
    let highlight_id: Option<String> = row.get("highlight_id")?;
    let folder_id: Option<String> = row.get("folder_id")?;

    Ok(ChatMessage {
        id: row.get("id")?,
        // The table's check constraint is what makes this total: exactly one of
        // the two is filled in every row it will accept.
        about: match (highlight_id, folder_id) {
            (Some(id), _) => Conversation::Highlight(id),
            (_, Some(id)) => Conversation::Folder(id),
            (None, None) => {
                return Err(rusqlite::Error::InvalidColumnName(
                    "a message about neither a highlight nor a folder".to_owned(),
                ));
            }
        },
        role: read_role(&row.get::<_, String>("role")?),
        content: row.get("content")?,
        citations: read_json(row.get::<_, Option<String>>("citations")?.as_deref()),
        created_at: read_time(&row.get::<_, String>("created_at")?),
    })
}

/// Reads a JSON column, forgivingly.
///
/// A column that cannot be read means the same thing downstream as an absent
/// one — a highlight with no geometry is not drawn, an answer with no citations
/// shows none — and neither is worth making the rest of the row unreadable
/// over. This is the stance chatbook takes on the same columns.
fn read_json<T: serde::de::DeserializeOwned + Default>(stored: Option<&str>) -> T {
    stored
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default()
}

fn read_outline(stored: Option<&str>) -> Vec<OutlineItem> {
    read_json(stored)
}

fn write_outline(outline: &[OutlineItem]) -> Option<String> {
    // A book with no outline stores NULL rather than `[]`: both mean "no
    // chapter bounds, use a page window", and one of them says it plainly.
    (!outline.is_empty())
        .then(|| serde_json::to_string(outline).ok())
        .flatten()
}

fn write_role(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

/// Anything that is not the reader is the agent: a role that cannot be read is
/// far more likely to be a spelling of "assistant" than words the reader wrote,
/// and attributing an answer to the reader would put it in their own history.
fn read_role(stored: &str) -> Role {
    match stored {
        "user" => Role::User,
        _ => Role::Assistant,
    }
}

fn read_time(stored: &str) -> OffsetDateTime {
    OffsetDateTime::parse(stored, &Rfc3339).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

fn format_time(time: OffsetDateTime) -> String {
    time.format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

fn now() -> String {
    format_time(OffsetDateTime::now_utc())
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn hash_of(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Where the platform keeps application data, plus pedro's own directory.
fn default_root() -> Result<PathBuf, StoreError> {
    Ok(dirs::data_dir()
        .ok_or(StoreError::NoDataDirectory)?
        .join("pedro"))
}

const SCHEMA: &str = r#"
CREATE TABLE books (
    id TEXT PRIMARY KEY,
    file_name TEXT NOT NULL,
    file_hash TEXT NOT NULL UNIQUE,
    full_text TEXT NOT NULL,
    page_count INTEGER NOT NULL,
    outline TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_read_page INTEGER,
    last_read_highlight_id TEXT,
    last_read_outline_open INTEGER,
    last_read_chat_panel_open INTEGER
);

CREATE TABLE highlights (
    id TEXT PRIMARY KEY,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    selected_text TEXT NOT NULL,
    page_number INTEGER NOT NULL,
    rects TEXT NOT NULL,
    color TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX highlights_by_book ON highlights(book_id);

CREATE TABLE chat_messages (
    id TEXT PRIMARY KEY,
    highlight_id TEXT NOT NULL REFERENCES highlights(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    citations TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX messages_by_highlight ON chat_messages(highlight_id);
"#;

/// Shelves, and conversations that hang off one.
///
/// A conversation belonged to a highlight, which is a passage of one book. A
/// shelf is asked about as a whole, so its conversation cannot belong to a
/// passage — hence a second column and the check that exactly one of them is
/// filled. SQLite cannot drop a NOT NULL, so the table is rebuilt rather than
/// altered; the rows are carried across, because a reader's conversations are
/// the part of this file that cannot be rebuilt from the PDFs. All of it in
/// one transaction, so a library that fails to open is still the library it was.
const SHELVES: &str = r#"
BEGIN;

CREATE TABLE folders (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL
);

ALTER TABLE books ADD COLUMN folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL;

CREATE INDEX books_by_folder ON books(folder_id);

CREATE TABLE chat_messages_new (
    id TEXT PRIMARY KEY,
    highlight_id TEXT REFERENCES highlights(id) ON DELETE CASCADE,
    folder_id TEXT REFERENCES folders(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    citations TEXT,
    created_at TEXT NOT NULL,
    CHECK ((highlight_id IS NULL) <> (folder_id IS NULL))
);

INSERT INTO chat_messages_new
    (id, highlight_id, folder_id, role, content, citations, created_at)
SELECT id, highlight_id, NULL, role, content, citations, created_at
FROM chat_messages;

DROP INDEX messages_by_highlight;
DROP TABLE chat_messages;
ALTER TABLE chat_messages_new RENAME TO chat_messages;

CREATE INDEX messages_by_highlight ON chat_messages(highlight_id);
CREATE INDEX messages_by_folder ON chat_messages(folder_id);

COMMIT;
"#;

/// Brings the database up to the current schema.
///
/// `user_version` rather than a migrations table: there is one writer, one
/// file, and the version is a number SQLite already carries.
fn migrate(connection: &Connection) -> Result<(), StoreError> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;

    if version < 1 {
        connection.execute_batch(SCHEMA)?;
        connection.pragma_update(None, "user_version", 1)?;
    }

    if version < 2 {
        index::create(connection)?;
        connection.pragma_update(None, "user_version", 2)?;
    }

    if version < 3 {
        // Foreign keys off for the rebuild: the rows are moved to a table the
        // old name still points at, and a key checked halfway through that
        // would fail on rows that are about to be correct.
        connection.pragma_update(None, "foreign_keys", "OFF")?;
        let rebuilt = connection.execute_batch(SHELVES);
        connection.pragma_update(None, "foreign_keys", "ON")?;
        rebuilt?;

        connection.pragma_update(None, "user_version", 3)?;
    }

    Ok(())
}
