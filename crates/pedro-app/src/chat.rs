//! One conversation about one passage.
//!
//! chatbook's unit of conversation is the highlight, not the book: a question
//! is about a passage, and the answers to it live with that passage. This holds
//! the one that is open, including the answer arriving a token at a time.

use std::time::{Duration, Instant};

use gpui::SharedString;
use pedro_agent::Cancellation;
use pedro_core::model::ChatMessage;

// Every constant below is per second rather than per frame, because the frame
// is not a fixed length: the writing is driven by the display, and the same
// answer has to be written at the same speed on a 60Hz screen and a 120Hz one.
// Per-frame constants would make it twice as fast on the better screen.

/// The slowest the answer is ever written, in characters per second. The pace
/// of the trickle between bursts.
const SLOWEST: f32 = 90.;

/// The fastest. Well above what any CLI actually produces, so the writing can
/// always catch up in the end; it is the easing below, not this ceiling, that
/// keeps a burst from landing as a block.
const FASTEST: f32 = 1200.;

/// How long the writing aims to take to drain what is waiting, in seconds.
///
/// This is also how far behind the agent the answer settles: while a stream is
/// running steadily, the writing is about this much text behind it.
const CATCH_UP: f32 = 0.6;

/// How quickly the rate moves towards that aim, as a time constant in seconds.
///
/// Easing the *rate* rather than the step is what stops a burst landing as a
/// block: stepping by a fraction of what is waiting puts the biggest jump on
/// the frame the chunk arrived, which is the chunk, redrawn.
const EASE: f32 = 0.25;

/// A frame is never treated as longer than this. A window that was occluded or
/// a machine that stalled should not dump a second of text in one step.
const LONGEST_FRAME: Duration = Duration::from_millis(100);

