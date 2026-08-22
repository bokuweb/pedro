//! Reading a PDF out of Google Drive.
//!
//! pedro is otherwise a program with nothing on the other end of it: the agent
//! is a subprocess, the library is a file, the embedding model is a table in
//! memory. This is the one crate that talks to a remote service, and it is
//! kept apart from the rest for exactly that reason — nothing in
//! `pedro-core` gains a network dependency because a book can now come from
//! somewhere other than the disk.
//!
//! What it does is narrow on purpose. Given a link the reader pasted, it puts
//! a PDF in a directory and says what it is called. Adding that file to the
//! library is [`pedro_core::store::Store::add_document`]'s job, unchanged: a
//! book from Drive is a book, and it is content-addressed like every other, so
//! fetching one twice does not make two of them.

use std::path::{Path, PathBuf};

mod api;
pub mod auth;
mod link;
mod loopback;

pub use auth::{Credentials, forget, is_signed_in};
pub use link::file_id;

#[derive(Debug, thiserror::Error)]
pub enum DriveError {
    #[error("Google Drive is not set up. Set PEDRO_GOOGLE_CLIENT_ID, and see docs/GOOGLE_DRIVE.md")]
    NotConfigured,

    #[error("that does not look like a Google Drive link")]
    NotALink,

    #[error("no file in Drive with id {0}, or this account cannot see it")]
    NoSuchFile(String),

    #[error("{name} is a {mime_type}, and pedro reads PDFs")]
    NotAPdf { name: String, mime_type: String },

    #[error("the browser could not be opened to sign in: {0}")]
    NoBrowser(String),

    #[error("signing in was not finished")]
    SignInTimedOut,

    #[error("signing in did not go through: {0}")]
    SignInRefused(String),

    /// Not shown to anyone: the sign-in is renewed instead.
    #[error("the stored Google sign-in has expired")]
    SignInExpired,

    #[error("the keychain could not be used: {0}")]
    Keychain(String),

    #[error("Google Drive could not be reached: {0}")]
    Network(String),

    #[error("Google Drive said: {0}")]
    Google(String),

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl DriveError {
    fn network(err: ureq::Error) -> Self {
        Self::Network(err.to_string())
    }
}

/// A PDF, fetched and written down.
pub struct Fetched {
    /// Where it was written, ready to be added to the library.
    pub path: PathBuf,
    /// What Drive calls it, which becomes the title in the library.
    pub name: String,
}

/// A directory of its own for one fetch.
///
/// A fetched file is a copy on the way to the library, which keeps its own
/// under a content hash. Somewhere temporary, and a fresh name every time, so
/// two fetches of files that happen to share a name cannot land on each other.
pub fn scratch() -> PathBuf {
    std::env::temp_dir()
        .join("pedro-drive")
        .join(uuid::Uuid::new_v4().simple().to_string())
}

/// Fetches whatever `link` names into `directory`.
///
/// Blocking from end to end, and the first time it runs it opens a browser and
/// waits for the reader to come back — so this belongs on a background thread,
/// never on the one drawing frames.
pub fn fetch(
    credentials: &Credentials,
    link: &str,
    directory: &Path,
) -> Result<Fetched, DriveError> {
    let file_id = link::file_id(link).ok_or(DriveError::NotALink)?;
    let access_token = auth::access_token(credentials)?;

    let metadata = api::metadata(&access_token, &file_id)?;
    tracing::info!(file_id, name = metadata.name, "fetching from Drive");

    let bytes = api::download(&access_token, &file_id, &metadata)?;

    std::fs::create_dir_all(directory)?;
    let path = directory.join(file_name(&metadata.name));
    std::fs::write(&path, &bytes)?;

    tracing::info!(?path, bytes = bytes.len(), "fetched from Drive");
    Ok(Fetched {
        path,
        name: metadata.name,
    })
}

/// A Drive name as something safe to write down.
///
/// Drive names are free text and can hold anything a path cannot — a slash, a
/// leading dot, nothing at all — and this name ends up as the book's title, so
/// it is cleaned rather than replaced.
fn file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    // Leading dots and separators are what turn a name into a path; nothing
    // is lost by dropping them, because a name that is only those is not one.
    let cleaned = cleaned.trim().trim_start_matches(['.', '-', ' ']).trim();

    let stem = match cleaned.is_empty() {
        true => "document",
        false => cleaned,
    };

    // A Google Doc exported as a PDF keeps its document name, with no
    // extension on it at all; the library reads the title off the file name,
    // and a book called "Notes.pdf" is what a reader expects to see.
    match stem.to_ascii_lowercase().ends_with(".pdf") {
        true => stem.to_owned(),
        false => format!("{stem}.pdf"),
    }
}

/// The one HTTP client, configured once.
mod http {
    use std::time::Duration;

    /// Long enough for a large book on a slow connection.
    const PATIENCE: Duration = Duration::from_secs(120);

    pub(crate) fn agent() -> ureq::Agent {
        ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                // Google puts the reason a call failed in the body, and a
                // client that turns the status into an error throws it away.
                .http_status_as_error(false)
                .timeout_global(Some(PATIENCE))
                .build(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_is_already_a_pdf_keeps_its_extension_once() {
        assert_eq!(file_name("sicp.pdf"), "sicp.pdf");
        assert_eq!(file_name("sicp.PDF"), "sicp.PDF");
    }

    #[test]
    fn an_exported_document_is_given_one() {
        assert_eq!(file_name("Meeting notes"), "Meeting notes.pdf");
    }

    #[test]
    fn a_name_cannot_reach_out_of_the_directory() {
        assert_eq!(file_name("../../etc/passwd"), "etc-passwd.pdf");
        assert_eq!(file_name("/absolute"), "absolute.pdf");
    }

    #[test]
    fn a_name_that_is_nothing_usable_still_names_a_file() {
        assert_eq!(file_name("   "), "document.pdf");
        assert_eq!(file_name("..."), "document.pdf");
    }
}
