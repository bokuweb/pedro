//! The reader's domain, with no window attached.
//!
//! Everything chatbook does on its server happens here instead: what a book is
//! and where its bytes live, how much of it travels with a question, what the
//! agent is told, and how the sources it names become pages to jump to.
//!
//! Nothing in this crate depends on GPUI, which is what lets the ported logic —
//! where chatbook's real thinking is — be covered by tests that run without a
//! window.

pub mod chat;
pub mod citation;
pub mod excerpt;
pub mod model;
pub mod prompt;
pub mod store;

pub use chat::{Asked, ChatError, Question, ask, prepare, record};
pub use citation::{BookText, Citation, CitationKind, PageLocation, PageMiss};
pub use excerpt::{Excerpt, PAGE_DELIMITER, select_excerpt};
pub use model::{Book, ChatMessage, Highlight, NewHighlight, ReadingState};
pub use prompt::{Role, Turn, build_conversation, build_system_prompt};
pub use store::{Store, StoreError};
