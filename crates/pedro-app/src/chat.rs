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
    /// How much of it has been shown, in characters.
    ///
    /// An agent does not deliver its answer evenly — a burst of a hundred
    /// characters, then nothing for a second — and drawing exactly what has
    /// arrived puts that unevenness on screen. Letting the display run behind
    /// and catch up turns arrival into writing.
    pub revealed: usize,
    /// Why the last question failed, if it did.
    pub error: Option<SharedString>,
    /// The command that would fix it, when the failure was a CLI that is
    /// installed but signed out.
    pub sign_in: Option<&'static str>,
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
            revealed: 0,
            error: None,
            sign_in: None,
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
        self.revealed = 0;
        self.error = None;
        self.sign_in = None;
        self.cancellation = Cancellation::new();
    }

    pub fn answered(&mut self, messages: Vec<ChatMessage>) {
        self.messages = messages;
        self.pending = None;
        self.streaming.clear();
        self.revealed = 0;
    }

    /// The part of the answer that is on screen.
    pub fn visible(&self) -> &str {
        match self.streaming.char_indices().nth(self.revealed) {
            Some((at, _)) => &self.streaming[..at],
            None => &self.streaming,
        }
    }

    /// Shows a little more of what has arrived.
    ///
    /// The step is a fraction of what is waiting, so a long burst is drained
    /// quickly and a trickle is drawn as it comes; the constant keeps it moving
    /// when only a character or two is waiting.
    ///
    /// Returns whether anything is still hidden.
    pub fn reveal_more(&mut self) -> bool {
        let arrived = self.streaming.chars().count();
        let waiting = arrived.saturating_sub(self.revealed);
        if waiting == 0 {
            return false;
        }

        self.revealed += (waiting / 4).max(3).min(waiting);

        self.revealed < arrived
    }

    /// Shows all of it at once, for when there is no more coming.
    pub fn reveal_everything(&mut self) {
        self.revealed = self.streaming.chars().count();
    }

    pub fn failed(&mut self, why: impl Into<SharedString>, sign_in: Option<&'static str>) {
        self.pending = None;
        self.streaming.clear();
        self.revealed = 0;
        self.error = Some(why.into());
        self.sign_in = sign_in;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arriving(text: &str) -> Conversation {
        let mut chat = Conversation::about("passage", 1);
        chat.asked("why?");
        chat.streaming.push_str(text);

        chat
    }

    #[test]
    fn nothing_is_shown_before_anything_is_revealed() {
        assert_eq!(arriving("hello").visible(), "");
    }

    #[test]
    fn revealing_walks_forward_and_then_stops() {
        let mut chat = arriving("hello");

        while chat.reveal_more() {}
        assert_eq!(chat.visible(), "hello");
        assert!(!chat.reveal_more(), "it kept going after the end");
    }

    /// A burst drains faster than a trickle: the step is a fraction of what is
    /// waiting, so an answer that arrives all at once does not crawl onto the
    /// screen two characters at a time.
    #[test]
    fn a_burst_is_drained_faster_than_a_trickle() {
        let mut burst = arriving(&"x".repeat(600));
        burst.reveal_more();

        let mut trickle = arriving("xxxx");
        trickle.reveal_more();

        assert!(burst.revealed > trickle.revealed * 10, "{}", burst.revealed);
    }

    /// Counting bytes here would cut a character in half, which is a panic
    /// rather than a rendering fault.
    #[test]
    fn revealing_counts_characters_rather_than_bytes() {
        let mut chat = arriving("あいうえお");
        chat.reveal_more();

        assert_eq!(chat.visible().chars().count(), chat.revealed);
        assert!(chat.visible().chars().count() < 5, "it revealed everything");
    }

    #[test]
    fn an_answer_that_is_finished_is_shown_whole() {
        let mut chat = arriving("hello");
        chat.reveal_everything();

        assert_eq!(chat.visible(), "hello");
    }
}