/// What the first frame of an answer is assumed to have taken.
const A_FRAME: Duration = Duration::from_millis(8);

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
    /// How fast it is being written, in characters per second; the fraction of
    /// a character carried over from the last frame; and when that frame was,
    /// which is what turns the rate into a number of characters.
    rate: f32,
    carry: f32,
    last_frame: Option<Instant>,
    /// The stored turns, waiting for the answer to finish being written.
    ///
    /// The agent finishing and the answer finishing are different moments.
    /// Swapping the stored turns in at the first snaps whatever has not been
    /// written yet onto the screen, which is where the writing most visibly
    /// stops being writing.
    settled: Option<Vec<ChatMessage>>,
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
            rate: SLOWEST,
            carry: 0.,
            last_frame: None,
            settled: None,
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
        self.rewind();
        self.error = None;
        self.sign_in = None;
        self.cancellation = Cancellation::new();
    }

    /// The agent has finished answering. Whether that is visible yet depends
    /// on whether the writing has caught up with it.
    pub fn answered(&mut self, messages: Vec<ChatMessage>) {
        self.settled = Some(messages);
        self.settle();
    }

    /// Swaps the stored turns in, if there is nothing left to write.
    ///
    /// Returns whether it did, so the caller knows to redraw.
    pub fn settle(&mut self) -> bool {
        if self.revealed < self.streaming.chars().count() {
            return false;
        }
        let Some(messages) = self.settled.take() else {
            return false;
        };

        self.messages = messages;
        self.pending = None;
        self.rewind();
        true
    }

    /// Back to an empty answer at the resting pace.
    fn rewind(&mut self) {
        self.streaming.clear();
        self.revealed = 0;
        self.rate = SLOWEST;
        self.carry = 0.;
        self.last_frame = None;
        self.settled = None;
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
    /// The rate is what accelerates, not the step: a backlog raises what the
    /// writing is aiming for, and it eases towards that aim over several
    /// frames. Stepping straight to the aim would put the shape of the
    /// agent's chunks back on the screen, which is the thing being smoothed
    /// out.
    ///
    /// Returns whether anything is still hidden.
    pub fn reveal_more(&mut self) -> bool {
        let now = Instant::now();
        let since = self
            .last_frame
            .replace(now)
            .map_or(A_FRAME, |last| now.saturating_duration_since(last))
            .min(LONGEST_FRAME);

        self.reveal_more_over(since)
    }

    /// The same, for a frame of a stated length, which is what the tests can
    /// hold still.
    fn reveal_more_over(&mut self, frame: Duration) -> bool {
        let arrived = self.streaming.chars().count();
        let waiting = arrived.saturating_sub(self.revealed);
        if waiting == 0 {
            // Come back at the resting pace rather than at whatever speed the
            // last burst worked it up to.
            self.rate = SLOWEST;
            self.carry = 0.;
            return false;
        }

        let seconds = frame.as_secs_f32();
        let aim = (waiting as f32 / CATCH_UP).clamp(SLOWEST, FASTEST);
        self.rate += (aim - self.rate) * (seconds / EASE).min(1.);

        self.carry += self.rate * seconds;
        let whole = self.carry.floor();
        self.carry -= whole;

        self.revealed = (self.revealed + whole as usize).min(arrived);
        self.revealed < arrived
    }

    /// Shows all of it at once, for when there is no more coming.
    pub fn reveal_everything(&mut self) {
        self.revealed = self.streaming.chars().count();
    }

    pub fn failed(&mut self, why: impl Into<SharedString>, sign_in: Option<&'static str>) {
        self.pending = None;
        self.rewind();
        self.error = Some(why.into());
        self.sign_in = sign_in;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of a 120Hz display.
    const FRAME: Duration = Duration::from_millis(8);

    /// Runs `frames` frames of writing.
    fn write(chat: &mut Conversation, frames: usize) {
        for _ in 0..frames {
            chat.reveal_more_over(FRAME);
        }
    }

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

        while chat.reveal_more_over(FRAME) {}
        assert_eq!(chat.visible(), "hello");
        assert!(!chat.reveal_more_over(FRAME), "it kept going after the end");
    }

    /// The fault this replaced: a step that was a quarter of what was waiting
    /// put a hundred characters of a four-hundred character burst on screen in
    /// a single frame, which is the burst itself, drawn.
    #[test]
    fn a_burst_does_not_land_in_one_frame() {
        let mut burst = arriving(&"x".repeat(600));
        write(&mut burst, 1);

        assert!(
            burst.revealed <= 2,
            "a chunk arrived rather than being written: {}",
            burst.revealed
        );
    }

    /// It does still catch up, though — by accelerating over a few frames
    /// rather than by jumping.
    #[test]
    fn a_burst_is_written_faster_than_a_trickle() {
        let mut burst = arriving(&"x".repeat(600));
        let mut trickle = arriving(&"x".repeat(40));
        write(&mut burst, 60);
        write(&mut trickle, 60);

        assert!(
            burst.revealed > trickle.revealed * 2,
            "burst {} vs trickle {}",
            burst.revealed,
            trickle.revealed
        );
    }

    /// The rate ramps rather than steps: no single frame of a burst reveals a
    /// large share of it.
    #[test]
    fn the_rate_ramps_up_rather_than_jumping() {
        let mut chat = arriving(&"x".repeat(600));

        let mut last = 0;
        for _ in 0..80 {
            chat.reveal_more_over(FRAME);
            let step = chat.revealed - last;
            assert!(step <= 12, "one frame revealed {step} characters");
            last = chat.revealed;
        }
    }

    /// An agent that has finished is not the same as an answer that has
    /// finished arriving on screen. Swapping the stored turns in early is what
    /// used to snap the tail of every answer into place.
    #[test]
    fn a_finished_answer_waits_for_the_writing_to_catch_up() {
        let mut chat = arriving(&"x".repeat(600));
        chat.answered(Vec::new());

        assert!(chat.is_answering(), "it settled before it had been written");
        assert!(!chat.visible().is_empty() || chat.revealed == 0);

        while chat.reveal_more_over(FRAME) {}
        assert!(chat.settle(), "it never settled once it had caught up");
        assert!(!chat.is_answering());
        assert_eq!(chat.visible(), "");
    }

    /// Counting bytes here would cut a character in half, which is a panic
    /// rather than a rendering fault.
    #[test]
    fn revealing_counts_characters_rather_than_bytes() {
        let mut chat = arriving("あいうえお");
        write(&mut chat, 2);

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
