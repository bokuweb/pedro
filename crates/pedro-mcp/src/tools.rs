//! What another agent may do with the library: find passages, and read them.
//!
//! The tools are the reader's own operations, minus the reader. Everything the
//! search box does — the same hybrid index, the same chunks, the same page
//! numbers — and the pages themselves, taken from the text already extracted
//! into the library so that nothing here needs pdfium.
//!
//! There is deliberately no tool for asking a question. pedro answers questions
//! by handing a passage to an agent CLI; the thing calling these tools is
//! already that agent, and giving it a way to ask a second one would be paying
//! twice to have the same reasoning done a step further from the caller. What
//! it needs from pedro is the retrieval, which is what pedro is for.
//!
//! Nor is there one for removing a book. Adding is undoable in the reader and
//! costs a file; deleting takes highlights and conversations with it, and that
//! is the reader's own decision to make.

use std::path::PathBuf;

use pedro_core::PAGE_DELIMITER;
use pedro_core::model::Book;
use pedro_core::store::Store;
use pedro_search::Hit;
use serde_json::{Value, json};

use crate::protocol::INVALID_PARAMS;

/// How many passages a search hands back when the caller does not say.
const PASSAGES: usize = 8;

/// The most it will hand back however loudly it is asked. A caller that wants
/// the whole book should read the pages.
const PASSAGE_CEILING: usize = 40;

/// The most pages one read may take. Enough for a chapter of a technical book,
/// bounded so that a mistyped range cannot return the entire library.
const PAGE_CEILING: u32 = 25;

/// What went wrong with a call.
///
/// The distinction is not decoration: a malformed call is a protocol error,
/// which the client's plumbing should surface to whoever wrote it, while work
/// that failed comes back as a tool result the model itself reads and can act
/// on — a missing book is something to try differently, not a bug in the
/// client.
enum Trouble {
    Params(String),
    Work(String),
}

/// Every tool, as `tools/list` describes them.
pub fn catalogue() -> Vec<Value> {
    vec![
        json!({
            "name": "list_books",
            "description": "List the books in the pedro library: their ids, how long they are, \
                            and whether they can be searched by meaning as well as by words. \
                            The id is what every other tool here takes.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
        }),
        json!({
            "name": "search_library",
            "description": "Find the passages in the library that bear on a query, best first. \
                            Searches by the words themselves and, when the embedding model has \
                            been fetched, by what they mean; the two rankings are fused. Each \
                            hit names the book and the page it is on, so it can be read in full \
                            with read_pages.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What to look for. A question works as well as a phrase.",
                    },
                    "book_id": {
                        "type": "string",
                        "description": "Search only this book. Omit to search the whole library.",
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": PASSAGE_CEILING,
                        "description": format!("How many passages to return. Defaults to {PASSAGES}."),
                    },
                },
                "required": ["query"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "read_pages",
            "description": "Read a book's pages verbatim, as the text they were extracted from. \
                            Use it after search_library to see a hit in its context, or to read \
                            a chapter named by book_contents.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "book_id": { "type": "string", "description": "From list_books or a search hit." },
                    "page": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "The first page to read, counted the way the reader counts: from 1.",
                    },
                    "until": {
                        "type": "integer",
                        "minimum": 1,
                        "description": format!(
                            "The last page to read, inclusive. Defaults to `page`. \
                             At most {PAGE_CEILING} pages come back at once."
                        ),
                    },
                },
                "required": ["book_id", "page"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "book_contents",
            "description": "The book's own table of contents: its top-level chapters and the \
                            page each one starts on. Empty for a book that ships no outline.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "book_id": { "type": "string", "description": "From list_books or a search hit." },
                },
                "required": ["book_id"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "add_book",
            "description": "Add a PDF on this machine to the library and index it, so that it \
                            can be searched and read. A book already in the library is not \
                            added twice: identity is the file's contents, so re-adding it under \
                            a new name only renames it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "An absolute path to a PDF file." },
                },
                "required": ["path"],
                "additionalProperties": false,
            },
        }),
    ]
}

/// Runs one `tools/call`, or says why it could not.
///
/// The error carries a JSON-RPC code, for the two things that are the client's
/// fault rather than the model's: a tool that does not exist, and arguments
/// that do not fit the schema this same module published.
pub fn call(store: &mut Store, params: &Value) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params.get("arguments").unwrap_or(&Value::Null);

    let done = match name {
        "list_books" => list_books(store),
        "search_library" => search_library(store, arguments),
        "read_pages" => read_pages(store, arguments),
        "book_contents" => book_contents(store, arguments),
        "add_book" => add_book(store, arguments),
        other => {
            return Err((INVALID_PARAMS, format!("no tool named {other}")));
        }
    };

    match done {
        Ok(body) => Ok(text(body, false)),
        Err(Trouble::Work(why)) => Ok(text(why, true)),
        Err(Trouble::Params(why)) => Err((INVALID_PARAMS, why)),
    }
}

