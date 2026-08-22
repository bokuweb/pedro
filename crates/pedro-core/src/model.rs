//! What the reader accumulates: books, the passages marked in them, and the
//! conversations those passages started.

use pedro_pdf::{OutlineItem, Rect};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::citation::Citation;

pub use pedro_agent::Role;

/// A book in the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Book {
    pub id: String,
    /// What to call it. Follows the file it was last added from, so re-adding
    /// a book under a better name renames it.
    pub file_name: String,
    /// SHA-256 of the file's bytes, and the name of the stored copy.
    ///
    /// Identity is the content, not the path: adding the same book twice is
    /// the same book, so its highlights and its place survive re-adding it.
    pub file_hash: String,
    pub page_count: u32,
    /// Top-level chapters, empty when the book ships no outline.
    pub outline: Vec<OutlineItem>,
    /// The shelf it is on, or `None` for a book that is not on one.
    pub folder_id: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    /// Where the reader left off, absent on a book nobody has opened.
    pub reading: Option<ReadingState>,
}

/// Where the reader left off, and how the panels sat around it.
///
/// The panels are `Option` because "nobody has said either way" is different
/// from "closed": a book opened once with the chat panel never touched should
/// open the way the reader's last book did, not folded shut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingState {
    pub page: u32,
    /// The conversation that was open, so it comes back with the page.
    pub highlight_id: Option<String>,
    pub outline_open: Option<bool>,
    pub chat_panel_open: Option<bool>,
}

/// A passage the reader marked.
#[derive(Debug, Clone, PartialEq)]
pub struct Highlight {
    pub id: String,
    pub book_id: String,
    pub selected_text: String,
    pub page_number: u32,
    /// One rectangle per line of the passage, as fractions of the page.
    pub rects: Vec<Rect>,
    pub color: String,
    pub created_at: OffsetDateTime,
}

/// A passage on its way to being stored.
#[derive(Debug, Clone, PartialEq)]
pub struct NewHighlight {
    pub selected_text: String,
    pub page_number: u32,
    pub rects: Vec<Rect>,
}

/// The colour a highlight gets when nothing else is asked for. chatbook's.
pub const DEFAULT_HIGHLIGHT_COLOR: &str = "#FFEB3B";

/// One turn of the conversation about a highlight, as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub id: String,
    /// What the conversation is about.
    pub about: Conversation,
    pub role: Role,
    /// The answer as the agent wrote it, `## Sources` section and all. The
    /// section is dropped when it is shown and when it is sent back as
    /// history, but it is kept here: it is the record of what was said.
    pub content: String,
    pub citations: Vec<Citation>,
    pub created_at: OffsetDateTime,
}

/// A shelf: books gathered so they can be asked about together.
///
/// Flat on purpose. A shelf is the unit a question is put to, and a question
/// put to a tree would have to say how deep it goes — which is a thing to
/// explain, and a thing to get wrong, in return for an arrangement most
/// libraries this size never need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub created_at: OffsetDateTime,
    /// How many books are on it, which is what the sidebar shows.
    pub book_count: u32,
}

/// What a conversation is about.
///
/// A question about a marked passage and a question about a shelf are the same
/// conversation to everything downstream — same turns, same streaming, same
/// citations — and differ only in what context is gathered for them and where
/// the reader finds them again. This is that difference, named once.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Conversation {
    /// A passage the reader marked. The conversation lives on the highlight,
    /// which is where they will look for it.
    Highlight(String),
    /// A shelf, asked as a whole.
    Folder(String),
}

impl Conversation {
    /// The two columns a message row carries, exactly one of them filled.
    pub fn columns(&self) -> (Option<&str>, Option<&str>) {
        match self {
            Conversation::Highlight(id) => (Some(id), None),
            Conversation::Folder(id) => (None, Some(id)),
        }
    }
}

/// How pages are laid out in the reader, remembered per book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageLayout {
    Single,
    Spread,
}
