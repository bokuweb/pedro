//! One conversation about one passage.
//!
//! chatbook's unit of conversation is the highlight, not the book: a question
//! is about a passage, and the answers to it live with that passage. This holds
//! the one that is open, including the answer arriving a token at a time.

use gpui::SharedString;
use pedro_agent::Cancellation;
use pedro_core::model::ChatMessage;

/// The conversation the chat panel is showing.
pub struct Conversation {
    /// The stored passage this conversation hangs off, once there is one.
    pub highlight_id: Option<String>,
    /// The passage the question is about, as the reader marked it.
    pub passage: SharedString,
    /// Where that passage is, so a stored highlight can be found again.
    pub page: u32,
    /// The stored turns, reloaded whenever an answer finishes.
    pub messages: Vec<ChatMessage>,
    /// The question being answered right now, which is not stored yet.
    pub pending: Option<SharedString>,
    /// The answer as far as it has arrived.
    pub streaming: String,
    /// Why the last question failed, if it did.
    pub error: Option<SharedString>,
    /// Stops the CLI mid-answer.
    pub cancellation: Cancellation,
}

impl Conversation {
    pub fn about(passage: impl Into<SharedString>, page: u32) -> Self {
        Self {
            highlight_id: None,
            passage: passage.into(),
            page,
            messages: Vec::new(),
            pending: None,
            streaming: String::new(),
            error: None,
            cancellation: Cancellation::new(),
        }
    }

    /// Whether an answer is being written right now.
    pub fn is_answering(&self) -> bool {
        self.pending.is_some()
    }

    /// Starts a question: it shows immediately, above an answer that is still
    /// empty, because a question that vanishes into a spinner reads as lost.
    pub fn asked(&mut self, question: impl Into<SharedString>) {
        self.pending = Some(question.into());
        self.streaming.clear();
        self.error = None;
        self.cancellation = Cancellation::new();
    }

    pub fn answered(&mut self, messages: Vec<ChatMessage>) {
        self.messages = messages;
        self.pending = None;
        self.streaming.clear();
    }

    pub fn failed(&mut self, why: impl Into<SharedString>) {
        self.pending = None;
        self.streaming.clear();
        self.error = Some(why.into());
    }
}