/// A tool result. `failed` is what tells the model the answer is a problem
/// rather than a finding.
fn text(body: String, failed: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": body }],
        "isError": failed,
    })
}

fn list_books(store: &mut Store) -> Result<String, Trouble> {
    let books = store.books().map_err(work)?;
    if books.is_empty() {
        return Ok("The library is empty. Add a PDF with add_book.".to_owned());
    }

    // The shelves a reader sorted the books onto. Named here rather than left
    // as an id, because the name is the only part of a shelf that says
    // anything: "on shelf Cryptography" is a reason to search there next.
    let shelves = store.folders().map_err(work)?;

    let mut lines = vec![format!(
        "{} in the library, most recently touched first. Searching by {}.\n",
        count(books.len(), "book"),
        retrieval(store),
    )];

    for book in &books {
        let shelf = book
            .folder_id
            .as_ref()
            .and_then(|id| shelves.iter().find(|shelf| &shelf.id == id))
            .map(|shelf| shelf.name.as_str());

        lines.push(describe(book, shelf));
    }

    Ok(lines.join("\n"))
}

fn search_library(store: &mut Store, arguments: &Value) -> Result<String, Trouble> {
    let query = string(arguments, "query")?;
    let book_id = optional_string(arguments, "book_id")?;
    let limit = optional_count(arguments, "limit")?
        .unwrap_or(PASSAGES)
        .min(PASSAGE_CEILING);

    if let Some(id) = &book_id {
        known_book(store, id)?;
    }

    let across = store.search(&query).map_err(work)?;
    let hits: Vec<&Hit> = across
        .iter()
        .filter(|hit| book_id.as_ref().is_none_or(|id| &hit.book_id == id))
        .take(limit)
        .collect();

    if hits.is_empty() {
        // A search held to one book is held to it after the library has been
        // ranked, so a book with nothing near the top of a library-wide
        // ranking comes back empty even when it has something. Saying what was
        // found elsewhere is what lets the caller tell the two apart.
        let elsewhere = match book_id.is_some() && !across.is_empty() {
            // Not counted: the ranking is cut off at a fixed depth, so a number
            // here would be the depth as often as it was the truth.
            true => " Other books in the library do match — search again without book_id.",
            false => "",
        };

        return Ok(format!(
            "Nothing matches {query:?}{}. Searching by {}.{elsewhere}",
            match &book_id {
                Some(id) => format!(" in book {id}"),
                None => " in the library".to_owned(),
            },
            retrieval(store),
        ));
    }

    let titles = titles(store)?;
    let mut lines = vec![format!(
        "{} for {query:?}, best first. Searching by {}.",
        count(hits.len(), "passage"),
        retrieval(store),
    )];

    // Numbered rather than scored. The two ways of searching are fused by
    // position, so what comes out is a rank; the number attached to it is an
    // artefact of the fusion and means nothing on its own, and a model shown
    // "0.03" would read a good hit as a bad one.
    for (rank, hit) in hits.iter().enumerate() {
        let title = titles
            .iter()
            .find(|(id, _)| id == &hit.book_id)
            .map(|(_, name)| name.as_str())
            .unwrap_or("a book");

        lines.push(format!(
            "\n{}. {title} — page {} (book_id {})\n{}",
            rank + 1,
            hit.page_number,
            hit.book_id,
            hit.text.trim(),
        ));
    }

    Ok(lines.join("\n"))
}

fn read_pages(store: &mut Store, arguments: &Value) -> Result<String, Trouble> {
    let book_id = string(arguments, "book_id")?;
    let first = page(arguments, "page")?;
    let last = optional_page(arguments, "until")?.unwrap_or(first);

    let book = known_book(store, &book_id)?;
    if last < first {
        return Err(Trouble::Params(format!(
            "until ({last}) is before page ({first})"
        )));
    }

    let full_text = store.full_text(&book_id).map_err(work)?;
    let pages: Vec<&str> = full_text.split(PAGE_DELIMITER).collect();
    let total = pages.len() as u32;

    if first > total {
        return Err(Trouble::Work(format!(
            "{} has {total} pages, so there is no page {first}.",
            book.file_name,
        )));
    }

    let wanted = last.min(total);
    let taken = wanted.min(first + PAGE_CEILING - 1);

    let mut lines = vec![format!(
        "{} — pages {first} to {taken} of {total}.",
        book.file_name,
    )];
    if taken < wanted {
        lines.push(format!(
            "Pages {} to {wanted} were left out: at most {PAGE_CEILING} pages come back at once.",
            taken + 1,
        ));
    }

    for number in first..=taken {
        let body = pages[number as usize - 1].trim();
        lines.push(format!("\n--- page {number} ---\n{body}"));
    }

    Ok(lines.join("\n"))
}

