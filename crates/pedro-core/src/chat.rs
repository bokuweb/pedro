//! Asking about a highlighted passage, or about a shelf of books.
//!
//! This is where the pieces meet: the highlight says which page, the page says
//! which chapter, the chapter becomes the context, the stored turns become the
//! conversation, an installed CLI answers it, and the sources it names become
//! pages the reader can jump back to.
//!
//! A question put to a shelf runs the same course with one piece swapped. There
//! is no passage and so no chapter to cut around it; searching every book on
//! the shelf for the question is what produces the context instead. Everything
//! after that — the turns, the streaming, the sources — is the same code, which
//! is why a citation from a shelf resolves to a page the same way, and only has
//! to say which book as well.
//!
//! It comes in three parts on purpose. The store is a single connection behind
//! a lock, and an agent takes as long as it takes — a minute is ordinary. Doing
//! all of it under one borrow of the store would stop the reader turning a page
//! for that whole minute. So the store is read, the agent runs with nothing
//! held, and the store is written: [`prepare`], the run, [`record`]. [`ask`] is
//! the three in a row, for callers with nothing else to do meanwhile.

use pedro_agent::{AgentError, AgentEvent, Cancellation, DiscoveredAgent, Prompt};

use crate::citation::{BookText, parse_citations};
use crate::excerpt::select_excerpts;
use crate::model::{ChatMessage, Conversation, Role};
use crate::prompt::{
    Passage, Retrieved, Turn, build_conversation, build_shelf_prompt, build_system_prompt,
};
use crate::store::{Store, StoreError};

/// How many passages a search may add to a question.
///
/// Enough that a question about two ends of a book has both, few enough that
/// the passages the reader actually marked are still the bulk of what is read.
const RETRIEVED: usize = 6;

/// How many a question put to a shelf may gather.
///
/// More than a book gets, because they are the whole of the context rather than
/// an addition to an excerpt, and they are shared out among several books.
const RETRIEVED_FROM_SHELF: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    #[error(transparent)]
    Store(#[from] StoreError),

    #[error(transparent)]
    Agent(#[from] AgentError),

    #[error("no highlight with id {0}")]
    NoSuchHighlight(String),

    #[error("no book with id {0}")]
    NoSuchBook(String),

    #[error("no folder with id {0}")]
    NoSuchFolder(String),

    #[error("there are no books on this shelf")]
    EmptyShelf,
}

/// What a question is put to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// The passages the reader marked, in the order they marked them.
    ///
    /// Their pages decide how much of the book travels with the question. The
    /// first is where the conversation lives: a question about two passages is
    /// still one conversation, and it has to hang somewhere the reader can find
    /// it again.
    Passages(Vec<String>),
    /// A shelf, asked as a whole.
    Shelf(String),
}

/// What the reader is asking, and about what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub about: Subject,
    pub text: String,
    /// Whether the agent may look beyond the books.
    pub web_search: bool,
}

/// A question that has been read out of the store and is ready to be asked.
///
/// Carries the books' text along with the prompt because the answer's sources
/// are looked up in it, and looking them up must not need the store again —
/// which is the whole point of the three-part split below.
pub struct Asked {
    /// Where the answer will be filed.
    pub about: Conversation,
    pub prompt: Prompt,
    books: Vec<Cited>,
}

/// A book an answer's sources may be looked up in.
struct Cited {
    id: String,
    title: String,
    full_text: String,
    page_count: u32,
}

/// Reads everything a question needs, and records the question itself.
///
/// The question is stored before the agent is asked. An answer that fails
/// therefore leaves the question in the conversation, which is the honest
/// record: the reader did ask, and can ask again without retyping it.
pub fn prepare(store: &Store, question: &Question) -> Result<Asked, ChatError> {
    match &question.about {
        Subject::Passages(ids) => prepare_passages(store, question, ids),
        Subject::Shelf(id) => prepare_shelf(store, question, id),
    }
}

fn prepare_passages(
    store: &Store,
    question: &Question,
    highlight_ids: &[String],
) -> Result<Asked, ChatError> {
    let mut marked = Vec::with_capacity(highlight_ids.len());
    for id in highlight_ids {
        marked.push(
            store
                .highlight(id)?
                .ok_or_else(|| ChatError::NoSuchHighlight(id.clone()))?,
        );
    }

    // The first passage is the one the conversation belongs to.
    let highlight = marked
        .first()
        .cloned()
        .ok_or_else(|| ChatError::NoSuchHighlight(String::new()))?;
    let book = store
        .book(&highlight.book_id)?
        .ok_or_else(|| ChatError::NoSuchBook(highlight.book_id.clone()))?;

    let full_text = store.full_text(&book.id)?;
    let pages: Vec<u32> = marked
        .iter()
        .map(|highlight| highlight.page_number)
        .collect();
    let passages: Vec<Passage> = marked
        .iter()
        .map(|highlight| Passage {
            page: highlight.page_number,
            text: highlight.selected_text.clone(),
        })
        .collect();

    let excerpts = select_excerpts(&full_text, &pages, &book.outline);

    // What the marked passages are near is one kind of context; what the
    // question itself is about is another. A book is searched for the question
    // and whatever that turns up outside the excerpt is added, because the
    // answer to "how does this square with chapter 20" is in chapter 20.
    let retrieved = store
        .passages_for(std::slice::from_ref(&book.id), &question.text, RETRIEVED)?
        .into_iter()
        .filter(|hit| {
            !excerpts
                .iter()
                .any(|excerpt| (excerpt.start_page..=excerpt.end_page).contains(&hit.page_number))
        })
        .map(|hit| Retrieved {
            book: None,
            page: hit.page_number,
            text: hit.text,
        })
        .collect::<Vec<_>>();

    let system = build_system_prompt(&excerpts, &passages, &retrieved, question.web_search);
    let about = Conversation::Highlight(highlight.id);

    finish(
        store,
        question,
        about,
        system,
        vec![Cited {
            id: book.id,
            title: book.file_name,
            full_text,
            page_count: book.page_count,
        }],
    )
}

