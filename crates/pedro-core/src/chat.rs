//! Asking about a highlighted passage.
//!
//! This is where the pieces meet: the highlight says which page, the page says
//! which chapter, the chapter becomes the context, the stored turns become the
//! conversation, an installed CLI answers it, and the sources it names become
//! pages the reader can jump back to.
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
use crate::model::{ChatMessage, Role};
use crate::prompt::{Passage, Retrieved, Turn, build_conversation, build_system_prompt};
use crate::store::{Store, StoreError};

/// How many passages a search may add to a question.
///
/// Enough that a question about two ends of a book has both, few enough that
/// the passages the reader actually marked are still the bulk of what is read.
const RETRIEVED: usize = 6;

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
}

/// What the reader is asking, and about what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// The passages the question is about, in the order they were marked.
    ///
    /// Their pages decide how much of the book travels with the question. The
    /// first is where the conversation lives: a question about two passages is
    /// still one conversation, and it has to hang somewhere the reader can find
    /// it again.
    pub highlight_ids: Vec<String>,
    pub text: String,
    /// Whether the agent may look beyond the book.
    pub web_search: bool,
}

/// A question that has been read out of the store and is ready to be asked.
///
/// Carries the book's text along with the prompt because the answer's sources
/// are looked up in it, and looking them up must not need the store again.
pub struct Asked {
    pub highlight_id: String,
    pub prompt: Prompt,
    full_text: String,
    page_count: u32,
}

/// Reads everything a question needs, and records the question itself.
///
/// The question is stored before the agent is asked. An answer that fails
/// therefore leaves the question in the conversation, which is the honest
/// record: the reader did ask, and can ask again without retyping it.
pub fn prepare(store: &Store, question: &Question) -> Result<Asked, ChatError> {
    let mut marked = Vec::with_capacity(question.highlight_ids.len());
    for id in &question.highlight_ids {
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
        .passages_for(&book.id, &question.text, RETRIEVED)?
        .into_iter()
        .filter(|hit| {
            !excerpts
                .iter()
                .any(|excerpt| (excerpt.start_page..=excerpt.end_page).contains(&hit.page_number))
        })
        .map(|hit| Retrieved {
            page: hit.page_number,
            text: hit.text,
        })
        .collect::<Vec<_>>();

    let system = build_system_prompt(&excerpts, &passages, &retrieved, question.web_search);

    let history: Vec<Turn> = store
        .messages(&highlight.id)?
        .into_iter()
        .map(|message| Turn {
            role: message.role,
            content: message.content,
        })
        .collect();

    store.add_message(&highlight.id, Role::User, &question.text, &[])?;

    Ok(Asked {
        highlight_id: highlight.id,
        prompt: Prompt {
            system,
            turns: build_conversation(&history, &question.text),
            web_search: question.web_search,
            workspace: Some(store.root().to_path_buf()),
        },
        full_text,
        page_count: book.page_count,
    })
}

/// Stores an answer with its sources resolved to pages.
///
/// The citations are resolved against the whole book rather than the excerpt:
/// the excerpt is a verbatim run of its pages, so a passage quoted from it is
/// in the book too, and looking in the book is what turns it into a page number
/// the reader can jump to.
pub fn record(store: &Store, asked: &Asked, answer: &str) -> Result<ChatMessage, ChatError> {
    let citations = parse_citations(
        answer,
        Some(BookText {
            full_text: &asked.full_text,
            page_count: asked.page_count,
        }),
    );

    Ok(store.add_message(&asked.highlight_id, Role::Assistant, answer, &citations)?)
}

/// Asks `agent` about a highlighted passage and stores both turns.
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
