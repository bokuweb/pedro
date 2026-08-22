//! Indexes whatever in the library is not indexed yet, and says what it did.
//!
//! ```bash
//! cargo run -p pedro-core --example reindex
//! ```
//!
//! The app does this at startup; this is the same call with a report, for
//! seeing how long a library takes and whether the model was found.

use pedro_core::store::Store;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "pedro=info".into()))
        .init();

    let mut store = Store::open_default()?;
    let done = store.index_missing()?;

    println!(
        "indexed {done} book(s); searching by {}",
        match store.can_search_by_meaning() {
            true => "words and meaning",
            false => "words alone",
        }
    );

    Ok(())
}