fn prepare_shelf(store: &Store, question: &Question, folder_id: &str) -> Result<Asked, ChatError> {
    let folder = store
        .folder(folder_id)?
        .ok_or_else(|| ChatError::NoSuchFolder(folder_id.to_owned()))?;

    let books = store.books_in(&folder.id)?;
    if books.is_empty() {
        return Err(ChatError::EmptyShelf);
    }

    let ids: Vec<String> = books.iter().map(|book| book.id.clone()).collect();
    let title_of = |book_id: &str| {
        books
            .iter()
            .find(|book| book.id == book_id)
            .map(|book| book.file_name.clone())
    };

    let retrieved = store
        .passages_for(&ids, &question.text, RETRIEVED_FROM_SHELF)?
        .into_iter()
        .map(|hit| Retrieved {
            book: title_of(&hit.book_id),
            page: hit.page_number,
            text: hit.text,
        })
        .collect::<Vec<_>>();

    let system = build_shelf_prompt(&folder.name, &retrieved, question.web_search);

    // Every book on the shelf, because a source is looked up by its quotation
    // and the quotation does not say which book it came from.
    let mut cited = Vec::with_capacity(books.len());
    for book in books {
        cited.push(Cited {
            full_text: store.full_text(&book.id)?,
            id: book.id,
            title: book.file_name,
            page_count: book.page_count,
        });
    }

    finish(
        store,
        question,
        Conversation::Folder(folder.id),
        system,
        cited,
    )
}

/// The half both kinds of question share: the stored turns become the
/// conversation, and the new question joins them.
fn finish(
    store: &Store,
    question: &Question,
    about: Conversation,
    system: String,
    books: Vec<Cited>,
) -> Result<Asked, ChatError> {
    let history: Vec<Turn> = store
        .messages(&about)?
        .into_iter()
        .map(|message| Turn {
            role: message.role,
            content: message.content,
        })
        .collect();

    store.add_message(&about, Role::User, &question.text, &[])?;

    Ok(Asked {
        about,
        prompt: Prompt {
            system,
            turns: build_conversation(&history, &question.text),
            web_search: question.web_search,
            workspace: Some(store.root().to_path_buf()),
        },
        books,
    })
}

/// Stores an answer with its sources resolved to pages.
///
/// The citations are resolved against whole books rather than the excerpt: the
/// excerpt is a verbatim run of its pages, so a passage quoted from it is in the
/// book too, and looking in the book is what turns it into a page number the
/// reader can jump to. A shelf is looked up the same way, in each of its books
/// until one holds the quotation.
pub fn record(store: &Store, asked: &Asked, answer: &str) -> Result<ChatMessage, ChatError> {
    let books: Vec<BookText<'_>> = asked
        .books
        .iter()
        .map(|book| BookText {
            id: &book.id,
            title: &book.title,
            full_text: &book.full_text,
            page_count: book.page_count,
        })
        .collect();

    let citations = parse_citations(answer, &books);

    Ok(store.add_message(&asked.about, Role::Assistant, answer, &citations)?)
}

/// Asks `agent` the question and stores both turns.
///
/// `on_delta` is called with each piece of the answer as it arrives, so the
/// reader watches it being written; the stored message is returned when it is
/// finished.
///
/// Holds `store` throughout, so a caller that has other uses for it should run
/// the three parts itself rather than calling this.
pub fn ask(
    store: &Store,
    agent: &DiscoveredAgent,
    question: &Question,
    cancellation: &Cancellation,
    on_delta: &mut dyn FnMut(&str),
) -> Result<ChatMessage, ChatError> {
    let asked = prepare(store, question)?;
    let answer = run(agent, &asked, cancellation, on_delta)?;

    record(store, &asked, &answer)
}

/// Puts the question to the agent. Touches no store.
pub fn run(
    agent: &DiscoveredAgent,
    asked: &Asked,
    cancellation: &Cancellation,
    on_delta: &mut dyn FnMut(&str),
) -> Result<String, AgentError> {
    pedro_agent::run(agent, &asked.prompt, cancellation, &mut |event| {
        if let AgentEvent::Delta(text) = &event {
            on_delta(text);
        }
    })
}
