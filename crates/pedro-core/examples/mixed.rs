//! Adds a book whose pages are not all one shape, for looking at the reader
//! with something other than a tidy A4 book in it.
//!
//! ```bash
//! PEDRO_LIBRARY_PATH=/tmp/a-copy cargo run -p pedro-core --example mixed
//! ```

use pedro_core::store::Store;
use pedro_pdf::fixtures::{Page, pdf_with_sizes};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("warn").init();

    if std::env::var_os("PEDRO_LIBRARY_PATH").is_none() {
        eprintln!("point this at a copy: PEDRO_LIBRARY_PATH=/tmp/a-copy");
        std::process::exit(2);
    }

    // A4 upright throughout, with an A3 turned sideways at pages 4 and 9 — the
    // fold-out plan in a book of text.
    let pages: Vec<Page<'_>> = (1..=12)
        .map(|number| match number {
            4 | 9 => Page::sized("sideways A3", 1191., 842.),
            _ => Page::sized("upright A4", 595., 842.),
        })
        .collect();

    let mut store = Store::open_default()?;
    let path = store.root().join("mixed-sizes.pdf");
    std::fs::write(&path, pdf_with_sizes(&pages))?;

    let book = store.add_document(&path)?;
    println!("added {} ({} pages)", book.file_name, book.page_count);

    Ok(())
}