fn book_contents(store: &mut Store, arguments: &Value) -> Result<String, Trouble> {
    let book_id = string(arguments, "book_id")?;
    let book = known_book(store, &book_id)?;

    if book.outline.is_empty() {
        return Ok(format!(
            "{} ships no outline. It has {} pages; read_pages takes them by number.",
            book.file_name, book.page_count,
        ));
    }

    let mut lines = vec![format!(
        "{} — {}, {} pages.\n",
        book.file_name,
        count(book.outline.len(), "chapter"),
        book.page_count,
    )];

    for chapter in &book.outline {
        lines.push(format!("page {:<5} {}", chapter.page_number, chapter.title));
    }

    Ok(lines.join("\n"))
}

fn add_book(store: &mut Store, arguments: &Value) -> Result<String, Trouble> {
    let path = PathBuf::from(string(arguments, "path")?);
    if !path.is_absolute() {
        return Err(Trouble::Params(format!(
            "path must be absolute, and {} is not — this server runs wherever the \
             client started it, so a relative path means nothing here",
            path.display(),
        )));
    }

    // Reading the whole document, extracting its text and indexing it, all of
    // which is what makes it searchable and all of which the caller waits for.
    let book = store
        .add_document(&path)
        .map_err(|err| Trouble::Work(format!("{} could not be added: {err}", path.display())))?;

    Ok(format!(
        "Added and indexed.\n\n{}\n\nSearchable by {}.",
        // A book arrives on no shelf; the reader is where it is put on one.
        describe(&book, None),
        retrieval(store),
    ))
}

/// One book, as every tool that names one names it.
fn describe(book: &Book, shelf: Option<&str>) -> String {
    let chapters = match book.outline.len() {
        0 => "no outline".to_owned(),
        n => count(n, "chapter"),
    };
    let place = match &book.reading {
        Some(state) => format!(", last read at page {}", state.page),
        None => String::new(),
    };
    let shelf = match shelf {
        Some(name) => format!("  ·  on shelf {name}"),
        None => String::new(),
    };

    format!(
        "{}\n  book_id {}  ·  {} pages  ·  {chapters}{place}{shelf}",
        book.file_name, book.id, book.page_count,
    )
}

/// Which kinds of search the library can currently do. Worth saying in every
/// answer: a library whose embedding model was never fetched still searches,
/// and a caller told only "no matches" would have no way to know the weaker
/// half is all it got.
fn retrieval(store: &Store) -> &'static str {
    match store.can_search_by_meaning() {
        true => "words and meaning",
        false => "words alone (fetch the embedding model to search by meaning)",
    }
}

fn titles(store: &Store) -> Result<Vec<(String, String)>, Trouble> {
    Ok(store
        .books()
        .map_err(work)?
        .into_iter()
        .map(|book| (book.id, book.file_name))
        .collect())
}

/// The book, or a failure the model can act on rather than a protocol error:
/// an id that is stale or invented is a thing to look up again, not a bug in
/// the client.
fn known_book(store: &Store, id: &str) -> Result<Book, Trouble> {
    store
        .book(id)
        .map_err(work)?
        .ok_or_else(|| Trouble::Work(format!("no book with id {id}. list_books has the ids.")))
}

fn work(err: impl std::fmt::Display) -> Trouble {
    Trouble::Work(err.to_string())
}

fn string(arguments: &Value, name: &str) -> Result<String, Trouble> {
    match arguments.get(name).and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => Ok(value.to_owned()),
        Some(_) => Err(Trouble::Params(format!("{name} is empty"))),
        None => Err(Trouble::Params(format!("{name} is required, as a string"))),
    }
}

fn optional_string(arguments: &Value, name: &str) -> Result<Option<String>, Trouble> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => string(arguments, name).map(Some),
    }
}

fn optional_count(arguments: &Value, name: &str) -> Result<Option<usize>, Trouble> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match value.as_u64() {
            Some(0) | None => Err(Trouble::Params(format!(
                "{name} must be a whole number of at least 1"
            ))),
            Some(number) => Ok(Some(number as usize)),
        },
    }
}

fn page(arguments: &Value, name: &str) -> Result<u32, Trouble> {
    optional_page(arguments, name)?
        .ok_or_else(|| Trouble::Params(format!("{name} is required, as a page number from 1")))
}

fn optional_page(arguments: &Value, name: &str) -> Result<Option<u32>, Trouble> {
    match optional_count(arguments, name)? {
        None => Ok(None),
        Some(number) if number <= u32::MAX as usize => Ok(Some(number as u32)),
        Some(_) => Err(Trouble::Params(format!("{name} is not a page number"))),
    }
}

/// "1 book", "3 books". Every count in an answer goes through here, because a
/// tool result is read by a model and "1 books" is the kind of thing it will
/// quote back.
///
fn count(n: usize, thing: &str) -> String {
    match n {
        1 => format!("1 {thing}"),
        _ => format!("{n} {thing}s"),
    }
}
