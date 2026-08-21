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
use crate::excerpt::select_excerpt;
use crate::model::{ChatMessage, Role};
use crate::prompt::{Turn, build_conversation, build_system_prompt};
use crate::store::{Store, StoreError};

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
    /// The passage the question is about. Its page decides how much of the
    /// book travels with it, and its conversation is where the answer lands.
    pub highlight_id: String,
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
    let highlight = store
        .highlight(&question.highlight_id)?
        .ok_or_else(|| ChatError::NoSuchHighlight(question.highlight_id.clone()))?;
    let book = store
        .book(&highlight.book_id)?
        .ok_or_else(|| ChatError::NoSuchBook(highlight.book_id.clone()))?;

    let full_text = store.full_text(&book.id)?;
    let excerpt = select_excerpt(&full_text, highlight.page_number, &book.outline);
    let system = build_system_prompt(&excerpt, &highlight.selected_text, question.web_search);

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
