//! Pages, text and outlines, read out of a PDF with pdfium.
//!
//! This is the native replacement for what pdf.js does in chatbook's browser:
//! rasterise a page, hand back the box around every character so a drag can be
//! turned into a selection, and read the outline that decides how much of the
//! book travels with a question.
//!
//! pdfium is a shared library rather than a crate, and it is bound at runtime
//! (see [`library_path`]), so a machine without it still builds pedro and only
//! fails when a document is opened.
//!
//! ```no_run
//! let document = pedro_pdf::Document::open("book.pdf".as_ref())?;
//! let page = document.render_page(0, 2.0)?;
//! println!("{}x{}", page.width, page.height);
//! # Ok::<(), pedro_pdf::PdfError>(())
//! ```

mod document;
pub mod fixtures;
mod library;
mod outline;
mod text;

pub use document::{Document, PageImage, PageSize, PixelFormat};
pub use library::library_path;
pub use outline::OutlineItem;
pub use text::{CharBox, PageText, Rect};

/// Everything that can go wrong between a file on disk and a page on screen.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    /// pdfium itself could not be loaded. Carries every path that was tried,
    /// because "install pdfium" is only actionable if you know where it was
    /// looked for.
    #[error("pdfium could not be loaded ({0})")]
    LibraryUnavailable(String),

    #[error("failed to read the document: {0}")]
    Io(#[from] std::io::Error),

    #[error("pdfium refused the document: {0}")]
    Pdfium(#[from] pdfium_render::prelude::PdfiumError),

    #[error("page {requested} does not exist in a {page_count}-page document")]
    NoSuchPage { requested: u32, page_count: u32 },

    /// pdfium is asked for BGRA and nothing else, so this means the library
    /// behaved differently than the version this was written against.
    #[error("pdfium returned an unsupported bitmap format")]
    UnsupportedBitmapFormat,
}
