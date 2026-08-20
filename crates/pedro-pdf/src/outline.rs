//! The book's own table of contents.
//!
//! Two things read this: the contents panel, and — more importantly — the code
//! that decides how much of the book travels with a question. chatbook sends
//! the chapter a highlight sits in, and the chapter boundaries are exactly the
//! page numbers of these entries.

/// One top-level entry of the outline.
///
/// Only the top level is kept, matching what chatbook stores. Sub-sections
/// would make the intervals between entries smaller, and the interval is the
/// unit of context a question is given: a chapter's worth of pages is the
/// point, and a subsection's worth is usually too little to answer from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutlineItem {
    pub title: String,
    /// One-based, so that it can be compared with a page number the reader sees.
    pub page_number: u32,
}
