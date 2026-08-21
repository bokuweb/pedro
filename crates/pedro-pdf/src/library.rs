//! Finding and binding the pdfium shared library.
//!
//! pdfium is not distributed as a crate: it is a shared library that has to be
//! present at runtime. Binding to it dynamically rather than linking it at
//! build time keeps `cargo build` working on a machine that does not have it —
//! the failure moves to the moment a document is opened, where it can be shown
//! to the reader with somewhere to go next.
//!
//! The search order is deliberately "what the developer said" first and "what
//! the system happens to have" last:
//!
//! 1. `PEDRO_PDFIUM_PATH`, either the library itself or the directory holding it
//! 2. next to the executable, and `../Frameworks` beside it (a macOS bundle)
//! 3. `vendor/pdfium/lib` in any ancestor of the working directory or of the
//!    executable, which is where `scripts/fetch-pdfium.sh` puts it — searching
//!    ancestors rather than one fixed path is what lets `cargo test`, which
//!    runs from the crate directory, find the copy at the workspace root
//! 4. the system library

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use pdfium_render::prelude::Pdfium;

use crate::PdfError;

/// The bound library and where it came from, or every path that was tried.
type Binding = Result<(&'static Pdfium, Option<PathBuf>), String>;

/// Bound once per process.
///
/// pdfium keeps process-global state and `Pdfium::bind_to_library` refuses a
/// second call, so this is a singleton whether we want one or not. It is
/// leaked rather than dropped so that documents can borrow it for `'static`,
/// which is what lets [`crate::Document`] be an ordinary owned value instead of
/// a self-referential one.
static LIBRARY: OnceLock<Binding> = OnceLock::new();

/// Held for the duration of every call into pdfium.
///
/// pdfium aborts the process when two threads are inside it at once — the
/// `thread_safe` feature guards its bindings, not the library's own state — so
/// the guarantee has to be made here rather than asked of every caller. The
/// cost is one uncontended lock per call, against a page render measured in
/// milliseconds.
static IN_USE: Mutex<()> = Mutex::new(());

/// Serialises access to pdfium for as long as the guard lives.
///
/// A panic inside pdfium leaves nothing of ours half-written — the state that
/// matters is pdfium's own — so a poisoned lock is taken rather than turning
/// one failed page into a permanently unusable reader.
pub(crate) fn in_use() -> MutexGuard<'static, ()> {
    IN_USE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn library() -> Result<&'static Pdfium, PdfError> {
    match LIBRARY.get_or_init(bind) {
        Ok((pdfium, _)) => Ok(pdfium),
        Err(attempts) => Err(PdfError::LibraryUnavailable(attempts.clone())),
    }
}

/// Where pdfium was loaded from, for the settings screen and bug reports.
///
/// `None` before the first document is opened, when loading failed, and when
/// the system library answered — that one has no path of ours to report.
pub fn library_path() -> Option<PathBuf> {
    LIBRARY.get()?.as_ref().ok()?.1.clone()
}

fn bind() -> Binding {
    let candidates = candidates();
    let mut attempts = Vec::new();

    // Only existing files are opened, so that the error a reader eventually
    // sees lists the places that were searched rather than a dozen identical
    // "no such file" messages.
    for path in candidates.iter().filter(|path| path.is_file()) {
        match Pdfium::bind_to_library(path) {
            Ok(bindings) => {
                tracing::info!(?path, "bound to pdfium");
                return Ok((
                    Box::leak(Box::new(Pdfium::new(bindings))),
                    Some(path.clone()),
                ));
            }
            Err(err) => attempts.push(format!("{}: {err}", path.display())),
        }
    }

    match Pdfium::bind_to_system_library() {
        Ok(bindings) => {
            tracing::info!("bound to the system pdfium");
            Ok((Box::leak(Box::new(Pdfium::new(bindings))), None))
        }
        Err(err) => {
            if attempts.is_empty() {
                attempts.push(format!("searched {}", display(&candidates)));
            }
            attempts.push(format!("system library: {err}"));
            Err(attempts.join("; "))
        }
    }
}

fn display(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Every path worth trying, in order, without repeats.
fn candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(configured) = std::env::var_os("PEDRO_PDFIUM_PATH") {
        let configured = PathBuf::from(configured);
        // A directory is the friendlier thing to point at, but pointing at the
        // library itself has to keep working: it is what an error message
        // naming a file invites you to do.
        if configured.is_dir() {
            paths.push(in_directory(&configured));
        } else {
            paths.push(configured);
        }
    }

    let executable_directory = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(Path::to_path_buf));

    if let Some(directory) = &executable_directory {
        paths.push(in_directory(directory));
        paths.push(in_directory(&directory.join("../Frameworks")));
    }

    let roots = std::env::current_dir()
        .into_iter()
        .chain(executable_directory);
    for root in roots {
        paths.extend(
            root.ancestors()
                .map(|ancestor| in_directory(&ancestor.join("vendor/pdfium/lib"))),
        );
    }

    paths.dedup();
    paths
}

fn in_directory(directory: &Path) -> PathBuf {
    Pdfium::pdfium_platform_library_name_at_path(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vendor_directory_is_always_a_candidate() {
        let vendored = Path::new("vendor/pdfium/lib").join(Pdfium::pdfium_platform_library_name());
        assert!(candidates().iter().any(|path| path.ends_with(&vendored)));
    }

    /// Only the configured path may name something other than the platform
    /// library, since it is allowed to point straight at a file.
    #[test]
    fn the_searched_directories_name_the_platform_library() {
        let name = Pdfium::pdfium_platform_library_name();
        let configured = std::env::var_os("PEDRO_PDFIUM_PATH").map(PathBuf::from);

        for path in candidates() {
            if configured.as_deref() == Some(path.as_path()) {
                continue;
            }
            assert_eq!(path.file_name(), Some(name.as_os_str()), "{path:?}");
        }
    }
}
