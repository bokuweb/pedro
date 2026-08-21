//! The reader's own books, as the shell holds them.
//!
//! [`pedro_core::store::Store`] is a SQLite connection: `Send`, not `Sync`, and
//! happy to be used from whichever thread holds it. The shell keeps it behind a
//! mutex so that quick reads can happen inline while adding a document — which
//! rasterises nothing but does extract the text of every page — runs in the
//! background without the list going away in the meantime.

use std::sync::{Arc, Mutex, MutexGuard};

use gpui::SharedString;
use pedro_core::model::Book;
use pedro_core::store::Store;
use time::OffsetDateTime;

/// A store shared between the UI thread and the background work it starts.
#[derive(Clone)]
pub struct SharedStore(Arc<Mutex<Store>>);

impl SharedStore {
    pub fn new(store: Store) -> Self {
        Self(Arc::new(Mutex::new(store)))
    }

    /// A panicking reader leaves the database exactly as SQLite left it — the
    /// state that could be half-written is in the file, under a transaction —
    /// so a poisoned lock is taken rather than making the library unusable for
    /// the rest of the session.
    pub fn lock(&self) -> MutexGuard<'_, Store> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// What the shell knows about the library.
pub enum Library {
    /// Being opened on a background thread; the first frame draws before the
    /// database is even on disk.
    Opening,
    Ready {
        store: SharedStore,
        /// Where the books are kept. Held here so that saying so costs no lock:
        /// an answer being written holds the store for as long as the agent
        /// takes to write it.
        root: std::path::PathBuf,
        books: Vec<Book>,
    },
    /// The library could not be opened at all — a read-only home directory, a
    /// corrupt database. Carries what to tell the reader.
    Failed(SharedString),
}

impl Library {
    pub fn books(&self) -> &[Book] {
        match self {
            Library::Ready { books, .. } => books,
            _ => &[],
        }
    }

    pub fn store(&self) -> Option<&SharedStore> {
        match self {
            Library::Ready { store, .. } => Some(store),
            _ => None,
        }
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        match self {
            Library::Ready { root, .. } => Some(root),
            _ => None,
        }
    }

    /// What the sidebar says when it has no rows to show.
    pub fn empty_message(&self) -> SharedString {
        match self {
            Library::Opening => "Opening the library…".into(),
            Library::Failed(why) => format!("The library could not be opened. {why}").into(),
            Library::Ready { .. } => "No documents yet. Add a PDF to get started.".into(),
        }
    }
}

/// The title a book is listed under: its file name without the extension,
/// which is as close to a title as a PDF on disk usually gets.
pub fn title_of(book: &Book) -> SharedString {
    book.file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .filter(|stem| !stem.is_empty())
        .unwrap_or(&book.file_name)
        .to_owned()
        .into()
}

/// How long ago, in the shortest form that is still true.
///
/// Coarse on purpose: the number is there to sort the eye down the list, and
/// "3d" does that as well as a date while taking a quarter of the width.
pub fn how_long_ago(when: OffsetDateTime) -> SharedString {
    let seconds = (OffsetDateTime::now_utc() - when).whole_seconds().max(0);

    match seconds {
        0..60 => "now".into(),
        60..3_600 => format!("{}m", seconds / 60).into(),
        3_600..86_400 => format!("{}h", seconds / 3_600).into(),
        86_400..2_592_000 => format!("{}d", seconds / 86_400).into(),
        _ => format!("{}mo", seconds / 2_592_000).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Duration;

    fn book(file_name: &str) -> Book {
        Book {
            id: "id".to_owned(),
            file_name: file_name.to_owned(),
            file_hash: "hash".to_owned(),
            page_count: 1,
            outline: Vec::new(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            reading: None,
        }
    }

    #[test]
    fn a_title_drops_the_extension() {
        assert_eq!(title_of(&book("sicp.pdf")), "sicp");
    }

    #[test]
    fn a_title_keeps_a_name_that_is_all_extension() {
        assert_eq!(title_of(&book(".pdf")), ".pdf");
    }

    #[test]
    fn a_title_keeps_a_name_with_no_extension_at_all() {
        assert_eq!(title_of(&book("sicp")), "sicp");
    }

    #[test]
    fn a_title_cuts_at_the_last_dot() {
        assert_eq!(
            title_of(&book("tcp.ip.illustrated.pdf")),
            "tcp.ip.illustrated"
        );
    }

    #[test]
    fn the_last_minute_is_now() {
        assert_eq!(how_long_ago(OffsetDateTime::now_utc()), "now");
    }

    #[test]
    fn ages_are_reported_in_the_largest_unit_that_fits() {
        let ago = |duration| how_long_ago(OffsetDateTime::now_utc() - duration);

        assert_eq!(ago(Duration::minutes(5)), "5m");
        assert_eq!(ago(Duration::hours(3)), "3h");
        assert_eq!(ago(Duration::days(2)), "2d");
        assert_eq!(ago(Duration::days(60)), "2mo");
    }

    /// A clock that has gone backwards is not worth a negative age.
    #[test]
    fn a_timestamp_in_the_future_is_now() {
        assert_eq!(
            how_long_ago(OffsetDateTime::now_utc() + Duration::hours(1)),
            "now"
        );
    }
}
