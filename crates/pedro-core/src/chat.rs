//! Asking about a highlighted passage.
//!
//! This is where the pieces meet: the highlight says which page, the page says
//! which chapter, the chapter becomes the context, the stored turns become the
//! conversation, an installed CLI answers it, and the sources it names become
//! pages the reader can jump back to.
//!
//! chatbook does this in a Worker with an API key. The only real difference
//! here is who answers — and that the whole exchange is rows on the reader's
//! own disk.

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

/// Asks `agent` about a highlighted passage and stores both turns.
///
/// `on_delta` is called with each piece of the answer as it arrives, so the
/// reader watches it being written; the stored message is returned when it is
/// finished.
///
/// The question is stored before the agent is asked. An answer that fails
/// therefore leaves the question in the conversation, which is the honest
/// record: the reader did ask, and can ask again without retyping it.
///
/// Blocking from start to finish — it runs a subprocess — so it belongs on a
/// background thread.
pub fn ask(
    store: &Store,
    agent: &DiscoveredAgent,
    question: &Question,
    cancellation: &Cancellation,
    on_delta: &mut dyn FnMut(&str),
) -> Result<ChatMessage, ChatError> {
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

    let prompt = Prompt {
        system,
        turns: build_conversation(&history, &question.text),
        web_search: question.web_search,
        workspace: Some(store.root().to_path_buf()),
    };

    let answer = pedro_agent::run(agent, &prompt, cancellation, &mut |event| {
        if let AgentEvent::Delta(text) = &event {
            on_delta(text);
        }
    })?;

    // The citations are resolved against the whole book rather than the
    // excerpt: the excerpt is a verbatim run of its pages, so a passage quoted
    // from it is in the book too, and looking in the book is what turns it into
    // a page number the reader can jump to.
    let citations = parse_citations(
        &answer,
        Some(BookText {
            full_text: &full_text,
            page_count: book.page_count,
        }),
    );

    Ok(store.add_message(&highlight.id, Role::Assistant, &answer, &citations)?)
}
