//! Finding things in the books the reader has added.
//!
//! Two ways of finding, in one SQLite database beside the library itself: the
//! words a reader remembers ([`tokenize`], FTS5) and the meaning of what they
//! ask ([`embed`], vectors). Both index the same [`chunk`]s, so a hit from
//! either names the same passage on the same page.
//!
//! The mechanism is adapted, with permission, from the author's own
//! ellisii-toolkit — its `store-sqlite`, `jp-tokenizer-bigram` and
//! `embed-static-jp` crates — rather than depended on, so that pedro stays
//! under one licence.

pub mod chunk;
pub mod index;
pub mod tokenize;

pub use chunk::Chunk;
pub use index::{Hit, IndexError};
